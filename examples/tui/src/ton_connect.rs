//! TON Connect HTTP bridge controller for the interactive TUI.

use std::{num::NonZeroU32, num::NonZeroUsize, str::FromStr as _, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt as _;
use reqwest::Client;
use serde::Serialize;
use serde_json::Map;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use ton_connect_core::{
    AppManifest, AppRequest, BridgeSseDecoder, ClientId, ConnectEvent, ConnectEventError,
    ConnectEventErrorCode, ConnectEventPayload, ConnectItem, ConnectItemReply, ConnectLink,
    DeviceInfo, DevicePlatform, Ed25519PublicKey, Ed25519Signature, Feature, HttpBridgeUrl,
    KnownAppRequest, NetworkId, RawAccountAddress, RawTransactionPayload, RpcError, RpcErrorCode,
    SendTransactionFeature, SendTransactionRequest, SessionCrypto, TonAddressItemReply, TonProof,
    TonProofDomain, TonProofItemReply, TraceId, TransactionPayload, Uint64String, WalletEventKind,
    WalletResponse, WalletResponseError, WalletResponseSuccess, WalletResult, WalletSessionState,
    WalletStateInit,
};
use wallet_engine::{
    Boc, Network, NonEmptyString, SendAmount, SendPhase, SendRequest, TonAddressString,
    TonConnectProofSignRequest, WalletClient, WalletDescriptor, WalletLifecycle,
};

const DEFAULT_BRIDGE_URL: &str = "https://connect.ton.org/bridge";
const MANIFEST_LIMIT_BYTES: usize = 1_048_576;
const SSE_EVENT_LIMIT_BYTES: usize = 1_048_576;
const BRIDGE_TTL_SECONDS: u32 = 300;
const HTTP_TIMEOUT_SECONDS: u64 = 15;
const RECONNECT_DELAY_SECONDS: u64 = 1;
const DEMO_WALLET_APP_NAME: &str = "tonkeeper";

pub(crate) struct ConnectPrompt {
    pub(crate) dapp_name: String,
    pub(crate) origin: String,
    pub(crate) icon_url: String,
    pub(crate) domain: String,
    pub(crate) account: String,
    pub(crate) proof_payload: Option<String>,
    response: Option<oneshot::Sender<bool>>,
}

impl ConnectPrompt {
    pub(crate) fn respond(mut self, approved: bool) {
        if let Some(response) = self.response.take() {
            let _ = response.send(approved);
        }
    }
}

pub(crate) struct TransactionPrompt {
    pub(crate) dapp_name: String,
    pub(crate) destination: String,
    pub(crate) amount_nanograms: String,
    pub(crate) deploys_contract: bool,
    pub(crate) has_payload: bool,
    response: Option<oneshot::Sender<bool>>,
}

impl TransactionPrompt {
    pub(crate) fn respond(mut self, approved: bool) {
        if let Some(response) = self.response.take() {
            let _ = response.send(approved);
        }
    }
}

pub(crate) enum TonConnectEvent {
    ConnectPrompt(ConnectPrompt),
    Connected { dapp_name: String, account: String },
    TransactionPrompt(TransactionPrompt),
    TransactionFinished(String),
    Disconnected,
    Failed(String),
}

pub(crate) struct TonConnectController {
    events: mpsc::UnboundedReceiver<TonConnectEvent>,
    cancellation: CancellationToken,
    task: JoinHandle<()>,
}

impl TonConnectController {
    pub(crate) fn start(
        link: String,
        descriptor: WalletDescriptor,
        lifecycle: Arc<WalletLifecycle>,
        wallet_client: Arc<WalletClient>,
    ) -> Self {
        let (events_tx, events) = mpsc::unbounded_channel();
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let task = tokio::spawn(async move {
            let result = run(
                &link,
                descriptor,
                lifecycle,
                wallet_client,
                &events_tx,
                &task_cancellation,
            )
            .await;
            if let Err(error) = result {
                let _ = events_tx.send(TonConnectEvent::Failed(error.to_string()));
            }
        });
        Self {
            events,
            cancellation,
            task,
        }
    }

    pub(crate) fn try_next(&mut self) -> Option<TonConnectEvent> {
        self.events.try_recv().ok()
    }

    pub(crate) async fn shutdown(self) {
        self.cancellation.cancel();
        let _ = self.task.await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    link_value: &str,
    descriptor: WalletDescriptor,
    lifecycle: Arc<WalletLifecycle>,
    wallet_client: Arc<WalletClient>,
    events: &mpsc::UnboundedSender<TonConnectEvent>,
    cancellation: &CancellationToken,
) -> Result<()> {
    let link = ConnectLink::parse(link_value)?;
    let request = link
        .request()
        .ok_or_else(|| anyhow!("connect link does not contain a full request"))?;
    let bridge_value =
        std::env::var("TON_CONNECT_BRIDGE_URL").unwrap_or_else(|_| DEFAULT_BRIDGE_URL.to_owned());
    let bridge = HttpBridgeUrl::try_from(bridge_value.as_str())?;
    let http = Client::builder()
        .connect_timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("wallet-engine-tui/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let session = SessionCrypto::generate()?;
    let account = ton_connect_account(&lifecycle, &descriptor)?;
    enforce_connect_network(request.items.as_slice(), &account.network)?;
    let manifest = load_manifest(&http, request.manifest_url.as_str()).await?;
    let domain = manifest.app_domain()?;

    let (approval_tx, approval_rx) = oneshot::channel();
    events
        .send(TonConnectEvent::ConnectPrompt(ConnectPrompt {
            dapp_name: manifest.name().to_owned(),
            origin: manifest.url().to_string(),
            icon_url: manifest.icon_url().to_string(),
            domain: domain.clone(),
            account: account.address.to_string(),
            proof_payload: request.items.as_slice().iter().find_map(|item| match item {
                ConnectItem::TonProof { payload } => Some(payload.clone()),
                ConnectItem::TonAddr { .. } | ConnectItem::Unsupported { .. } => None,
            }),
            response: Some(approval_tx),
        }))
        .map_err(|_| anyhow!("TUI event receiver closed"))?;
    let approved = tokio::select! {
        result = approval_rx => result.unwrap_or(false),
        () = cancellation.cancelled() => return Ok(()),
    };
    if !approved {
        send_connect_error(
            &http,
            &bridge,
            &session,
            link.client_id(),
            link.trace_id(),
            ConnectEventErrorCode::UserDeclined,
            "User declined the connection",
        )
        .await?;
        return Ok(());
    }

    let transition =
        WalletSessionState::pending_connect().prepare_event(WalletEventKind::Connect)?;
    let connect_event = connect_event(
        transition.id(),
        request.items.as_slice(),
        &account,
        &lifecycle,
        &descriptor,
        &domain,
    )
    .await?;
    let mut state = transition.into_state();
    send_encrypted(
        &http,
        &bridge,
        &session,
        link.client_id(),
        &connect_event,
        None,
        link.trace_id(),
    )
    .await?;
    events
        .send(TonConnectEvent::Connected {
            dapp_name: manifest.name().to_owned(),
            account: account.address.to_string(),
        })
        .map_err(|_| anyhow!("TUI event receiver closed"))?;

    listen(
        &http,
        &bridge,
        &session,
        link.client_id(),
        &mut state,
        &wallet_client,
        &descriptor,
        manifest.name(),
        events,
        cancellation,
    )
    .await
}

fn ton_connect_account(
    lifecycle: &WalletLifecycle,
    descriptor: &WalletDescriptor,
) -> Result<TonAddressItemReply> {
    let info = lifecycle.ton_connect_account(descriptor.clone())?;
    let public_key = <[u8; 32]>::try_from(info.public_key.as_slice())
        .map_err(|_| anyhow!("wallet public key is not 32 bytes"))?;
    Ok(TonAddressItemReply::new(
        RawAccountAddress::from_str(&info.address)?,
        NetworkId::try_from(info.network.as_str())?,
        WalletStateInit::try_from(info.wallet_state_init)?,
        Ed25519PublicKey::from_bytes(public_key),
    ))
}

fn enforce_connect_network(items: &[ConnectItem], active: &NetworkId) -> Result<()> {
    let mut has_address = false;
    for item in items {
        if let ConnectItem::TonAddr { network } = item {
            has_address = true;
            if network
                .as_ref()
                .is_some_and(|requested| requested != active)
            {
                bail!("dApp requested a different network");
            }
        }
    }
    if !has_address {
        bail!("connect request does not contain ton_addr");
    }
    Ok(())
}

async fn load_manifest(client: &Client, url: &str) -> Result<AppManifest> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .send()
        .await?
        .error_for_status()?;
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len().saturating_add(chunk.len()) > MANIFEST_LIMIT_BYTES {
            bail!("manifest response exceeds one MiB");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&body)?)
}

async fn connect_event(
    id: u64,
    requested: &[ConnectItem],
    account: &TonAddressItemReply,
    lifecycle: &WalletLifecycle,
    descriptor: &WalletDescriptor,
    domain: &str,
) -> Result<ConnectEvent> {
    let mut items = Vec::new();
    for item in requested {
        match item {
            ConnectItem::TonAddr { .. } => {
                items.push(ConnectItemReply::TonAddress(account.clone()));
            }
            ConnectItem::TonProof { payload } => {
                let timestamp = unix_timestamp()?;
                let signed = lifecycle
                    .sign_ton_connect_proof(TonConnectProofSignRequest {
                        descriptor: descriptor.clone(),
                        domain: domain.to_owned(),
                        timestamp,
                        payload: payload.clone(),
                    })
                    .await?;
                let signature = <[u8; 64]>::try_from(signed.signature.as_slice())
                    .map_err(|_| anyhow!("TON Connect signature is not 64 bytes"))?;
                items.push(ConnectItemReply::TonProof(TonProofItemReply::new(
                    TonProof {
                        timestamp: Uint64String::from(timestamp),
                        domain: TonProofDomain::new(domain.to_owned())?,
                        payload: payload.clone(),
                        signature: Ed25519Signature::from_bytes(signature),
                    },
                )));
            }
            ConnectItem::Unsupported { .. } => {
                items.push(ConnectItemReply::unsupported(item, None));
            }
        }
    }
    Ok(ConnectEvent::Connect {
        id,
        payload: ConnectEventPayload {
            items,
            device: DeviceInfo {
                platform: current_platform(),
                app_name: DEMO_WALLET_APP_NAME.to_owned(),
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
                max_protocol_version: u32::from(ton_connect_core::PROTOCOL_VERSION),
                features: vec![Feature::SendTransaction(SendTransactionFeature::new(
                    1,
                    Some(false),
                    None,
                )?)],
            },
        },
        response: None,
    })
}

#[allow(clippy::too_many_arguments)]
async fn listen(
    client: &Client,
    bridge: &HttpBridgeUrl,
    session: &SessionCrypto,
    peer: ClientId,
    state: &mut WalletSessionState,
    wallet_client: &Arc<WalletClient>,
    descriptor: &WalletDescriptor,
    dapp_name: &str,
    events: &mpsc::UnboundedSender<TonConnectEvent>,
    cancellation: &CancellationToken,
) -> Result<()> {
    let limit = NonZeroUsize::new(SSE_EVENT_LIMIT_BYTES)
        .ok_or_else(|| anyhow!("invalid SSE event limit"))?;
    let mut last_event_id = None::<String>;
    loop {
        let endpoint = bridge.events_endpoint(session.client_id(), last_event_id.as_deref(), None);
        let response = tokio::select! {
            result = client.get(endpoint).send() => result?,
            () = cancellation.cancelled() => return Ok(()),
        };
        if !response.status().is_success() {
            bail!("bridge returned HTTP {}", response.status());
        }
        let mut decoder = BridgeSseDecoder::new(limit);
        let mut stream = response.bytes_stream();
        loop {
            let chunk = tokio::select! {
                chunk = stream.next() => chunk,
                () = cancellation.cancelled() => return Ok(()),
            };
            let Some(chunk) = chunk else {
                break;
            };
            for event in decoder.push(&chunk?)? {
                if let Some(event_id) = event.event_id() {
                    last_event_id = Some(event_id.to_owned());
                }
                let envelope = event.into_message();
                if envelope.from() != peer {
                    continue;
                }
                let encrypted = envelope.message().decode()?;
                let Ok(plaintext) = session.decrypt(peer, &encrypted) else {
                    continue;
                };
                let Ok(request) = serde_json::from_slice::<AppRequest>(&plaintext) else {
                    continue;
                };
                if process_request(
                    client,
                    bridge,
                    session,
                    peer,
                    state,
                    request,
                    envelope.trace_id(),
                    wallet_client,
                    descriptor,
                    dapp_name,
                    events,
                    cancellation,
                )
                .await?
                {
                    let _ = events.send(TonConnectEvent::Disconnected);
                    return Ok(());
                }
            }
        }
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(RECONNECT_DELAY_SECONDS)) => {}
            () = cancellation.cancelled() => return Ok(()),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_request(
    client: &Client,
    bridge: &HttpBridgeUrl,
    session: &SessionCrypto,
    peer: ClientId,
    state: &mut WalletSessionState,
    request: AppRequest,
    trace_id: Option<&TraceId>,
    wallet_client: &Arc<WalletClient>,
    descriptor: &WalletDescriptor,
    dapp_name: &str,
    events: &mpsc::UnboundedSender<TonConnectEvent>,
    cancellation: &CancellationToken,
) -> Result<bool> {
    let prepared = match state.prepare_request(&request) {
        Ok(prepared) => prepared,
        Err(_) => return Ok(false),
    };
    *state = prepared.into_state();
    let request_id = request.id.clone();
    let topic = request.method.clone();
    let (response, disconnected) = match request.decode() {
        Ok(KnownAppRequest::Disconnect(_)) => (
            WalletResponse::Success(WalletResponseSuccess {
                result: WalletResult::Object(Map::new()),
                id: request_id,
            }),
            true,
        ),
        Ok(KnownAppRequest::SendTransaction(request)) => (
            handle_send_transaction(
                session.client_id(),
                &request,
                wallet_client,
                descriptor,
                dapp_name,
                events,
                cancellation,
            )
            .await,
            false,
        ),
        Ok(KnownAppRequest::SignMessage(_) | KnownAppRequest::SignData(_))
        | Err(RpcError::UnsupportedMethod(_)) => (
            rpc_error(
                request_id,
                RpcErrorCode::MethodNotSupported,
                "Method is not supported",
            ),
            false,
        ),
        Err(RpcError::InvalidParameterCount { .. } | RpcError::InvalidPayload(_)) => (
            rpc_error(request_id, RpcErrorCode::BadRequest, "Malformed request"),
            false,
        ),
    };
    send_encrypted(
        client,
        bridge,
        session,
        peer,
        &response,
        Some(&topic),
        trace_id,
    )
    .await?;
    Ok(disconnected)
}

#[allow(clippy::too_many_arguments)]
async fn handle_send_transaction(
    session_id: ClientId,
    request: &SendTransactionRequest,
    wallet_client: &Arc<WalletClient>,
    descriptor: &WalletDescriptor,
    dapp_name: &str,
    events: &mpsc::UnboundedSender<TonConnectEvent>,
    cancellation: &CancellationToken,
) -> WalletResponse {
    let request_id = request.id.clone();
    let engine_request = match engine_send_request(session_id, request, descriptor) {
        Ok(request) => request,
        Err(code) => return rpc_error(request_id, code, "Unsupported transaction shape"),
    };
    let (approval_tx, approval_rx) = oneshot::channel();
    let SendAmount::Exact { nanograms } = &engine_request.amount else {
        return rpc_error(request_id, RpcErrorCode::BadRequest, "Invalid amount");
    };
    if events
        .send(TonConnectEvent::TransactionPrompt(TransactionPrompt {
            dapp_name: dapp_name.to_owned(),
            destination: engine_request.destination.to_string(),
            amount_nanograms: nanograms.to_string(),
            deploys_contract: engine_request.state_init.is_some(),
            has_payload: engine_request.payload.is_some(),
            response: Some(approval_tx),
        }))
        .is_err()
    {
        return rpc_error(request_id, RpcErrorCode::Unknown, "TUI closed");
    }
    let approved = tokio::select! {
        result = approval_rx => result.unwrap_or(false),
        () = cancellation.cancelled() => false,
    };
    if !approved {
        return rpc_error(
            request_id,
            RpcErrorCode::UserDeclined,
            "User declined the transaction",
        );
    }
    match wallet_client.send(engine_request).await {
        Ok(result)
            if matches!(
                result.phase,
                SendPhase::Submitted | SendPhase::SubmissionUnknown | SendPhase::Confirmed
            ) =>
        {
            let _ = events.send(TonConnectEvent::TransactionFinished(format!(
                "Transaction finished: {:?}",
                result.phase
            )));
            WalletResponse::Success(WalletResponseSuccess {
                result: WalletResult::String(String::from(result.signed_boc)),
                id: request_id,
            })
        }
        Ok(result) => {
            let _ = events.send(TonConnectEvent::TransactionFinished(format!(
                "Transaction was not submitted: {:?}",
                result.phase
            )));
            rpc_error(
                request_id,
                RpcErrorCode::Unknown,
                "Transaction was not submitted",
            )
        }
        Err(error) => {
            let _ = events.send(TonConnectEvent::TransactionFinished(error.to_string()));
            rpc_error(
                request_id,
                RpcErrorCode::Unknown,
                "Transaction submission failed",
            )
        }
    }
}

fn engine_send_request(
    session_id: ClientId,
    request: &SendTransactionRequest,
    descriptor: &WalletDescriptor,
) -> Result<SendRequest, RpcErrorCode> {
    let TransactionPayload::Raw(payload) = &request.payload else {
        return Err(RpcErrorCode::MethodNotSupported);
    };
    validate_transaction_sender(payload, descriptor)?;
    let messages = payload.messages.as_slice();
    let Some(message) = messages.first() else {
        return Err(RpcErrorCode::BadRequest);
    };
    if messages.len() != 1 || message.extra_currency.is_some() {
        return Err(RpcErrorCode::MethodNotSupported);
    }
    let body = decode_boc(message.payload.as_ref())?;
    let state_init = decode_boc(message.state_init.as_ref())?;
    Ok(SendRequest {
        operation_id: NonEmptyString::try_from(format!(
            "ton-connect-{}-{}",
            session_id, request.id
        ))
        .map_err(|_| RpcErrorCode::BadRequest)?,
        destination: TonAddressString::try_from(message.address.as_str())
            .map_err(|_| RpcErrorCode::BadRequest)?,
        amount: SendAmount::exact(message.amount.as_str()).map_err(|_| RpcErrorCode::BadRequest)?,
        valid_until: payload.valid_until,
        payload: body,
        state_init,
        comment: None,
    })
}

fn decode_boc(value: Option<&ton_connect_core::CellBoc>) -> Result<Option<Boc>, RpcErrorCode> {
    value
        .map(|value| Boc::try_from(value.as_bytes().to_vec()).map_err(|_| RpcErrorCode::BadRequest))
        .transpose()
}

fn validate_transaction_sender(
    payload: &RawTransactionPayload,
    descriptor: &WalletDescriptor,
) -> Result<(), RpcErrorCode> {
    let expected_network = match descriptor.network {
        Network::Mainnet => "-239",
        Network::Testnet => "-3",
    };
    if payload
        .network
        .as_ref()
        .is_some_and(|network| network.as_str() != expected_network)
    {
        return Err(RpcErrorCode::BadRequest);
    }
    if let Some(from) = payload.from.as_ref() {
        let from =
            TonAddressString::try_from(from.to_string()).map_err(|_| RpcErrorCode::BadRequest)?;
        if from != descriptor.address {
            return Err(RpcErrorCode::BadRequest);
        }
    }
    Ok(())
}

fn rpc_error(id: String, code: RpcErrorCode, message: &str) -> WalletResponse {
    WalletResponse::Error {
        error: WalletResponseError {
            code,
            message: message.to_owned(),
            data: None,
        },
        id,
    }
}

async fn send_encrypted<T: Serialize>(
    client: &Client,
    bridge: &HttpBridgeUrl,
    session: &SessionCrypto,
    peer: ClientId,
    message: &T,
    topic: Option<&str>,
    trace_id: Option<&TraceId>,
) -> Result<()> {
    let plaintext = serde_json::to_vec(message)?;
    let encoded = STANDARD.encode(session.encrypt(peer, &plaintext)?);
    let ttl = NonZeroU32::new(BRIDGE_TTL_SECONDS).ok_or_else(|| anyhow!("invalid bridge TTL"))?;
    let endpoint = bridge.message_endpoint(session.client_id(), peer, ttl, topic, trace_id);
    client
        .post(endpoint)
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(encoded)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn send_connect_error(
    client: &Client,
    bridge: &HttpBridgeUrl,
    session: &SessionCrypto,
    peer: ClientId,
    trace_id: Option<&TraceId>,
    code: ConnectEventErrorCode,
    message: &str,
) -> Result<()> {
    let transition =
        WalletSessionState::pending_connect().prepare_event(WalletEventKind::ConnectError)?;
    let event = ConnectEvent::ConnectError {
        id: transition.id(),
        payload: ConnectEventError {
            code,
            message: message.to_owned(),
        },
    };
    send_encrypted(client, bridge, session, peer, &event, None, trace_id).await
}

fn unix_timestamp() -> Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .context("system clock is before Unix epoch")
}

const fn current_platform() -> DevicePlatform {
    #[cfg(target_os = "macos")]
    {
        DevicePlatform::Mac
    }
    #[cfg(target_os = "windows")]
    {
        DevicePlatform::Windows
    }
    #[cfg(target_os = "linux")]
    {
        DevicePlatform::Linux
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        DevicePlatform::Browser
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ton_connect_core::{
        AccountAddress, CellBoc, DecimalString, FriendlyAddress, NonEmptyVec, RawMessage,
    };

    const EMPTY_CELL_BOC: &str = "te6ccgEBAQEAAgAAAA==";

    #[test]
    fn send_mapping_preserves_contract_payload_state_init_and_validity() -> Result<()> {
        let descriptor = WalletDescriptor {
            record_id: "test-wallet".to_owned(),
            address: TonAddressString::try_from(
                "0:1111111111111111111111111111111111111111111111111111111111111111",
            )?,
            public_key: vec![0_u8; 32],
            network: Network::Testnet,
            secret_ref: wallet_engine::ProtectedSecretRef {
                value: "wallet:test-wallet:mnemonic".to_owned(),
            },
        };
        let destination =
            FriendlyAddress::try_from("Ef8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADAU")?;
        let message = RawMessage {
            address: destination.clone(),
            amount: DecimalString::try_from("1000000")?,
            payload: Some(CellBoc::try_from(EMPTY_CELL_BOC)?),
            state_init: Some(CellBoc::try_from(EMPTY_CELL_BOC)?),
            extra_currency: None,
        };
        let request = SendTransactionRequest {
            id: "1".to_owned(),
            payload: TransactionPayload::Raw(RawTransactionPayload {
                valid_until: Some(1_900_000_000),
                network: Some(NetworkId::try_from("-3")?),
                from: Some(AccountAddress::try_from(descriptor.address.to_string())?),
                messages: NonEmptyVec::try_from(vec![message])?,
            }),
        };

        let converted =
            engine_send_request(ClientId::from_bytes([7_u8; 32]), &request, &descriptor)
                .map_err(|code| anyhow!("unexpected RPC error: {code:?}"))?;

        assert_eq!(converted.destination.as_str(), destination.as_str());
        assert_eq!(converted.valid_until, Some(1_900_000_000));
        assert!(converted.payload.is_some());
        assert!(converted.state_init.is_some());
        Ok(())
    }

    #[test]
    fn send_mapping_rejects_cross_network_requests() -> Result<()> {
        let descriptor = WalletDescriptor {
            record_id: "test-wallet".to_owned(),
            address: TonAddressString::try_from(
                "0:1111111111111111111111111111111111111111111111111111111111111111",
            )?,
            public_key: vec![0_u8; 32],
            network: Network::Testnet,
            secret_ref: wallet_engine::ProtectedSecretRef {
                value: "wallet:test-wallet:mnemonic".to_owned(),
            },
        };
        let payload = RawTransactionPayload {
            valid_until: None,
            network: Some(NetworkId::try_from("-239")?),
            from: None,
            messages: NonEmptyVec::try_from(vec![RawMessage {
                address: FriendlyAddress::try_from(
                    "Ef8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADAU",
                )?,
                amount: DecimalString::try_from("1")?,
                payload: None,
                state_init: None,
                extra_currency: None,
            }])?,
        };

        assert_eq!(
            validate_transaction_sender(&payload, &descriptor),
            Err(RpcErrorCode::BadRequest)
        );
        Ok(())
    }
}
