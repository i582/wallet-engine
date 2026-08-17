//! Minimal real TON Connect wallet over the common HTTP bridge.
//!
//! This example intentionally supports only `ton_addr`, `ton_proof`, and the
//! dApp-initiated `disconnect` RPC. It creates an ephemeral V5R1 test account,
//! fetches and displays the dApp manifest, asks for terminal confirmation, and
//! never prints or persists the generated signing key.

use std::{
    error::Error,
    io::{self, Read as _},
    num::{NonZeroU32, NonZeroUsize},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use clap::{Parser, ValueEnum};
use ed25519_dalek::{Signer as _, SigningKey};
use reqwest::blocking::{Client, Response};
use serde::Serialize;
use serde_json::Map;
use thiserror::Error;
use ton_connect_core::{
    AppManifest, AppRequest, BridgeSseDecoder, ClientId, ConnectEvent, ConnectEventError,
    ConnectEventErrorCode, ConnectEventPayload, ConnectItem, ConnectItemReply, ConnectLink,
    DeviceInfo, DevicePlatform, Ed25519PublicKey, Ed25519Signature, HttpBridgeUrl, KnownAppRequest,
    NetworkId, RawAccountAddress, RpcError, RpcErrorCode, SessionCrypto, TonAddressItemReply,
    TonProof, TonProofDomain, TonProofItemReply, TraceId, Uint64String, WalletEventKind,
    WalletResponse, WalletResponseError, WalletResponseSuccess, WalletResult, WalletSessionState,
    WalletStateInit,
};
use ton_core::{cell::TonCell, traits::tlb::TLB as _};

const DEFAULT_BRIDGE_URL: &str = "https://connect.ton.org/bridge";
const MANIFEST_LIMIT_BYTES: usize = 1_048_576;
const SSE_EVENT_LIMIT_BYTES: usize = 1_048_576;
const BRIDGE_TTL_SECONDS: u32 = 300;
const HTTP_TIMEOUT_SECONDS: u64 = 15;
const RECONNECT_DELAY_SECONDS: u64 = 1;
const WALLET_V5R1_ID_MAINNET: i32 = 0x7fff_ff11;
const WALLET_V5R1_ID_TESTNET: i32 = 0x7fff_fffd;
// The demo link is generated from Tonkeeper's wallet source, so the dApp SDK
// resolves this event through Tonkeeper's wallets-list entry. A distributable
// wallet must use its own registered app_name instead of impersonating it.
const DEMO_WALLET_APP_NAME: &str = "tonkeeper";
const WALLET_V5R1_CODE: &str = include_str!("../src/testdata/wallet_v5.code");

type DemoResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum DemoNetwork {
    Mainnet,
    Testnet,
}

impl DemoNetwork {
    fn network_id(self) -> Result<NetworkId, ton_connect_core::ValueError> {
        NetworkId::try_from(match self {
            Self::Mainnet => "-239",
            Self::Testnet => "-3",
        })
    }

    const fn wallet_id(self) -> i32 {
        match self {
            Self::Mainnet => WALLET_V5R1_ID_MAINNET,
            Self::Testnet => WALLET_V5R1_ID_TESTNET,
        }
    }
}

/// Connect an ephemeral V5R1 account to a dApp through a TON Connect bridge.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Options {
    /// Full TON Connect v2 link copied from a listening dApp.
    #[arg(value_name = "CONNECT_LINK")]
    connect_link: String,

    /// HTTP bridge base used by the dApp when it generated the link.
    #[arg(long = "bridge", default_value = DEFAULT_BRIDGE_URL, value_parser = parse_bridge_url)]
    bridge_url: HttpBridgeUrl,

    /// Network exposed by the ephemeral account.
    #[arg(long, value_enum, default_value_t = DemoNetwork::Testnet)]
    network: DemoNetwork,

    /// Skip the interactive approval prompt; intended only for experiments.
    #[arg(long = "yes")]
    approve_without_prompt: bool,
}

