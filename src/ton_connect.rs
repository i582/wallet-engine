//! FFI-safe wallet-side TON Connect session orchestration.
//!
//! Protocol parsing, authenticated encryption, replay protection, and durable
//! response state remain in Rust. Platform clients own HTTP streaming,
//! protected persistence, manifest loading, and approval presentation.

use std::collections::BTreeMap;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use ton_connect_client::{
    IncomingRequest, PersistedTonConnectClient, TonConnectClient, TonConnectClientConfig,
};
use ton_connect_core::{
    AppManifest, ConnectEventErrorCode, ConnectEventPayload, ConnectItem, ConnectItemReply,
    DeviceInfo, DevicePlatform, Ed25519PublicKey, Ed25519Signature, EmbeddedResponse,
    EmbeddedResponseError, Feature, HeartbeatMode, HttpBridgeUrl, KnownAppRequest, NetworkId,
    RawAccountAddress, RpcErrorCode, SendTransactionFeature, TonAddressItemReply, TonProof,
    TonProofDomain, TonProofItemReply, TransactionPayload, Uint64String, WalletResponse,
    WalletResponseError, WalletResponseSuccess, WalletResult, WalletSessionPhase, WalletStateInit,
};

use crate::{
    Boc, NonEmptyString, SendAmount, SendExpiration, SendIntent, SendMessage, SendMessageBody,
    SendRequest, TonConnectAccountInfo, WalletClientError, bounded_diagnostic,
};

/// Limits and bridge identity for one wallet-side TON Connect session.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TonConnectSessionConfig {
    /// Wallet-owned HTTP bridge base URL.
    pub bridge_url: String,
    /// Maximum bytes accepted for one SSE event.
    pub max_event_bytes: u64,
    /// Bridge message lifetime in seconds.
    pub message_ttl_seconds: u32,
}

/// Runtime platform advertised as the TypeScript protocol's `DeviceInfo.platform`.
///
/// This is deliberately different from wallets-list platforms such as `ios`,
/// `macos`, or `chrome`, which describe installation targets rather than the
/// runtime that produced a connect event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum TonConnectDevicePlatform {
    /// iPhone application.
    Iphone,
    /// iPad application.
    Ipad,
    /// Android application.
    Android,
    /// Windows application.
    Windows,
    /// macOS application.
    Mac,
    /// Linux application.
    Linux,
    /// Browser application or extension.
    Browser,
}

/// Host-owned subset of the TypeScript protocol's `DeviceInfo`.
///
/// Rust fills `maxProtocolVersion` and `features` from the methods the engine
/// actually implements, so platform clients cannot advertise unsupported
/// capabilities.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TonConnectDevice {
    /// Current wallet runtime platform.
    pub platform: TonConnectDevicePlatform,
    /// Wallet registry identifier.
    pub app_name: String,
    /// Wallet application version.
    pub app_version: String,
}

/// Initial dApp request that must be shown before connection approval.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TonConnectConnectPrompt {
    /// URL of `tonconnect-manifest.json`.
    pub manifest_url: String,
    /// Optional requested network identifier.
    pub requested_network: Option<String>,
    /// Optional ownership-proof challenge.
    pub proof_payload: Option<String>,
}

/// Validated dApp manifest fields needed by wallet UI and proof signing.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TonConnectManifest {
    /// Canonical dApp URL.
    pub url: String,
    /// Human-readable dApp name.
    pub name: String,
    /// HTTPS raster icon URL.
    pub icon_url: String,
    /// DNS domain bound into `ton_proof`.
    pub domain: String,
}

/// Signed ownership proof supplied when approving a connect request.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TonConnectProofReply {
    /// Unix signing timestamp in seconds.
    pub timestamp: u64,
    /// Manifest domain shown to the user.
    pub domain: String,
    /// Exact dApp challenge.
    pub payload: String,
    /// Exact 64-byte Ed25519 signature.
    pub signature: Vec<u8>,
}

/// A complete encrypted HTTP bridge POST.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TonConnectPreparedPost {
    /// Absolute bridge message URL including recipient and TTL.
    pub url: String,
    /// Canonical base64 encrypted body.
    pub body: String,
}

/// Public lifecycle phase of a TON Connect session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TonConnectSessionPhase {
    /// Waiting for connect approval.
    PendingConnect,
    /// Connected and accepting RPC requests.
    Connected,
    /// Terminally disconnected.
    Disconnected,
}

