//! Runtime-neutral wallet-side TON Connect session orchestration.
//!
//! The client owns protocol, replay, encryption, and SSE cursor state. The host
//! owns HTTP streaming, durable storage, wallet operations, and user approval.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

use std::num::{NonZeroU32, NonZeroUsize};

use serde::{Deserialize, Deserializer, Serialize, de};
use thiserror::Error;
use ton_connect_core::{
    AppRequest, BridgeCodecError, BridgeSseDecoder, ClientId, ConnectEvent, ConnectEventError,
    ConnectEventErrorCode, ConnectEventPayload, ConnectLink, ConnectLinkError, ConnectRequest,
    ConnectValidationError, EmbeddedRequest, EmbeddedRequestError, EmbeddedResponse, EmptyObject,
    HeartbeatMode, HttpBridgeError, HttpBridgeUrl, HttpSessionError, KnownAppRequest,
    PersistedHttpSession, PreparedBridgePost, RawAccountAddress, ResponseValidationError, RpcError,
    SessionCrypto, SessionCryptoError, SessionStateError, TraceId, WalletEventKind, WalletResponse,
    WalletSessionPhase, WalletSessionState, decode_embedded_request_param,
    encode_embedded_request_param,
};

/// Limits and bridge identity used by one wallet-side HTTP session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TonConnectClientConfig {
    bridge_url: HttpBridgeUrl,
    max_event_bytes: NonZeroUsize,
    message_ttl_seconds: NonZeroU32,
    heartbeat: HeartbeatMode,
}

impl TonConnectClientConfig {
    /// Creates an HTTP bridge configuration from already validated values.
    #[must_use]
    pub const fn new(
        bridge_url: HttpBridgeUrl,
        max_event_bytes: NonZeroUsize,
        message_ttl_seconds: NonZeroU32,
        heartbeat: HeartbeatMode,
    ) -> Self {
        Self {
            bridge_url,
            max_event_bytes,
            message_ttl_seconds,
            heartbeat,
        }
    }

    /// Returns the wallet-owned bridge base URL.
    #[must_use]
    pub const fn bridge_url(&self) -> &HttpBridgeUrl {
        &self.bridge_url
    }
}

/// A fresh dApp request accepted by the replay reducer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncomingRequest {
    request: AppRequest,
    trace_id: Option<TraceId>,
    closes_session: bool,
}

/// Complete session state that a host can store and restore after a restart.
///
/// The serialized value contains the HTTP session secret key. Store it with
/// the same protection as wallet authentication credentials.
#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedTonConnectClient {
    session: PersistedHttpSession,
    connect_request: Option<ConnectRequest>,
    embedded_request: Option<String>,
    initial_trace_id: Option<TraceId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedTonConnectClientWire {
    session: PersistedHttpSession,
    connect_request: Option<ConnectRequest>,
    embedded_request: Option<String>,
    initial_trace_id: Option<TraceId>,
}

impl<'de> Deserialize<'de> for PersistedTonConnectClient {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = PersistedTonConnectClientWire::deserialize(deserializer)?;
        let pending = wire.session.reducer().phase() == WalletSessionPhase::PendingConnect;
        if pending != wire.connect_request.is_some()
            || (wire.embedded_request.is_some() && wire.connect_request.is_none())
        {
            return Err(de::Error::custom(
                "persisted TON Connect client does not match its session phase",
            ));
        }
        if let Some(embedded) = wire.embedded_request.as_deref() {
            let _ = decode_embedded_request_param(embedded).map_err(de::Error::custom)?;
        }
        Ok(Self {
            session: wire.session,
            connect_request: wire.connect_request,
            embedded_request: wire.embedded_request,
            initial_trace_id: wire.initial_trace_id,
        })
    }
}

impl IncomingRequest {
    /// Returns the original bridge request envelope.
    #[must_use]
    pub const fn request(&self) -> &AppRequest {
        &self.request
    }

    /// Parses the method-specific request body.
    pub fn decode(&self) -> Result<KnownAppRequest, RpcError> {
        self.request.clone().decode()
    }

    /// Returns the bridge topic for the response.
    #[must_use]
    pub fn topic(&self) -> &str {
        &self.request.method
    }

    /// Returns the trace identifier that the bridge attached to the request.
    #[must_use]
    pub const fn trace_id(&self) -> Option<&TraceId> {
        self.trace_id.as_ref()
    }