#[derive(Debug, Error)]
enum DemoError {
    #[error("connect link does not contain a full connect request")]
    ReducedLink,
    #[error("connect request does not contain the mandatory ton_addr item")]
    MissingTonAddress,
    #[error("dApp requested a different network from this demo wallet")]
    NetworkMismatch,
    #[error("bridge request failed with HTTP status {0}")]
    BridgeHttp(reqwest::StatusCode),
    #[error("system clock is before the Unix epoch")]
    InvalidClock,
    #[error("bridge SSE stream closed; reconnecting")]
    StreamClosed,
    #[error("invalid non-zero demo transport constant")]
    InvalidTransportConstant,
}

#[derive(Debug, Error)]
enum ManifestLoadError {
    #[error("manifest request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("manifest request failed with HTTP status {0}")]
    Http(reqwest::StatusCode),
    #[error("manifest response exceeds one MiB")]
    TooLarge,
    #[error("manifest content is invalid: {0}")]
    InvalidContent(#[from] serde_json::Error),
    #[error("manifest response size cannot be represented by this platform")]
    SizeOverflow,
    #[error("manifest response could not be read: {0}")]
    Read(#[from] io::Error),
}

impl ManifestLoadError {
    const fn connect_error_code(&self) -> ConnectEventErrorCode {
        match self {
            Self::Request(_) | Self::Http(_) | Self::Read(_) => {
                ConnectEventErrorCode::ManifestNotFound
            }
            Self::TooLarge | Self::InvalidContent(_) | Self::SizeOverflow => {
                ConnectEventErrorCode::ManifestContent
            }
        }
    }
}

fn main() -> DemoResult<()> {
    let options = Options::parse();
    let link = ConnectLink::parse(&options.connect_link)?;
    let request = link.request().ok_or(DemoError::ReducedLink)?;
    let session = SessionCrypto::generate()?;
    let network = options.network.network_id()?;

    let http = Client::builder()
        .connect_timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .user_agent(concat!("ton-connect-core-demo/", env!("CARGO_PKG_VERSION")))
        .build()?;
    if let Err(error) = enforce_connect_network(request.items.as_slice(), &network) {
        send_connect_error(
            &http,
            &options.bridge_url,
            &session,
            link.client_id(),
            link.trace_id(),
            ConnectEventErrorCode::BadRequest,
            "Connect request does not match the demo wallet",
        )?;
        return Err(error.into());
    }
    let manifest = match load_manifest(&http, request.manifest_url.as_str()) {
        Ok(manifest) => manifest,
        Err(error) => {
            send_connect_error(
                &http,
                &options.bridge_url,
                &session,
                link.client_id(),
                link.trace_id(),
                error.connect_error_code(),
                "Failed to load dApp manifest",
            )?;
            return Err(error.into());
        }
    };
    let domain = manifest.app_domain()?;

    let signing_key = generate_signing_key()?;
    let (state_init, address) = wallet_state(
        signing_key.verifying_key().to_bytes(),
        options.network.wallet_id(),
    )?;
    let account = TonAddressItemReply::new(
        address,
        network,
        state_init,
        Ed25519PublicKey::from_bytes(signing_key.verifying_key().to_bytes()),
    );

    show_confirmation(&manifest, &domain, &account, request.items.as_slice());
    if !options.approve_without_prompt && !confirmed()? {
        send_connect_error(
            &http,
            &options.bridge_url,
            &session,
            link.client_id(),
            link.trace_id(),
            ConnectEventErrorCode::UserDeclined,
            "User declined the connection",
        )?;
        return Ok(());
    }

    let transition =
        WalletSessionState::pending_connect().prepare_event(WalletEventKind::Connect)?;
    let connect_event = connect_event(
        transition.id(),
        request.items.as_slice(),
        &account,
        &signing_key,
        &domain,
    )?;
    let mut state = transition.into_state();

    send_encrypted(
        &http,
        &options.bridge_url,
        &session,
        link.client_id(),
        &connect_event,
        None,
        link.trace_id(),
    )?;
    println!(
        "Connected. Wallet bridge client_id: {}",
        session.client_id()
    );
    println!("Waiting for dApp requests; Ctrl-C stops the ephemeral session.");

    listen(
        &http,
        &options.bridge_url,
        &session,
        link.client_id(),
        &mut state,
    )
}

fn parse_bridge_url(value: &str) -> Result<HttpBridgeUrl, String> {
    HttpBridgeUrl::try_from(value).map_err(|error| error.to_string())
}

fn load_manifest(client: &Client, url: &str) -> Result<AppManifest, ManifestLoadError> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .send()?;
    if !response.status().is_success() {
        return Err(ManifestLoadError::Http(response.status()));
    }

    let byte_limit = u64::try_from(MANIFEST_LIMIT_BYTES)
        .map_err(|_| ManifestLoadError::SizeOverflow)?
        .saturating_add(1);
    let mut body = Vec::new();
    let _ = response.take(byte_limit).read_to_end(&mut body)?;
    if body.len() > MANIFEST_LIMIT_BYTES {
        return Err(ManifestLoadError::TooLarge);
    }
    Ok(serde_json::from_slice(&body)?)
}

fn enforce_connect_network(items: &[ConnectItem], active: &NetworkId) -> Result<(), DemoError> {
    let mut has_address = false;
    for item in items {
        if let ConnectItem::TonAddr { network } = item {
            has_address = true;
            if network
                .as_ref()
                .is_some_and(|requested| requested != active)
            {
                return Err(DemoError::NetworkMismatch);
            }
        }
    }
    if has_address {
        Ok(())
    } else {
        Err(DemoError::MissingTonAddress)
    }
}

fn generate_signing_key() -> DemoResult<SigningKey> {
    let mut seed = [0_u8; 32];
    getrandom::fill(&mut seed)?;
    Ok(SigningKey::from_bytes(&seed))
}

fn wallet_state(
    public_key: [u8; 32],
    wallet_id: i32,
) -> DemoResult<(WalletStateInit, RawAccountAddress)> {
    let code = TonCell::from_boc_base64(WALLET_V5R1_CODE.trim())?;
    let mut data = TonCell::builder();
    data.write_bit(true)?;
    data.write_num(&0_u32, 32)?;
    data.write_num(&wallet_id, 32)?;
    data.write_bits(public_key, 256)?;
    data.write_bit(false)?;

    let mut root = TonCell::builder();
    root.write_bit(false)?;
    root.write_bit(false)?;
    root.write_bit(true)?;
    root.write_ref(code)?;
    root.write_bit(true)?;
    root.write_ref(data.build()?)?;
    root.write_bit(false)?;

    let state_init = WalletStateInit::from_boc(root.build()?.to_boc()?)?;
    let address = state_init.derive_address(0)?;
    Ok((state_init, address))
}

fn show_confirmation(
    manifest: &AppManifest,
    domain: &str,
    account: &TonAddressItemReply,
    items: &[ConnectItem],
) {
    println!("dApp: {}", manifest.name());
    println!("Origin: {}", manifest.url());
    println!("Icon: {}", manifest.icon_url());
    println!("Proof domain: {domain}");
    println!("Ephemeral account: {}", account.address);
    if let Some(payload) = items.iter().find_map(|item| match item {
        ConnectItem::TonProof { payload } => Some(payload.as_str()),
        ConnectItem::TonAddr { .. } => None,
    }) {
        println!("Off-chain login payload: {payload}");
    }
    println!("No transaction or network fee is involved in this connection.");
}

fn confirmed() -> DemoResult<bool> {
    println!("Approve this dApp connection? Type 'yes' to continue:");
    let mut answer = String::new();
    let _ = io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case("yes"))
}