/// Kind of authenticated dApp request delivered to the wallet host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TonConnectIncomingRequestKind {
    /// A supported single-message raw `sendTransaction` request.
    SendTransaction,
    /// A dApp-initiated disconnect request.
    Disconnect,
    /// A malformed or unsupported RPC request.
    Unsupported,
}

/// TON Connect RPC error code returned for an unsupported request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TonConnectRpcErrorCode {
    /// Unexpected wallet-side failure.
    Unknown,
    /// Malformed request.
    BadRequest,
    /// Unknown or revoked application session.
    UnknownApp,
    /// User declined the request.
    UserDeclined,
    /// Wallet does not implement the method or shape.
    MethodNotSupported,
}

/// One replay-safe authenticated request awaiting a wallet response.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TonConnectIncomingRequest {
    /// Exact dApp request identifier.
    pub id: String,
    /// Exact RPC method name.
    pub method: String,
    /// Parsed request kind.
    pub kind: TonConnectIncomingRequestKind,
    /// Exact send intent when `kind` is `SendTransaction`.
    pub send_request: Option<SendRequest>,
    /// Recommended protocol error for unsupported input.
    pub error_code: Option<TonConnectRpcErrorCode>,
    /// Sanitized protocol diagnostic for unsupported input.
    pub error_message: Option<String>,
}

/// Failure at the FFI-safe TON Connect session boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum TonConnectSessionError {
    /// The session or protocol input is invalid.
    #[error("TON Connect session failed: {diagnostic}")]
    Failed {
        /// Bounded secret-free diagnostic.
        diagnostic: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedTonConnectSession {
    client: PersistedTonConnectClient,
    pending_requests: Vec<String>,
    pending_post: Option<TonConnectPreparedPost>,
    connected_network: Option<NetworkId>,
}

struct TonConnectSessionState {
    client: TonConnectClient,
    pending_requests: BTreeMap<String, IncomingRequest>,
    pending_post: Option<TonConnectPreparedPost>,
    connected_network: Option<NetworkId>,
}

/// Thread-safe TON Connect session exposed to Swift, Kotlin, and other hosts.
#[derive(uniffi::Object)]
pub struct TonConnectSession {
    state: Mutex<TonConnectSessionState>,
}

/// Parses and validates a dApp manifest with the Rust protocol implementation.
#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "UniFFI exports owned strings at the foreign-language boundary"
)]
pub fn parse_ton_connect_manifest(
    json: String,
) -> Result<TonConnectManifest, TonConnectSessionError> {
    let manifest = serde_json::from_str::<AppManifest>(&json).map_err(session_error)?;
    Ok(TonConnectManifest {
        url: manifest.url().as_str().to_owned(),
        name: manifest.name().to_owned(),
        icon_url: manifest.icon_url().as_str().to_owned(),
        domain: manifest.app_domain().map_err(session_error)?,
    })
}

/// Creates a fresh encrypted session from a full TON Connect link.
#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "UniFFI exports owned records and strings at the foreign-language boundary"
)]
pub fn ton_connect_session_from_link(
    link: String,
    config: TonConnectSessionConfig,
) -> Result<Arc<TonConnectSession>, TonConnectSessionError> {
    let client =
        TonConnectClient::from_link(&link, client_config(&config)?).map_err(session_error)?;
    Ok(Arc::new(TonConnectSession {
        state: Mutex::new(TonConnectSessionState {
            client,
            pending_requests: BTreeMap::new(),
            pending_post: None,
            connected_network: None,
        }),
    }))
}

/// Restores a secret-bearing session without generating new crypto keys.
#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "UniFFI exports owned records and strings at the foreign-language boundary"
)]
pub fn ton_connect_session_restore(
    persisted: String,
    config: TonConnectSessionConfig,
) -> Result<Arc<TonConnectSession>, TonConnectSessionError> {
    let persisted =
        serde_json::from_str::<PersistedTonConnectSession>(&persisted).map_err(session_error)?;
    let max_event_bytes = nonzero_usize(config.max_event_bytes, "max event bytes")?;
    let message_ttl_seconds = NonZeroU32::new(config.message_ttl_seconds)
        .ok_or_else(|| failed("message TTL must be positive"))?;
    let client = TonConnectClient::restore(
        &persisted.client,
        max_event_bytes,
        message_ttl_seconds,
        HeartbeatMode::Message,
    )
    .map_err(session_error)?;
    let mut pending_requests = BTreeMap::new();
    for incoming in persisted.pending_requests {
        let incoming = IncomingRequest::restore(&incoming).map_err(session_error)?;
        let id = incoming.request().id.clone();
        if pending_requests.insert(id, incoming).is_some() {
            return Err(failed(
                "persisted session contains duplicate pending request IDs",
            ));
        }
    }
    Ok(Arc::new(TonConnectSession {
        state: Mutex::new(TonConnectSessionState {
            client,
            pending_requests,
            pending_post: persisted.pending_post,
            connected_network: persisted.connected_network,
        }),
    }))
}