    /// Reports whether this request atomically ended the session.
    #[must_use]
    pub const fn closes_session(&self) -> bool {
        self.closes_session
    }
}

/// Wallet-side TON Connect HTTP session state machine.
pub struct TonConnectClient {
    connect_request: Option<ConnectRequest>,
    embedded_request: Option<EmbeddedRequest>,
    initial_trace_id: Option<TraceId>,
    peer_client_id: ClientId,
    bridge_url: HttpBridgeUrl,
    max_event_bytes: NonZeroUsize,
    message_ttl_seconds: NonZeroU32,
    heartbeat: HeartbeatMode,
    crypto: SessionCrypto,
    reducer: WalletSessionState,
    connected_address: Option<RawAccountAddress>,
    last_bridge_event_id: Option<String>,
    decoder: BridgeSseDecoder,
}

impl TonConnectClient {
    /// Parses a full connect link and creates a fresh encrypted HTTP session.
    pub fn from_link(
        link: &str,
        config: TonConnectClientConfig,
    ) -> Result<Self, TonConnectClientError> {
        let link = ConnectLink::parse(link)?;
        Self::from_parsed_link(&link, config)
    }

    /// Creates a fresh encrypted HTTP session from a parsed full connect link.
    pub fn from_parsed_link(
        link: &ConnectLink,
        config: TonConnectClientConfig,
    ) -> Result<Self, TonConnectClientError> {
        let Some(connect_request) = link.request().cloned() else {
            return Err(TonConnectClientError::MissingConnectRequest);
        };
        let embedded_request = link.embedded_request().cloned();
        let initial_trace_id = link.trace_id().cloned();
        let peer_client_id = link.client_id();
        let crypto = SessionCrypto::generate()?;
        let decoder = BridgeSseDecoder::new(config.max_event_bytes);
        Ok(Self {
            connect_request: Some(connect_request),
            embedded_request,
            initial_trace_id,
            peer_client_id,
            bridge_url: config.bridge_url,
            max_event_bytes: config.max_event_bytes,
            message_ttl_seconds: config.message_ttl_seconds,
            heartbeat: config.heartbeat,
            crypto,
            reducer: WalletSessionState::pending_connect(),
            connected_address: None,
            last_bridge_event_id: None,
            decoder,
        })
    }

    /// Restores a previously persisted session without creating new keys.
    pub fn restore(
        persisted: &PersistedTonConnectClient,
        max_event_bytes: NonZeroUsize,
        message_ttl_seconds: NonZeroU32,
        heartbeat: HeartbeatMode,
    ) -> Result<Self, TonConnectClientError> {
        Ok(Self {
            connect_request: persisted.connect_request.clone(),
            embedded_request: persisted
                .embedded_request
                .as_deref()
                .map(decode_embedded_request_param)
                .transpose()?,
            initial_trace_id: persisted.initial_trace_id.clone(),
            peer_client_id: persisted.session.peer_client_id(),
            bridge_url: persisted.session.bridge_url().clone(),
            max_event_bytes,
            message_ttl_seconds,
            heartbeat,
            crypto: persisted.session.restore_crypto()?,
            reducer: persisted.session.reducer().clone(),
            connected_address: persisted.session.connected_address(),
            last_bridge_event_id: persisted.session.last_bridge_event_id().map(str::to_owned),
            decoder: BridgeSseDecoder::new(max_event_bytes),
        })
    }

    /// Returns the initial connect request while the session is being approved.
    #[must_use]
    pub const fn connect_request(&self) -> Option<&ConnectRequest> {
        self.connect_request.as_ref()
    }

    /// Returns the optional one-tap action from the connect link.
    #[must_use]
    pub const fn embedded_request(&self) -> Option<&EmbeddedRequest> {
        self.embedded_request.as_ref()
    }

    /// Returns the dApp bridge client identifier fixed for this session.
    #[must_use]
    pub const fn peer_client_id(&self) -> ClientId {
        self.peer_client_id
    }

    /// Returns the wallet bridge client identifier fixed for this session.
    #[must_use]
    pub const fn client_id(&self) -> ClientId {
        self.crypto.client_id()
    }

    /// Returns the current protocol lifecycle phase.
    #[must_use]
    pub const fn phase(&self) -> WalletSessionPhase {
        self.reducer.phase()
    }