fn connect_event(
    id: u64,
    requested: &[ConnectItem],
    account: &TonAddressItemReply,
    signing_key: &SigningKey,
    domain: &str,
) -> DemoResult<ConnectEvent> {
    let mut items = Vec::new();
    for item in requested {
        match item {
            ConnectItem::TonAddr { .. } => {
                items.push(ConnectItemReply::TonAddress(account.clone()));
            }
            ConnectItem::TonProof { payload } => {
                let timestamp = unix_timestamp()?;
                let mut proof = TonProof {
                    timestamp: Uint64String::from(timestamp),
                    domain: TonProofDomain::new(domain.to_owned())?,
                    payload: payload.clone(),
                    signature: Ed25519Signature::from_bytes([0_u8; 64]),
                };
                proof.signature = Ed25519Signature::from_bytes(
                    signing_key
                        .sign(&proof.signing_hash(&account.address)?)
                        .to_bytes(),
                );
                items.push(ConnectItemReply::TonProof(TonProofItemReply::new(proof)));
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
                features: Vec::new(),
            },
        },
        response: None,
    })
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

fn unix_timestamp() -> Result<u64, DemoError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| DemoError::InvalidClock)
}

fn send_encrypted<T: Serialize>(
    client: &Client,
    bridge: &HttpBridgeUrl,
    session: &SessionCrypto,
    peer: ClientId,
    message: &T,
    topic: Option<&str>,
    trace_id: Option<&TraceId>,
) -> DemoResult<()> {
    let plaintext = serde_json::to_vec(message)?;
    let encrypted = session.encrypt(peer, &plaintext)?;
    let encoded = STANDARD.encode(encrypted);
    let ttl = NonZeroU32::new(BRIDGE_TTL_SECONDS).ok_or(DemoError::InvalidTransportConstant)?;
    let endpoint = bridge.message_endpoint(session.client_id(), peer, ttl, topic, trace_id);
    let response = client
        .post(endpoint)
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECONDS))
        .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(encoded)
        .send()?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(DemoError::BridgeHttp(response.status()).into())
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "connect error transport fields are protocol data"
)]
fn send_connect_error(
    client: &Client,
    bridge: &HttpBridgeUrl,
    session: &SessionCrypto,
    peer: ClientId,
    trace_id: Option<&TraceId>,
    code: ConnectEventErrorCode,
    message: &str,
) -> DemoResult<()> {
    let transition =
        WalletSessionState::pending_connect().prepare_event(WalletEventKind::ConnectError)?;
    let event = ConnectEvent::ConnectError {
        id: transition.id(),
        payload: ConnectEventError {
            code,
            message: message.to_owned(),
        },
    };
    send_encrypted(client, bridge, session, peer, &event, None, trace_id)
}