#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "UniFFI exports owned records, byte buffers, and strings at the foreign-language boundary"
)]
impl TonConnectSession {
    /// Returns the current lifecycle phase.
    pub fn phase(&self) -> Result<TonConnectSessionPhase, TonConnectSessionError> {
        let state = self.lock()?;
        Ok(match state.client.phase() {
            WalletSessionPhase::PendingConnect => TonConnectSessionPhase::PendingConnect,
            WalletSessionPhase::Connected => TonConnectSessionPhase::Connected,
            WalletSessionPhase::Disconnected => TonConnectSessionPhase::Disconnected,
        })
    }

    /// Returns the initial connect prompt while approval is pending.
    pub fn connect_prompt(
        &self,
    ) -> Result<Option<TonConnectConnectPrompt>, TonConnectSessionError> {
        let state = self.lock()?;
        let Some(request) = state.client.connect_request() else {
            return Ok(None);
        };
        let requested_network = request.items.as_slice().iter().find_map(|item| match item {
            ConnectItem::TonAddr { network } => {
                network.as_ref().map(|value| value.as_str().to_owned())
            }
            ConnectItem::TonProof { .. } | ConnectItem::Unsupported { .. } => None,
        });
        let proof_payload = request.items.as_slice().iter().find_map(|item| match item {
            ConnectItem::TonProof { payload } => Some(payload.clone()),
            ConnectItem::TonAddr { .. } | ConnectItem::Unsupported { .. } => None,
        });
        Ok(Some(TonConnectConnectPrompt {
            manifest_url: request.manifest_url.as_str().to_owned(),
            requested_network,
            proof_payload,
        }))
    }

    /// Returns a complete SSE subscription URL and resets stream framing.
    pub fn begin_events_subscription(&self) -> Result<String, TonConnectSessionError> {
        let mut state = self.lock()?;
        if state.pending_post.is_some() {
            return Err(failed(
                "a durable bridge response must be delivered before reading events",
            ));
        }
        Ok(state.client.begin_events_subscription().to_string())
    }

    /// Approves the connect request and keeps its encrypted response pending delivery.
    ///
    /// Persist the session before posting the returned response. After the
    /// bridge accepts it, call [`Self::complete_pending_post`].
    pub fn approve_connect(
        &self,
        account: TonConnectAccountInfo,
        proof: Option<TonConnectProofReply>,
        device: TonConnectDevice,
    ) -> Result<TonConnectPreparedPost, TonConnectSessionError> {
        let mut state = self.lock()?;
        ensure_no_pending_post(&state)?;
        let request = state
            .client
            .connect_request()
            .cloned()
            .ok_or_else(|| failed("connect request is no longer pending"))?;
        let (account_reply, network) = account_reply(&account)?;
        let mut items = Vec::with_capacity(request.items.as_slice().len());
        for item in request.items.as_slice() {
            let reply = match item {
                ConnectItem::TonAddr { .. } => ConnectItemReply::TonAddress(account_reply.clone()),
                ConnectItem::TonProof { payload } => {
                    let proof = proof
                        .as_ref()
                        .filter(|proof| &proof.payload == payload)
                        .ok_or_else(|| {
                            failed("connect request requires a matching ownership proof")
                        })?;
                    ConnectItemReply::TonProof(TonProofItemReply::new(proof_reply(proof)?))
                }
                ConnectItem::Unsupported { .. } => {
                    ConnectItemReply::unsupported(item, Some("Method is not supported".to_owned()))
                }
            };
            items.push(reply);
        }
        let feature = SendTransactionFeature::new(1, Some(false), None).map_err(session_error)?;
        let device = DeviceInfo::new(
            device_platform(device.platform),
            device.app_name,
            device.app_version,
            u32::from(ton_connect_core::PROTOCOL_VERSION),
            vec![Feature::SendTransaction(feature)],
        )
        .map_err(session_error)?;
        let embedded_response = state.client.embedded_request().map(|_| {
            EmbeddedResponse::Error(EmbeddedResponseError {
                error: WalletResponseError {
                    code: RpcErrorCode::MethodNotSupported,
                    message: "Embedded request is not supported".to_owned(),
                    data: None,
                },
            })
        });
        let post = state
            .client
            .approve_connect(ConnectEventPayload { items, device }, embedded_response)
            .map_err(session_error)?;
        state.connected_network = Some(network);
        Ok(stage_post(&mut state, &post))
    }

