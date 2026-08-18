use std::num::{NonZeroU32, NonZeroUsize};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde_json::Map;
use ton_connect_client::{
    PersistedTonConnectClient, TonConnectClient, TonConnectClientConfig, TonConnectClientError,
};
use ton_connect_core::{
    AppRequest, ConnectEvent, ConnectEventErrorCode, ConnectEventPayload, ConnectItem,
    ConnectItemReply, ConnectLink, ConnectRequest, DeviceInfo, DevicePlatform, Ed25519PublicKey,
    Feature, HeartbeatMode, HttpBridgeUrl, HttpsUrl, NetworkId, NonEmptyVec, ReturnStrategy,
    SendTransactionFeature, SessionCrypto, TonAddressItem, TonAddressItemReply, TraceId,
    WalletResponse, WalletResponseSuccess, WalletResult, WalletSessionPhase, WalletStateInit,
    decode_embedded_request_param,
};
use ton_core::{cell::TonCell, traits::tlb::TLB as _};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn config() -> Result<TonConnectClientConfig, Box<dyn std::error::Error>> {
    Ok(TonConnectClientConfig::new(
        HttpBridgeUrl::try_from("https://bridge.example/bridge")?,
        NonZeroUsize::new(4096).ok_or("event limit")?,
        NonZeroU32::new(300).ok_or("message TTL")?,
        HeartbeatMode::Message,
    ))
}

fn connect_request() -> Result<ConnectRequest, Box<dyn std::error::Error>> {
    Ok(ConnectRequest {
        manifest_url: HttpsUrl::try_from("https://app.example/manifest.json")?,
        items: NonEmptyVec::try_from(vec![ConnectItem::from(TonAddressItem {
            network: Some(NetworkId::try_from("-3")?),
        })])?,
    })
}

fn account() -> Result<TonAddressItemReply, Box<dyn std::error::Error>> {
    let mut state = TonCell::builder();
    state.write_bit(false)?;
    state.write_bit(false)?;
    state.write_bit(true)?;
    state.write_ref(TonCell::empty().to_owned())?;
    state.write_bit(true)?;
    state.write_ref(TonCell::empty().to_owned())?;
    state.write_bit(false)?;
    let state = WalletStateInit::from_boc(state.build()?.to_boc()?)?;
    let address = state.derive_address(0)?;
    Ok(TonAddressItemReply::new(
        address,
        NetworkId::try_from("-3")?,
        state,
        Ed25519PublicKey::from_bytes([0_u8; 32]),
    ))
}

fn device() -> Result<DeviceInfo, Box<dyn std::error::Error>> {
    Ok(DeviceInfo::new(
        DevicePlatform::Linux,
        "example-wallet".to_owned(),
        "1.0.0".to_owned(),
        2,
        vec![Feature::SendTransaction(SendTransactionFeature::new(
            1,
            Some(false),
            None,
        )?)],
    )?)
}