fn listen(
    client: &Client,
    bridge: &HttpBridgeUrl,
    session: &SessionCrypto,
    peer: ClientId,
    state: &mut WalletSessionState,
) -> DemoResult<()> {
    let event_limit =
        NonZeroUsize::new(SSE_EVENT_LIMIT_BYTES).ok_or(DemoError::InvalidTransportConstant)?;
    let mut decoder = BridgeSseDecoder::new(event_limit);
    let mut last_event_id = None::<String>;

    loop {
        let endpoint = bridge.events_endpoint(session.client_id(), last_event_id.as_deref(), None);
        match client.get(endpoint).send() {
            Ok(response) if response.status().is_success() => {
                if read_event_stream(
                    client,
                    bridge,
                    session,
                    peer,
                    state,
                    &mut decoder,
                    &mut last_event_id,
                    response,
                )? {
                    return Ok(());
                }
            }
            Ok(response) => return Err(DemoError::BridgeHttp(response.status()).into()),
            Err(error) => eprintln!("Bridge connection failed: {error}; reconnecting"),
        }
        thread::sleep(Duration::from_secs(RECONNECT_DELAY_SECONDS));
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "keeps one SSE connection's mutable state explicit"
)]
fn read_event_stream(
    client: &Client,
    bridge: &HttpBridgeUrl,
    session: &SessionCrypto,
    peer: ClientId,
    state: &mut WalletSessionState,
    decoder: &mut BridgeSseDecoder,
    last_event_id: &mut Option<String>,
    mut response: Response,
) -> DemoResult<bool> {
    let mut buffer = [0_u8; 8192];
    loop {
        let count = match response.read(&mut buffer) {
            Ok(0) => {
                eprintln!("{}", DemoError::StreamClosed);
                return Ok(false);
            }
            Ok(count) => count,
            Err(error) => {
                eprintln!("Bridge stream read failed: {error}; reconnecting");
                return Ok(false);
            }
        };
        let chunk = buffer.get(..count).ok_or(DemoError::StreamClosed)?;
        for event in decoder.push(chunk)? {
            if let Some(event_id) = event.event_id() {
                *last_event_id = Some(event_id.to_owned());
            }
            let envelope = event.into_message();
            if envelope.from() != peer {
                continue;
            }
            let encrypted = envelope.message().decode()?;
            let Ok(plaintext) = session.decrypt(peer, &encrypted) else {
                eprintln!("Ignored an unauthenticated bridge message");
                continue;
            };
            let Ok(request) = serde_json::from_slice::<AppRequest>(&plaintext) else {
                eprintln!("Ignored a malformed dApp request");
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
            )? {
                return Ok(true);
            }
        }
    }
}