    /// Rejects the initial connection and keeps its terminal event pending delivery.
    ///
    /// Persist the session before posting the returned response. After the
    /// bridge accepts it, call [`Self::complete_pending_post`].
    pub fn reject_connect(
        &self,
        message: String,
    ) -> Result<TonConnectPreparedPost, TonConnectSessionError> {
        let mut state = self.lock()?;
        ensure_no_pending_post(&state)?;
        let post = state
            .client
            .reject_connect(
                ConnectEventErrorCode::UserDeclined,
                bounded_diagnostic(message),
            )
            .map_err(session_error)?;
        Ok(stage_post(&mut state, &post))
    }

    /// Ingests an SSE chunk and returns only fresh authenticated requests.
    pub fn ingest_sse_chunk(
        &self,
        chunk: Vec<u8>,
        now: u64,
    ) -> Result<Vec<TonConnectIncomingRequest>, TonConnectSessionError> {
        let mut state = self.lock()?;
        let incoming = state
            .client
            .ingest_sse_chunk(&chunk)
            .map_err(session_error)?;
        let mut decoded = Vec::with_capacity(incoming.len());
        for request in incoming {
            let public = decode_incoming(&state, &request, now)?;
            let id = request.request().id.clone();
            if state.pending_requests.insert(id, request).is_some() {
                return Err(failed("bridge delivered a duplicate pending request ID"));
            }
            decoded.push(public);
        }
        Ok(decoded)
    }

    /// Returns restored requests that still await a wallet response.
    pub fn pending_requests(
        &self,
        now: u64,
    ) -> Result<Vec<TonConnectIncomingRequest>, TonConnectSessionError> {
        let state = self.lock()?;
        state
            .pending_requests
            .values()
            .map(|request| decode_incoming(&state, request, now))
            .collect()
    }

    /// Keeps a successful `sendTransaction` response pending bridge delivery.
    ///
    /// The response contains the exact signed BOC returned to the dApp.
    pub fn prepare_send_success(
        &self,
        request_id: String,
        signed_boc: String,
    ) -> Result<TonConnectPreparedPost, TonConnectSessionError> {
        let response = WalletResponse::Success(WalletResponseSuccess {
            result: WalletResult::String(signed_boc),
            id: request_id.clone(),
        });
        self.prepare_response(&request_id, &response)
    }

    /// Keeps a successful dApp-initiated disconnect response pending delivery.
    pub fn prepare_disconnect_success(
        &self,
        request_id: String,
    ) -> Result<TonConnectPreparedPost, TonConnectSessionError> {
        let response = WalletResponse::Success(WalletResponseSuccess {
            result: WalletResult::Object(serde_json::Map::new()),
            id: request_id.clone(),
        });
        self.prepare_response(&request_id, &response)
    }

    /// Keeps a protocol RPC error pending delivery for the selected request.
    pub fn prepare_error(
        &self,
        request_id: String,
        code: TonConnectRpcErrorCode,
        message: String,
    ) -> Result<TonConnectPreparedPost, TonConnectSessionError> {
        let response = WalletResponse::Error {
            error: WalletResponseError {
                code: rpc_error_code(code),
                message: bounded_diagnostic(message),
                data: None,
            },
            id: request_id.clone(),
        };
        self.prepare_response(&request_id, &response)
    }

    /// Keeps a wallet-initiated disconnect event pending bridge delivery.
    pub fn disconnect(&self) -> Result<TonConnectPreparedPost, TonConnectSessionError> {
        let mut state = self.lock()?;
        ensure_no_pending_post(&state)?;
        let post = state.client.disconnect(None).map_err(session_error)?;
        Ok(stage_post(&mut state, &post))
    }