    /// Returns the account fixed by a successful connect response.
    #[must_use]
    pub const fn connected_address(&self) -> Option<RawAccountAddress> {
        self.connected_address
    }

    /// Returns the most recent bridge SSE resume cursor.
    #[must_use]
    pub fn last_bridge_event_id(&self) -> Option<&str> {
        self.last_bridge_event_id.as_deref()
    }

    /// Builds the next SSE subscription URL and resets stream-local framing state.
    #[must_use]
    pub fn begin_events_subscription(&mut self) -> url::Url {
        self.decoder = BridgeSseDecoder::new(self.max_event_bytes);
        self.bridge_url.events_endpoint_with_heartbeat(
            self.crypto.client_id(),
            self.last_bridge_event_id.as_deref(),
            self.initial_trace_id.as_ref(),
            self.heartbeat,
        )
    }

    /// Validates and encrypts a successful connect event.
    ///
    /// Persist [`Self::persisted`] before the host posts the returned message.
    pub fn approve_connect(
        &mut self,
        payload: ConnectEventPayload,
        embedded_response: Option<EmbeddedResponse>,
    ) -> Result<PreparedBridgePost, TonConnectClientError> {
        let Some(request) = self.connect_request.as_ref() else {
            return Err(TonConnectClientError::MissingConnectRequest);
        };
        let transition = self.reducer.prepare_event(WalletEventKind::Connect)?;
        let event = ConnectEvent::Connect {
            id: transition.id(),
            payload,
            response: embedded_response,
        };
        event.validate_for_connect(request, self.embedded_request.as_ref())?;
        let Some(address) = connected_address(&event) else {
            return Err(TonConnectClientError::MissingConnectedAddress);
        };
        let post = self.prepare_post(&event, None, self.initial_trace_id.as_ref())?;
        self.reducer = transition.into_state();
        self.connected_address = Some(address);
        self.connect_request = None;
        self.embedded_request = None;
        self.initial_trace_id = None;
        Ok(post)
    }

    /// Encrypts a terminal connect error and closes the pending session.
    ///
    /// Persist [`Self::persisted`] before the host posts the returned message.
    pub fn reject_connect(
        &mut self,
        code: ConnectEventErrorCode,
        message: String,
    ) -> Result<PreparedBridgePost, TonConnectClientError> {
        let transition = self.reducer.prepare_event(WalletEventKind::ConnectError)?;
        let event = ConnectEvent::ConnectError {
            id: transition.id(),
            payload: ConnectEventError { code, message },
        };
        let post = self.prepare_post(&event, None, self.initial_trace_id.as_ref())?;
        self.reducer = transition.into_state();
        self.connect_request = None;
        self.embedded_request = None;
        self.initial_trace_id = None;
        Ok(post)
    }

    /// Encrypts a wallet-initiated disconnect event and closes the session.
    ///
    /// Post the returned message before deleting the persisted session secret.
    pub fn disconnect(
        &mut self,
        trace_id: Option<&TraceId>,
    ) -> Result<PreparedBridgePost, TonConnectClientError> {
        let transition = self.reducer.prepare_event(WalletEventKind::Disconnect)?;
        let event = ConnectEvent::Disconnect {
            id: transition.id(),
            payload: EmptyObject,
        };
        let post = self.prepare_post(&event, None, trace_id)?;
        self.reducer = transition.into_state();
        Ok(post)
    }

    /// Decodes an SSE chunk and returns only fresh authenticated dApp requests.
    ///
    /// Messages from another peer, malformed ciphertext, and replayed request
    /// identifiers are discarded before they reach wallet approval code.
    /// Persist [`Self::persisted`] before processing any returned request.
    pub fn ingest_sse_chunk(
        &mut self,
        chunk: &[u8],
    ) -> Result<Vec<IncomingRequest>, TonConnectClientError> {
        let events = self.decoder.push(chunk)?;
        let mut requests = Vec::new();
        for event in events {
            if let Some(event_id) = event.event_id() {
                self.last_bridge_event_id = Some(event_id.to_owned());
            }
            let trace_id = event.message().trace_id().cloned();
            let Ok(request) = event.decrypt::<AppRequest>(&self.crypto, self.peer_client_id) else {
                continue;
            };
            let Ok(prepared) = self.reducer.prepare_request(&request) else {
                continue;
            };
            let closes_session = prepared.closes_session();
            self.reducer = prepared.into_state();
            requests.push(IncomingRequest {
                request,
                trace_id,
                closes_session,
            });
        }
        Ok(requests)
    }