#[test]
fn rejected_connect_is_encrypted_and_persistently_terminal() -> TestResult {
    let dapp = SessionCrypto::generate()?;
    let link = ConnectLink::connect(
        dapp.client_id(),
        connect_request()?,
        ReturnStrategy::None,
        Some(decode_embedded_request_param(
            &URL_SAFE_NO_PAD.encode(r#"{"m":"sd","t":"text","tx":"authorize"}"#),
        )?),
        None,
    );
    let link = link.to_url("tc://")?;
    let config = config()?;
    assert_eq!(
        config.bridge_url().as_str(),
        "https://bridge.example/bridge"
    );
    let mut client = TonConnectClient::from_link(link.as_str(), config)?;
    assert_eq!(client.peer_client_id(), dapp.client_id());
    assert!(client.embedded_request().is_some());
    let pending = serde_json::from_str::<PersistedTonConnectClient>(&serde_json::to_string(
        &client.persisted()?,
    )?)?;
    let original_client_id = client.client_id();
    client = TonConnectClient::restore(
        &pending,
        NonZeroUsize::new(4096).ok_or("event limit")?,
        NonZeroU32::new(300).ok_or("message TTL")?,
        HeartbeatMode::Message,
    )?;
    assert_eq!(client.client_id(), original_client_id);
    assert!(client.connect_request().is_some());
    assert!(client.embedded_request().is_some());
    let post = client.reject_connect(
        ConnectEventErrorCode::UserDeclined,
        "User declined".to_owned(),
    )?;

    let encrypted = post.body().decode()?;
    let plaintext = dapp.decrypt(client.client_id(), &encrypted)?;
    let event = serde_json::from_slice::<ConnectEvent>(&plaintext)?;
    assert!(matches!(
        event,
        ConnectEvent::ConnectError { payload, .. }
            if payload.code == ConnectEventErrorCode::UserDeclined
    ));
    assert_eq!(client.phase(), WalletSessionPhase::Disconnected);

    let persisted = client.persisted()?;
    let persisted =
        serde_json::from_str::<PersistedTonConnectClient>(&serde_json::to_string(&persisted)?)?;
    let restored = TonConnectClient::restore(
        &persisted,
        NonZeroUsize::new(4096).ok_or("event limit")?,
        NonZeroU32::new(300).ok_or("message TTL")?,
        HeartbeatMode::Message,
    )?;
    assert_eq!(restored.client_id(), client.client_id());
    assert_eq!(restored.phase(), WalletSessionPhase::Disconnected);
    Ok(())
}

#[test]
fn authenticated_requests_are_replay_safe_and_responses_reuse_the_trace() -> TestResult {
    let dapp = SessionCrypto::generate()?;
    let link = ConnectLink::connect(
        dapp.client_id(),
        connect_request()?,
        ReturnStrategy::Back,
        None,
        Some(TraceId::try_from("018f4f84-7b8d-7c3f-8d8e-123456789abc")?),
    );
    let mut client = TonConnectClient::from_parsed_link(&link, config()?)?;
    let account = account()?;
    let address = account.address;
    let connect_post = client.approve_connect(
        ConnectEventPayload {
            items: vec![ConnectItemReply::TonAddress(account)],
            device: device()?,
        },
        None,
    )?;
    assert!(connect_post.url().as_str().contains("ttl=300"));
    assert!(connect_post.url().as_str().contains("trace_id="));
    assert_eq!(client.connected_address(), Some(address));

    let subscription = client.begin_events_subscription();
    assert!(subscription.as_str().contains("heartbeat=message"));
    assert!(!subscription.as_str().contains("trace_id="));
    let attacker = SessionCrypto::generate()?;
    let attacker_plaintext = serde_json::to_vec(&AppRequest {
        method: "disconnect".to_owned(),
        params: Vec::new(),
        id: "1".to_owned(),
    })?;
    let attacker_envelope = serde_json::json!({
        "from": attacker.client_id().to_string(),
        "message": STANDARD.encode(attacker.encrypt(client.client_id(), &attacker_plaintext)?),
    });
    let attacker_sse = format!("id: 6\ndata: {attacker_envelope}\n\n");
    assert!(client.ingest_sse_chunk(attacker_sse.as_bytes())?.is_empty());
    assert_eq!(client.last_bridge_event_id(), Some("6"));

    let request = AppRequest {
        method: "disconnect".to_owned(),
        params: Vec::new(),
        id: "1".to_owned(),
    };
    let plaintext = serde_json::to_vec(&request)?;
    let ciphertext = STANDARD.encode(dapp.encrypt(client.client_id(), &plaintext)?);
    let envelope = serde_json::json!({
        "from": dapp.client_id().to_string(),
        "message": ciphertext,
        "trace_id": "018f4f84-7b8d-7c3f-8d8e-123456789abc",
    });
    let sse = format!("id: 7\ndata: {envelope}\n\n");
    let incoming = client.ingest_sse_chunk(sse.as_bytes())?;
    let incoming = incoming.first().ok_or("fresh request")?;
    assert_eq!(incoming.request(), &request);
    assert!(incoming.closes_session());
    assert_eq!(client.phase(), WalletSessionPhase::Disconnected);
    assert_eq!(client.last_bridge_event_id(), Some("7"));

    let response = WalletResponse::Success(WalletResponseSuccess {
        result: WalletResult::Object(Map::new()),
        id: "1".to_owned(),
    });
    let wrong_response = WalletResponse::Success(WalletResponseSuccess {
        result: WalletResult::Object(Map::new()),
        id: "2".to_owned(),
    });
    assert!(matches!(
        client.prepare_response(incoming, &wrong_response),
        Err(TonConnectClientError::ResponseIdMismatch)
    ));
    let response_post = client.prepare_response(incoming, &response)?;
    assert!(response_post.url().as_str().contains("topic=disconnect"));
    assert!(response_post.url().as_str().contains("trace_id="));
    let encrypted = response_post.body().decode()?;
    let plaintext = dapp.decrypt(client.client_id(), &encrypted)?;
    assert_eq!(
        serde_json::from_slice::<WalletResponse>(&plaintext)?,
        response
    );

    let replay = client.ingest_sse_chunk(sse.replace("id: 7", "id: 8").as_bytes())?;
    assert!(replay.is_empty());
    assert_eq!(client.last_bridge_event_id(), Some("8"));

    let persisted = client.persisted()?;
    let persisted =
        serde_json::from_str::<PersistedTonConnectClient>(&serde_json::to_string(&persisted)?)?;
    let mut restored = TonConnectClient::restore(
        &persisted,
        NonZeroUsize::new(4096).ok_or("event limit")?,
        NonZeroU32::new(300).ok_or("message TTL")?,
        HeartbeatMode::Message,
    )?;
    assert_eq!(restored.connected_address(), Some(address));
    assert!(
        restored
            .begin_events_subscription()
            .as_str()
            .contains("last_event_id=8")
    );
    Ok(())
}

#[test]
fn wallet_initiated_disconnect_is_encrypted_and_terminal() -> TestResult {
    let dapp = SessionCrypto::generate()?;
    let link = ConnectLink::connect(
        dapp.client_id(),
        connect_request()?,
        ReturnStrategy::None,
        None,
        None,
    );
    let mut client = TonConnectClient::from_parsed_link(&link, config()?)?;
    let account = account()?;
    let _ = client.approve_connect(
        ConnectEventPayload {
            items: vec![ConnectItemReply::TonAddress(account)],
            device: device()?,
        },
        None,
    )?;
    let trace_id = TraceId::try_from("018f4f84-7b8d-7c3f-8d8e-123456789abc")?;
    let post = client.disconnect(Some(&trace_id))?;
    assert!(post.url().as_str().contains("trace_id="));
    let encrypted = post.body().decode()?;
    let plaintext = dapp.decrypt(client.client_id(), &encrypted)?;
    assert!(matches!(
        serde_json::from_slice::<ConnectEvent>(&plaintext)?,
        ConnectEvent::Disconnect { .. }
    ));
    assert_eq!(client.phase(), WalletSessionPhase::Disconnected);
    assert!(client.disconnect(None).is_err());
    Ok(())
}