    /// Returns the exact response that must be retried before reading more events.
    pub fn pending_post(&self) -> Result<Option<TonConnectPreparedPost>, TonConnectSessionError> {
        Ok(self.lock()?.pending_post.clone())
    }

    /// Removes the pending response after the bridge accepts it.
    pub fn complete_pending_post(&self) -> Result<(), TonConnectSessionError> {
        self.lock()?.pending_post = None;
        Ok(())
    }

    /// Serializes the secret-bearing session, requests, and pending response.
    pub fn persisted(&self) -> Result<String, TonConnectSessionError> {
        let state = self.lock()?;
        let persisted = PersistedTonConnectSession {
            client: state.client.persisted().map_err(session_error)?,
            pending_requests: state
                .pending_requests
                .values()
                .map(IncomingRequest::persisted)
                .collect::<Result<Vec<_>, _>>()
                .map_err(session_error)?,
            pending_post: state.pending_post.clone(),
            connected_network: state.connected_network.clone(),
        };
        serde_json::to_string(&persisted).map_err(session_error)
    }
}

impl TonConnectSession {
    fn lock(&self) -> Result<MutexGuard<'_, TonConnectSessionState>, TonConnectSessionError> {
        self.state
            .lock()
            .map_err(|_| failed("session state lock is unavailable"))
    }

    fn prepare_response(
        &self,
        request_id: &str,
        response: &WalletResponse,
    ) -> Result<TonConnectPreparedPost, TonConnectSessionError> {
        let mut state = self.lock()?;
        ensure_no_pending_post(&state)?;
        let post = {
            let incoming = state
                .pending_requests
                .get(request_id)
                .ok_or_else(|| failed("request is not pending"))?;
            state
                .client
                .prepare_response(incoming, response)
                .map_err(session_error)?
        };
        let _ = state.pending_requests.remove(request_id);
        Ok(stage_post(&mut state, &post))
    }
}

fn client_config(
    config: &TonConnectSessionConfig,
) -> Result<TonConnectClientConfig, TonConnectSessionError> {
    let bridge_url = HttpBridgeUrl::try_from(config.bridge_url.as_str()).map_err(session_error)?;
    let max_event_bytes = nonzero_usize(config.max_event_bytes, "max event bytes")?;
    let ttl = NonZeroU32::new(config.message_ttl_seconds)
        .ok_or_else(|| failed("message TTL must be positive"))?;
    Ok(TonConnectClientConfig::new(
        bridge_url,
        max_event_bytes,
        ttl,
        HeartbeatMode::Message,
    ))
}

fn nonzero_usize(value: u64, name: &str) -> Result<NonZeroUsize, TonConnectSessionError> {
    let value = usize::try_from(value).map_err(|_| failed(format!("{name} exceeds usize")))?;
    NonZeroUsize::new(value).ok_or_else(|| failed(format!("{name} must be positive")))
}

fn ensure_no_pending_post(state: &TonConnectSessionState) -> Result<(), TonConnectSessionError> {
    if state.pending_post.is_some() {
        Err(failed(
            "a durable bridge response is already pending delivery",
        ))
    } else {
        Ok(())
    }
}

fn stage_post(
    state: &mut TonConnectSessionState,
    post: &ton_connect_core::PreparedBridgePost,
) -> TonConnectPreparedPost {
    let post = TonConnectPreparedPost {
        url: post.url().to_string(),
        body: post.body().as_str().to_owned(),
    };
    state.pending_post = Some(post.clone());
    post
}

fn account_reply(
    account: &TonConnectAccountInfo,
) -> Result<(TonAddressItemReply, NetworkId), TonConnectSessionError> {
    let address = account
        .address
        .parse::<RawAccountAddress>()
        .map_err(session_error)?;
    let network = NetworkId::try_from(account.network.as_str()).map_err(session_error)?;
    let state_init =
        WalletStateInit::try_from(account.wallet_state_init.as_str()).map_err(session_error)?;
    let public_key = <[u8; 32]>::try_from(account.public_key.as_slice())
        .map_err(|_| failed("TON Connect public key must contain 32 bytes"))?;
    Ok((
        TonAddressItemReply::new(
            address,
            network.clone(),
            state_init,
            Ed25519PublicKey::from_bytes(public_key),
        ),
        network,
    ))
}