    /// Validates correlation when possible and encrypts a wallet RPC response.
    pub fn prepare_response(
        &self,
        incoming: &IncomingRequest,
        response: &WalletResponse,
    ) -> Result<PreparedBridgePost, TonConnectClientError> {
        if response_id(response) != incoming.request.id {
            return Err(TonConnectClientError::ResponseIdMismatch);
        }
        if let Ok(known) = incoming.decode() {
            let _ = response.validate_for(&known)?;
        }
        self.prepare_post(response, Some(incoming.topic()), incoming.trace_id())
    }

    /// Captures the complete secret-bearing session state for protected storage.
    pub fn persisted(&self) -> Result<PersistedTonConnectClient, TonConnectClientError> {
        Ok(PersistedTonConnectClient {
            session: PersistedHttpSession::new(
                &self.crypto,
                self.peer_client_id,
                self.bridge_url.clone(),
                self.reducer.clone(),
                self.connected_address,
                self.last_bridge_event_id.clone(),
            )?,
            connect_request: self.connect_request.clone(),
            embedded_request: self
                .embedded_request
                .as_ref()
                .map(encode_embedded_request_param)
                .transpose()?,
            initial_trace_id: self.initial_trace_id.clone(),
        })
    }

    fn prepare_post<T: Serialize>(
        &self,
        payload: &T,
        topic: Option<&str>,
        trace_id: Option<&TraceId>,
    ) -> Result<PreparedBridgePost, TonConnectClientError> {
        Ok(self.bridge_url.prepare_post(
            &self.crypto,
            self.peer_client_id,
            self.message_ttl_seconds,
            topic,
            trace_id,
            payload,
        )?)
    }
}

fn connected_address(event: &ConnectEvent) -> Option<RawAccountAddress> {
    let ConnectEvent::Connect { payload, .. } = event else {
        return None;
    };
    payload.items.iter().find_map(|reply| match reply {
        ton_connect_core::ConnectItemReply::TonAddress(account) => Some(account.address),
        ton_connect_core::ConnectItemReply::TonProof(_)
        | ton_connect_core::ConnectItemReply::Error(_) => None,
    })
}

fn response_id(response: &WalletResponse) -> &str {
    match response {
        WalletResponse::Success(response) => &response.id,
        WalletResponse::Error { id, .. } => id,
    }
}

/// Failure in wallet-side TON Connect session orchestration.
#[derive(Debug, Error)]
pub enum TonConnectClientError {
    /// A reduced link cannot start a new wallet session.
    #[error("TON Connect link does not contain a connect request")]
    MissingConnectRequest,
    /// A validated connect response unexpectedly contained no account.
    #[error("TON Connect connect response has no connected account")]
    MissingConnectedAddress,
    /// A wallet response does not echo the accepted dApp request identifier.
    #[error("TON Connect wallet response id does not match the dApp request")]
    ResponseIdMismatch,
    /// The deep link is malformed.
    #[error(transparent)]
    Link(#[from] ConnectLinkError),
    /// Session key generation or encryption failed.
    #[error(transparent)]
    Crypto(#[from] SessionCryptoError),
    /// The connect event does not match the initiating request.
    #[error(transparent)]
    ConnectValidation(#[from] ConnectValidationError),
    /// The protocol reducer rejected a lifecycle transition.
    #[error(transparent)]
    SessionState(#[from] SessionStateError),
    /// HTTP bridge framing or endpoint construction failed.
    #[error(transparent)]
    HttpBridge(#[from] HttpBridgeError),
    /// Bridge plaintext encryption or encoding failed.
    #[error(transparent)]
    BridgeCodec(#[from] BridgeCodecError),
    /// A known response does not match its request method.
    #[error(transparent)]
    ResponseValidation(#[from] ResponseValidationError),
    /// Persisted session material is malformed.
    #[error(transparent)]
    HttpSession(#[from] HttpSessionError),
    /// An embedded request cannot be encoded or restored.
    #[error(transparent)]
    EmbeddedRequest(#[from] EmbeddedRequestError),
}