fn process_request(
    client: &Client,
    bridge: &HttpBridgeUrl,
    session: &SessionCrypto,
    peer: ClientId,
    state: &mut WalletSessionState,
    request: AppRequest,
    trace_id: Option<&TraceId>,
) -> DemoResult<bool> {
    let prepared = match state.prepare_request(&request) {
        Ok(prepared) => prepared,
        Err(error) => {
            eprintln!("Ignored replayed or invalid request: {error}");
            return Ok(false);
        }
    };

    // A production host atomically persists this state before processing. The
    // ephemeral demo intentionally keeps it in memory, but preserves ordering.
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
        Ok(
            KnownAppRequest::SendTransaction(_)
            | KnownAppRequest::SignMessage(_)
            | KnownAppRequest::SignData(_),
        )
        | Err(RpcError::UnsupportedMethod(_)) => (
            rpc_error(request_id, RpcErrorCode::MethodNotSupported),
            false,
        ),
        Err(RpcError::InvalidParameterCount { .. } | RpcError::InvalidPayload(_)) => {
            (rpc_error(request_id, RpcErrorCode::BadRequest), false)
        }
    };

    send_encrypted(
        client,
        bridge,
        session,
        peer,
        &response,
        Some(&topic),
        trace_id,
    )?;
    if disconnected {
        println!("dApp disconnected; ephemeral session finished.");
    }
    Ok(disconnected)
}

fn rpc_error(id: String, code: RpcErrorCode) -> WalletResponse {
    WalletResponse::Error {
        error: WalletResponseError {
            code,
            message: match code {
                RpcErrorCode::BadRequest => "Malformed request",
                RpcErrorCode::MethodNotSupported => "Method is not supported by this demo",
                RpcErrorCode::Unknown | RpcErrorCode::UnknownApp | RpcErrorCode::UserDeclined => {
                    "Request failed"
                }
            }
            .to_owned(),
            data: None,
        },
        id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_defaults_to_the_common_testnet_bridge() -> DemoResult<()> {
        let options = Options::try_parse_from(["demo", "tc://?id=example"])?;
        assert_eq!(options.network, DemoNetwork::Testnet);
        assert_eq!(
            options
                .bridge_url
                .events_endpoint(ClientId::from_bytes([0_u8; 32]), None, None)
                .as_str(),
            concat!(
                "https://connect.ton.org/bridge/events?client_id=",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "&heartbeat=message"
            )
        );
        Ok(())
    }

    #[test]
    fn clap_rejects_unknown_networks_and_invalid_bridge_urls() {
        assert!(
            Options::try_parse_from(["demo", "tc://?id=example", "--network", "devnet"]).is_err()
        );
        assert!(
            Options::try_parse_from(["demo", "tc://?id=example", "--bridge", "file:///tmp/bridge"])
                .is_err()
        );
    }
}