fn proof_reply(proof: &TonConnectProofReply) -> Result<TonProof, TonConnectSessionError> {
    let signature = <[u8; 64]>::try_from(proof.signature.as_slice())
        .map_err(|_| failed("TON Connect proof signature must contain 64 bytes"))?;
    Ok(TonProof {
        timestamp: Uint64String::from(proof.timestamp),
        domain: TonProofDomain::new(proof.domain.clone()).map_err(session_error)?,
        payload: proof.payload.clone(),
        signature: Ed25519Signature::from_bytes(signature),
    })
}

fn decode_incoming(
    state: &TonConnectSessionState,
    incoming: &IncomingRequest,
    now: u64,
) -> Result<TonConnectIncomingRequest, TonConnectSessionError> {
    let id = incoming.request().id.clone();
    let method = incoming.request().method.clone();
    let decoded = match incoming.decode() {
        Ok(KnownAppRequest::SendTransaction(request)) => {
            decode_send_request(state, &id, method, request.payload, now)?
        }
        Ok(KnownAppRequest::Disconnect(_)) => TonConnectIncomingRequest {
            id,
            method,
            kind: TonConnectIncomingRequestKind::Disconnect,
            send_request: None,
            error_code: None,
            error_message: None,
        },
        Ok(KnownAppRequest::SignMessage(_) | KnownAppRequest::SignData(_)) => unsupported_request(
            id,
            method,
            TonConnectRpcErrorCode::MethodNotSupported,
            "Method is not supported",
        ),
        Err(error) => unsupported_request(
            id,
            method,
            TonConnectRpcErrorCode::BadRequest,
            &error.to_string(),
        ),
    };
    Ok(decoded)
}

fn decode_send_request(
    state: &TonConnectSessionState,
    request_id: &str,
    method: String,
    payload: TransactionPayload,
    now: u64,
) -> Result<TonConnectIncomingRequest, TonConnectSessionError> {
    let network = state
        .connected_network
        .as_ref()
        .ok_or_else(|| failed("connected network is unavailable"))?;
    let address = state
        .client
        .connected_address()
        .ok_or_else(|| failed("connected address is unavailable"))?;
    if let Err(error) = payload.validate_context(now, network, &address) {
        return Ok(unsupported_request(
            request_id.to_owned(),
            method,
            TonConnectRpcErrorCode::BadRequest,
            &error.to_string(),
        ));
    }
    let TransactionPayload::Raw(payload) = payload else {
        return Ok(unsupported_request(
            request_id.to_owned(),
            method,
            TonConnectRpcErrorCode::MethodNotSupported,
            "Structured transactions are not supported",
        ));
    };
    if payload.messages.as_slice().len() != 1 {
        return Ok(unsupported_request(
            request_id.to_owned(),
            method,
            TonConnectRpcErrorCode::MethodNotSupported,
            "Only one outgoing message is supported",
        ));
    }
    let Some(message) = payload.messages.into_vec().into_iter().next() else {
        return Err(failed("validated transaction has no outgoing message"));
    };
    if message
        .extra_currency
        .as_ref()
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(unsupported_request(
            request_id.to_owned(),
            method,
            TonConnectRpcErrorCode::MethodNotSupported,
            "Extra currencies are not supported",
        ));
    }
    let payload_boc = message
        .payload
        .map(|value| Boc::try_from(value.as_bytes().to_vec()))
        .transpose()
        .map_err(session_error)?;
    let state_init = message
        .state_init
        .map(|value| Boc::try_from(value.as_bytes().to_vec()))
        .transpose()
        .map_err(session_error)?;
    let operation_id = NonEmptyString::try_from(format!(
        "ton-connect:{}:{request_id}",
        state.client.client_id()
    ))
    .map_err(session_error)?;
    let send_request = SendRequest {
        operation_id,
        force: false,
        intent: SendIntent {
            expiration: payload
                .valid_until
                .map_or(SendExpiration::EngineDefault, |value| {
                    SendExpiration::Exact {
                        unix_timestamp: value,
                    }
                }),
            message: SendMessage {
                destination: crate::TonAddressString::try_from(message.address.to_string())
                    .map_err(session_error)?,
                amount: SendAmount::exact(message.amount.as_str().to_owned())
                    .map_err(session_error)?,
                body: payload_boc.map_or(SendMessageBody::Empty, |boc| {
                    SendMessageBody::RawPayload { boc }
                }),
                state_init,
            },
        },
    };
    Ok(TonConnectIncomingRequest {
        id: request_id.to_owned(),
        method,
        kind: TonConnectIncomingRequestKind::SendTransaction,
        send_request: Some(send_request),
        error_code: None,
        error_message: None,
    })
}

fn unsupported_request(
    id: String,
    method: String,
    code: TonConnectRpcErrorCode,
    message: &str,
) -> TonConnectIncomingRequest {
    TonConnectIncomingRequest {
        id,
        method,
        kind: TonConnectIncomingRequestKind::Unsupported,
        send_request: None,
        error_code: Some(code),
        error_message: Some(bounded_diagnostic(message)),
    }
}

const fn device_platform(platform: TonConnectDevicePlatform) -> DevicePlatform {
    match platform {
        TonConnectDevicePlatform::Iphone => DevicePlatform::Iphone,
        TonConnectDevicePlatform::Ipad => DevicePlatform::Ipad,
        TonConnectDevicePlatform::Android => DevicePlatform::Android,
        TonConnectDevicePlatform::Windows => DevicePlatform::Windows,
        TonConnectDevicePlatform::Mac => DevicePlatform::Mac,
        TonConnectDevicePlatform::Linux => DevicePlatform::Linux,
        TonConnectDevicePlatform::Browser => DevicePlatform::Browser,
    }
}

const fn rpc_error_code(code: TonConnectRpcErrorCode) -> RpcErrorCode {
    match code {
        TonConnectRpcErrorCode::Unknown => RpcErrorCode::Unknown,
        TonConnectRpcErrorCode::BadRequest => RpcErrorCode::BadRequest,
        TonConnectRpcErrorCode::UnknownApp => RpcErrorCode::UnknownApp,
        TonConnectRpcErrorCode::UserDeclined => RpcErrorCode::UserDeclined,
        TonConnectRpcErrorCode::MethodNotSupported => RpcErrorCode::MethodNotSupported,
    }
}

fn session_error(error: impl std::fmt::Display) -> TonConnectSessionError {
    failed(error.to_string())
}

fn failed(diagnostic: impl Into<String>) -> TonConnectSessionError {
    TonConnectSessionError::Failed {
        diagnostic: bounded_diagnostic(diagnostic.into()),
    }
}

impl From<WalletClientError> for TonConnectSessionError {
    fn from(value: WalletClientError) -> Self {
        session_error(value)
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ton::ton_core::{cell::TonCell, traits::tlb::TLB as _};
    use ton_connect_core::{
        AppRequest, ClientId, ConnectLink, ConnectRequest, HttpsUrl, NonEmptyVec, ReturnStrategy,
        SessionCrypto, TonAddressItem,
    };

    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// Keeps the public enum and its wire names exhaustive with TypeScript `DeviceInfo.platform`.
    #[test]
    fn ffi_device_platforms_match_typescript_and_wire_names() -> TestResult {
        let cases = [
            (
                TonConnectDevicePlatform::Iphone,
                DevicePlatform::Iphone,
                "iphone",
            ),
            (TonConnectDevicePlatform::Ipad, DevicePlatform::Ipad, "ipad"),
            (
                TonConnectDevicePlatform::Android,
                DevicePlatform::Android,
                "android",
            ),
            (
                TonConnectDevicePlatform::Windows,
                DevicePlatform::Windows,
                "windows",
            ),
            (TonConnectDevicePlatform::Mac, DevicePlatform::Mac, "mac"),
            (
                TonConnectDevicePlatform::Linux,
                DevicePlatform::Linux,
                "linux",
            ),
            (
                TonConnectDevicePlatform::Browser,
                DevicePlatform::Browser,
                "browser",
            ),
        ];
        for (public, protocol, wire_name) in cases {
            assert_eq!(device_platform(public), protocol);
            assert_eq!(serde_json::to_string(&public)?, format!("\"{wire_name}\""));
        }
        Ok(())
    }

    #[test]
    fn ffi_session_preserves_authenticated_request_and_response_across_restart() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let link = ConnectLink::connect(
            dapp.client_id(),
            ConnectRequest {
                manifest_url: HttpsUrl::try_from("https://app.example/manifest.json")?,
                items: NonEmptyVec::try_from(vec![ConnectItem::from(TonAddressItem {
                    network: Some(NetworkId::try_from("-3")?),
                })])?,
            },
            ReturnStrategy::None,
            None,
            None,
        )
        .to_url("tc://")?;
        let config = TonConnectSessionConfig {
            bridge_url: "https://bridge.example/bridge".to_owned(),
            max_event_bytes: 4096,
            message_ttl_seconds: 300,
        };
        let session = ton_connect_session_from_link(link.to_string(), config.clone())?;
        assert_eq!(
            session.connect_prompt()?.map(|value| value.manifest_url),
            Some("https://app.example/manifest.json".to_owned())
        );

        let mut state = TonCell::builder();
        state.write_bit(false)?;
        state.write_bit(false)?;
        state.write_bit(true)?;
        state.write_ref(TonCell::empty().to_owned())?;
        state.write_bit(true)?;
        state.write_ref(TonCell::empty().to_owned())?;
        state.write_bit(false)?;
        let state = WalletStateInit::from_boc(state.build()?.to_boc()?)?;
        let account = TonConnectAccountInfo {
            address: state.derive_address(0)?.to_string(),
            network: "-3".to_owned(),
            wallet_state_init: state.as_str().to_owned(),
            public_key: vec![0_u8; 32],
        };
        let connect_post = session.approve_connect(
            account.clone(),
            None,
            TonConnectDevice {
                platform: TonConnectDevicePlatform::Iphone,
                app_name: "test-wallet".to_owned(),
                app_version: "1.0.0".to_owned(),
            },
        )?;
        assert_eq!(session.pending_post()?, Some(connect_post));
        session.complete_pending_post()?;

        let events_url = url::Url::parse(&session.begin_events_subscription()?)?;
        let wallet_client_id = events_url
            .query_pairs()
            .find_map(|(key, value)| (key == "client_id").then(|| value.into_owned()))
            .ok_or("events URL has no client_id")?
            .parse::<ClientId>()?;
        let request = AppRequest {
            method: "sendTransaction".to_owned(),
            params: vec![
                serde_json::json!({
                    "valid_until": 1_900_000_000_u64,
                    "network": "-3",
                    "from": account.address,
                    "messages": [{
                        "address": "Ef8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADAU",
                        "amount": "1000000",
                        "payload": "te6ccgEBAQEAAgAAAA=="
                    }]
                })
                .to_string(),
            ],
            id: "7".to_owned(),
        };
        let encrypted = dapp.encrypt(wallet_client_id, &serde_json::to_vec(&request)?)?;
        let envelope = serde_json::json!({
            "from": dapp.client_id().to_string(),
            "message": STANDARD.encode(encrypted)
        });
        let sse = format!("id: 1\nevent: message\ndata: {envelope}\n\n");
        let incoming = session.ingest_sse_chunk(sse.into_bytes(), 1_800_000_000)?;
        let Some(first_incoming) = incoming.first() else {
            return Err("authenticated request was not delivered".into());
        };
        assert_eq!(
            first_incoming.kind,
            TonConnectIncomingRequestKind::SendTransaction
        );
        assert_eq!(
            first_incoming
                .send_request
                .as_ref()
                .map(|request| &request.intent.expiration),
            Some(&SendExpiration::Exact {
                unix_timestamp: 1_900_000_000,
            })
        );
        assert!(matches!(
            first_incoming
                .send_request
                .as_ref()
                .map(|request| &request.intent.message.body),
            Some(SendMessageBody::RawPayload { .. })
        ));
        assert!(
            first_incoming
                .send_request
                .as_ref()
                .is_some_and(|request| request.intent.message.state_init.is_none())
        );

        let persisted = session.persisted()?;
        let restored = ton_connect_session_restore(persisted, config)?;
        assert_eq!(restored.pending_requests(1_800_000_000)?, incoming);
        let response =
            restored.prepare_send_success("7".to_owned(), "te6ccgEBAQEAAgAAAA==".to_owned())?;
        assert_eq!(restored.pending_post()?, Some(response));
        restored.complete_pending_post()?;
        assert!(restored.pending_requests(1_800_000_000)?.is_empty());
        Ok(())
    }

    #[test]
    fn manifest_validation_stays_in_rust() -> TestResult {
        let manifest = parse_ton_connect_manifest(
            r#"{"url":"https://app.example.com","name":"Example","iconUrl":"https://app.example.com/icon.png"}"#
                .to_owned(),
        )?;
        assert_eq!(manifest.domain, "app.example.com");
        assert!(parse_ton_connect_manifest("{}".to_owned()).is_err());
        Ok(())
    }
}
