//! Cross-implementation conformance cases ported to the normative wire model.
//!
//! TypeScript SDK source revision:
//! `tonkeeper/tonconnect-sdk@beb31b373e0d9db4b7d0bfd55a1ab0d0a439b74a`.
//! SDK-only camelCase request fields are translated to the protocol RPC shape.

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde_json::{Value, json};
use ton_connect_core::{
    AppManifest, AppRequest, BridgeMessage, CellBoc, ConnectEvent, ConnectItem, ConnectItemReply,
    DeviceInfo, EmbeddedRequest, ExtraCurrencies, KnownAppRequest, RpcError, SignDataPayload,
    StructuredItem, TransactionPayload, WalletsList, decode_embedded_request_param,
    encode_embedded_request_param,
};

const RAW_ADDRESS: &str = "0:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FRIENDLY_ADDRESS: &str = "UQAzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzM-lI";
const EMPTY_CELL_BOC: &str = "te6ccgEBAQEAAgAAAA==";

#[allow(
    clippy::needless_pass_by_value,
    reason = "data-driven conformance tables transfer each owned JSON fixture once"
)]
fn decode(method: &str, payload: Value) -> Result<KnownAppRequest, RpcError> {
    AppRequest {
        method: method.to_owned(),
        params: vec![payload.to_string()],
        id: "1".to_owned(),
    }
    .decode()
}

/// Ported from every applicable `validateSendTransactionRequest` case in
/// `packages/sdk/tests/validation/schemas.test.ts`.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "keeps every upstream schema case in one auditable data table"
)]
fn typescript_send_transaction_schema_cases() {
    for from in [RAW_ADDRESS, FRIENDLY_ADDRESS] {
        let decoded = decode(
            "sendTransaction",
            json!({
                "valid_until": 1_900_000_000_u64,
                "network": "-239",
                "from": from,
                "messages": [{
                    "address": FRIENDLY_ADDRESS,
                    "amount": "1000",
                    "stateInit": EMPTY_CELL_BOC,
                    "payload": EMPTY_CELL_BOC,
                    "extra_currency": { "100": "1" }
                }]
            }),
        );
        assert!(matches!(decoded, Ok(KnownAppRequest::SendTransaction(_))));
    }

    let omitted_optional = decode(
        "sendTransaction",
        json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1" }] }),
    );
    assert!(omitted_optional.is_ok());

    let invalid = [
        ("non-object", json!(null)),
        (
            "extra top-level property",
            json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1" }], "extra": true }),
        ),
        (
            "string valid_until",
            json!({ "valid_until": "1", "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1" }] }),
        ),
        (
            "negative valid_until",
            json!({ "valid_until": -1, "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1" }] }),
        ),
        (
            "invalid network",
            json!({ "network": "abc", "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1" }] }),
        ),
        (
            "null network",
            json!({ "network": null, "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1" }] }),
        ),
        (
            "empty network",
            json!({ "network": "", "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1" }] }),
        ),
        (
            "invalid from",
            json!({ "from": "bad", "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1" }] }),
        ),
        (
            "null from",
            json!({ "from": null, "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1" }] }),
        ),
        ("missing body", json!({ "network": "-239" })),
        ("empty messages", json!({ "messages": [] })),
        ("empty items", json!({ "items": [] })),
        (
            "both body forms",
            json!({
                "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1" }],
                "items": [{ "type": "ton", "address": FRIENDLY_ADDRESS, "amount": "1" }]
            }),
        ),
        ("non-object message", json!({ "messages": ["oops"] })),
        (
            "extra message property",
            json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1", "unknown": true }] }),
        ),
        (
            "raw destination",
            json!({ "messages": [{ "address": RAW_ADDRESS, "amount": "1" }] }),
        ),
        (
            "invalid destination",
            json!({ "messages": [{ "address": "not-friendly", "amount": "1" }] }),
        ),
        (
            "nonnumeric amount",
            json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1x" }] }),
        ),
        (
            "numeric amount",
            json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": 1 }] }),
        ),
        (
            "invalid state init",
            json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1", "stateInit": "abc" }] }),
        ),
        (
            "null state init",
            json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1", "stateInit": null }] }),
        ),
        (
            "empty payload",
            json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1", "payload": "" }] }),
        ),
        (
            "invalid payload",
            json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1", "payload": "def" }] }),
        ),
        (
            "null extra currency",
            json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1", "extra_currency": null }] }),
        ),
        (
            "invalid currency id",
            json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1", "extra_currency": { "x": "1" } }] }),
        ),
        (
            "invalid currency amount",
            json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1", "extra_currency": { "100": "x" } }] }),
        ),
        (
            "unknown structured item",
            json!({ "items": [{ "type": "future" }] }),
        ),
    ];

    for (name, payload) in invalid {
        assert!(
            decode("sendTransaction", payload).is_err(),
            "accepted {name}"
        );
    }
}

/// Ported from every applicable `validateSignDataPayload` case in the same
/// TypeScript suite. Cell values use a semantic `BoC` instead of its regex-only
/// `te6ccAAA` fixture.
#[test]
fn typescript_sign_data_schema_cases() {
    for payload in [
        json!({ "type": "text", "text": "hello", "network": "-239", "from": RAW_ADDRESS }),
        json!({ "type": "text", "text": "hello", "from": FRIENDLY_ADDRESS }),
        json!({ "type": "binary", "bytes": "AA==", "network": "1", "from": RAW_ADDRESS }),
        json!({ "type": "binary", "bytes": "AA==", "from": FRIENDLY_ADDRESS }),
        json!({ "type": "cell", "schema": "v1", "cell": EMPTY_CELL_BOC, "network": "0", "from": RAW_ADDRESS }),
        json!({ "type": "cell", "schema": "v1", "cell": EMPTY_CELL_BOC, "from": FRIENDLY_ADDRESS }),
        json!({ "type": "text", "text": "hello" }),
    ] {
        assert!(matches!(
            decode("signData", payload),
            Ok(KnownAppRequest::SignData(_))
        ));
    }

    let invalid = [
        ("non-object", json!(null)),
        ("unknown type", json!({ "type": "oops", "any": "x" })),
        ("missing text", json!({ "type": "text" })),
        (
            "extra text field",
            json!({ "type": "text", "text": "x", "extra": true }),
        ),
        (
            "invalid text network",
            json!({ "type": "text", "text": "x", "network": "abc" }),
        ),
        (
            "null text network",
            json!({ "type": "text", "text": "x", "network": null }),
        ),
        (
            "empty text network",
            json!({ "type": "text", "text": "x", "network": "" }),
        ),
        (
            "invalid text from",
            json!({ "type": "text", "text": "x", "from": 1 }),
        ),
        (
            "null text from",
            json!({ "type": "text", "text": "x", "from": null }),
        ),
        (
            "invalid binary",
            json!({ "type": "binary", "bytes": "not-base64" }),
        ),
        ("missing binary", json!({ "type": "binary" })),
        (
            "invalid cell",
            json!({ "type": "cell", "schema": "v1", "cell": "dGVzdA==" }),
        ),
        (
            "missing schema",
            json!({ "type": "cell", "cell": EMPTY_CELL_BOC }),
        ),
        ("missing cell", json!({ "type": "cell", "schema": "v1" })),
    ];
    for (name, payload) in invalid {
        assert!(decode("signData", payload).is_err(), "accepted {name}");
    }
}

/// Ported from `validateTonProofItemReply` cases. The vectors use the current
/// normative string timestamp, 64-byte signature, and item error catalogue.
#[test]
fn typescript_ton_proof_reply_schema_cases() {
    let signature = STANDARD.encode([0_u8; 64]);
    let proof = json!({
        "name": "ton_proof",
        "proof": {
            "timestamp": "1",
            "domain": { "lengthBytes": 3, "value": "abc" },
            "payload": "some-payload",
            "signature": signature
        }
    });
    assert!(matches!(
        serde_json::from_value::<ConnectItemReply>(proof.clone()),
        Ok(ConnectItemReply::TonProof(_))
    ));
    assert!(matches!(
        serde_json::from_value::<ConnectItemReply>(
            json!({ "name": "ton_proof", "error": { "code": 400, "message": "unsupported" } })
        ),
        Ok(ConnectItemReply::Error(_))
    ));

    let mut both = proof.clone();
    if let Some(object) = both.as_object_mut() {
        let _ = object.insert("error".to_owned(), json!({ "code": 400 }));
    }
    let invalid = [
        ("missing proof and error", json!({ "name": "ton_proof" })),
        ("both proof and error", both),
        (
            "extra top-level property",
            json!({ "name": "ton_proof", "x": 1 }),
        ),
        (
            "numeric timestamp",
            json!({ "name": "ton_proof", "proof": { "timestamp": 1, "domain": { "lengthBytes": 3, "value": "abc" }, "payload": "p", "signature": STANDARD.encode([0_u8; 64]) } }),
        ),
        (
            "wrong domain length",
            json!({ "name": "ton_proof", "proof": { "timestamp": "1", "domain": { "lengthBytes": 5, "value": "abc" }, "payload": "p", "signature": STANDARD.encode([0_u8; 64]) } }),
        ),
        (
            "short signature",
            json!({ "name": "ton_proof", "proof": { "timestamp": "1", "domain": { "lengthBytes": 3, "value": "abc" }, "payload": "p", "signature": "QQ==" } }),
        ),
        (
            "extra proof property",
            json!({ "name": "ton_proof", "proof": { "timestamp": "1", "domain": { "lengthBytes": 3, "value": "abc" }, "payload": "p", "signature": STANDARD.encode([0_u8; 64]), "x": 1 } }),
        ),
        (
            "invalid item error code",
            json!({ "name": "ton_proof", "error": { "code": 1, "message": "bad" } }),
        ),
    ];
    for (name, value) in invalid {
        assert!(
            serde_json::from_value::<ConnectItemReply>(value).is_err(),
            "accepted {name}"
        );
    }
}

/// The TypeScript manager uses a built-in fallback for fetch errors. Fetch and
/// fallback policy belong to the host. The core still enforces list shape.
#[test]
fn typescript_wallet_list_shape_cases_at_the_core_boundary() {
    assert!(serde_json::from_value::<WalletsList>(json!({})).is_err());
    assert!(serde_json::from_value::<WalletsList>(json!([])).is_err());
    assert!(serde_json::from_value::<WalletsList>(json!([{"name":"broken"}])).is_err());
}

#[test]
fn decoded_payload_variants_remain_typed() {
    let transaction = decode(
        "sendTransaction",
        json!({ "messages": [{ "address": FRIENDLY_ADDRESS, "amount": "1" }] }),
    );
    assert!(matches!(
        transaction,
        Ok(KnownAppRequest::SendTransaction(request))
            if matches!(request.payload, TransactionPayload::Raw(_))
    ));

    let sign_data = decode("signData", json!({ "type": "text", "text": "hello" }));
    assert!(matches!(
        sign_data,
        Ok(KnownAppRequest::SignData(request))
            if matches!(request.payload, SignDataPayload::Text { .. })
    ));
}

/// Additional cross-model regression derived from the TypeScript null cases.
/// Every optional wire field can be absent, but explicit JSON null is invalid.
#[test]
fn every_protocol_optional_field_rejects_explicit_null() {
    assert!(
        serde_json::from_value::<ConnectItem>(json!({
            "name": "ton_addr",
            "network": null
        }))
        .is_err()
    );

    for feature in [
        json!({ "name": "SendTransaction", "maxMessages": 1, "extraCurrencySupported": null }),
        json!({ "name": "SendTransaction", "maxMessages": 1, "itemTypes": null }),
        json!({ "name": "SignMessage", "maxMessages": 1, "extraCurrencySupported": null }),
        json!({ "name": "SignMessage", "maxMessages": 1, "itemTypes": null }),
    ] {
        let device = json!({
            "platform": "browser",
            "appName": "wallet",
            "appVersion": "1",
            "maxProtocolVersion": 2,
            "features": [feature]
        });
        assert!(serde_json::from_value::<DeviceInfo>(device).is_err());
    }

    assert!(
        serde_json::from_value::<ConnectItemReply>(json!({
            "name": "future_item",
            "error": { "code": 400, "message": null }
        }))
        .is_err()
    );

    assert!(
        serde_json::from_value::<ConnectEvent>(json!({
            "event": "connect",
            "id": 1,
            "payload": {
                "items": [],
                "device": {
                    "platform": "browser",
                    "appName": "wallet",
                    "appVersion": "1",
                    "maxProtocolVersion": 2,
                    "features": []
                }
            },
            "response": null
        }))
        .is_err()
    );

    for field in ["termsOfUseUrl", "privacyPolicyUrl"] {
        let mut manifest = json!({
            "url": "https://app.example",
            "name": "App",
            "iconUrl": "https://app.example/icon.png"
        });
        if let Some(object) = manifest.as_object_mut() {
            let _ = object.insert(field.to_owned(), Value::Null);
        }
        assert!(serde_json::from_value::<AppManifest>(manifest).is_err());
    }

    assert!(
        serde_json::from_value::<BridgeMessage>(json!({
            "from": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "message": "AA==",
            "trace_id": null
        }))
        .is_err()
    );

    for field in ["tondns", "universal_url", "deepLink"] {
        let mut wallet = json!({
            "app_name": "wallet",
            "name": "Wallet",
            "image": "https://wallet.example/icon.png",
            "about_url": "https://wallet.example",
            "bridge": [{ "type": "js", "key": "wallet" }],
            "platforms": ["chrome"],
            "features": [{ "name": "SendTransaction", "maxMessages": 1 }]
        });
        if let Some(object) = wallet.as_object_mut() {
            let _ = object.insert(field.to_owned(), Value::Null);
        }
        assert!(serde_json::from_value::<WalletsList>(json!([wallet])).is_err());
    }

    for wire in [
        json!({ "m": "st", "n": null, "ms": [{ "a": FRIENDLY_ADDRESS, "am": "1" }] }),
        json!({ "m": "st", "f": null, "ms": [{ "a": FRIENDLY_ADDRESS, "am": "1" }] }),
        json!({ "m": "st", "ms": [{ "a": FRIENDLY_ADDRESS, "am": "1", "p": null }] }),
        json!({ "m": "sd", "t": "text", "tx": "hello", "n": null }),
    ] {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(wire.to_string());
        assert!(decode_embedded_request_param(&encoded).is_err());
    }
}

fn decoded_embedded_payload(
    request: &EmbeddedRequest,
) -> Result<(&'static str, Value), serde_json::Error> {
    match request {
        EmbeddedRequest::SendTransaction(payload) => {
            Ok(("sendTransaction", serde_json::to_value(payload)?))
        }
        EmbeddedRequest::SignMessage(payload) => {
            Ok(("signMessage", serde_json::to_value(payload)?))
        }
        EmbeddedRequest::SignData(payload) => Ok(("signData", serde_json::to_value(payload)?)),
    }
}

/// Ported from every case in the official protocol
/// `embedded-request.spec.ts` at
/// `273bc3a6050e6024886ca50c12677dc42ae142a9`.
#[test]
#[allow(
    clippy::too_many_lines,
    clippy::unreadable_literal,
    reason = "keeps the complete official embedded-request vector table auditable"
)]
fn official_typescript_embedded_request_vectors() -> Result<(), Box<dyn std::error::Error>> {
    const ADDRESS: &str = "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs";
    const ZERO: &str = "EQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAM9c";
    const PAYLOAD: &str = "te6cckEBAQEADAAAFAAAAABIZWxsbyGVgYQo";
    const SDK_STATE_INIT_FIXTURE: &str = "te6cckEBAQEABgAACAAAAABT+rFy";
    const STATE_INIT: &str = EMPTY_CELL_BOC;
    assert!(CellBoc::try_from(PAYLOAD).is_ok(), "official payload BoC");
    // The SDK test validates only field expansion. Its state-init fixture has
    // an invalid CRC32C, so the semantic protocol boundary must reject it.
    assert!(CellBoc::try_from(SDK_STATE_INIT_FIXTURE).is_err());
    let valid_from = format!("0:{}", "ab".repeat(32));

    let cases = [
        (
            "raw transaction",
            json!({ "m": "st", "n": "-239", "vu": 1761071945, "ms": [{ "a": ADDRESS, "am": "1000000000" }] }),
            "sendTransaction",
            json!({ "valid_until": 1761071945, "network": "-239", "messages": [{ "address": ADDRESS, "amount": "1000000000" }] }),
        ),
        (
            "raw transaction optional fields",
            json!({ "m": "st", "vu": 1761071945, "ms": [{ "a": ADDRESS, "am": "500000000", "p": PAYLOAD, "si": STATE_INIT, "ec": { "239": "100" } }] }),
            "sendTransaction",
            json!({ "valid_until": 1761071945, "messages": [{ "address": ADDRESS, "amount": "500000000", "payload": PAYLOAD, "stateInit": STATE_INIT, "extra_currency": { "239": "100" } }] }),
        ),
        (
            "ton item",
            json!({ "m": "st", "vu": 1761071945, "i": [{ "t": "ton", "a": ADDRESS, "am": "1000000000", "p": PAYLOAD }] }),
            "sendTransaction",
            json!({ "valid_until": 1761071945, "items": [{ "type": "ton", "address": ADDRESS, "amount": "1000000000", "payload": PAYLOAD }] }),
        ),
        (
            "jetton all fields",
            json!({ "m": "st", "n": "-239", "vu": 1761071945, "i": [{ "t": "jetton", "ma": ADDRESS, "d": ZERO, "am": "10000000", "aa": "50000000", "rd": ADDRESS, "cp": PAYLOAD, "fa": "50", "fp": PAYLOAD, "qi": "42" }] }),
            "sendTransaction",
            json!({ "valid_until": 1761071945, "network": "-239", "items": [{ "type": "jetton", "master": ADDRESS, "destination": ZERO, "amount": "10000000", "attachAmount": "50000000", "responseDestination": ADDRESS, "customPayload": PAYLOAD, "forwardAmount": "50", "forwardPayload": PAYLOAD, "queryId": "42" }] }),
        ),
        (
            "jetton required fields",
            json!({ "m": "st", "vu": 1761071945, "i": [{ "t": "jetton", "ma": ADDRESS, "d": ZERO, "am": "10000000" }] }),
            "sendTransaction",
            json!({ "valid_until": 1761071945, "items": [{ "type": "jetton", "master": ADDRESS, "destination": ZERO, "amount": "10000000" }] }),
        ),
        (
            "nft all fields",
            json!({ "m": "st", "vu": 1761071945, "i": [{ "t": "nft", "na": ADDRESS, "no": ZERO, "aa": "100000000", "rd": ADDRESS, "cp": PAYLOAD, "fa": "1", "fp": PAYLOAD, "qi": "99" }] }),
            "sendTransaction",
            json!({ "valid_until": 1761071945, "items": [{ "type": "nft", "nftAddress": ADDRESS, "newOwner": ZERO, "attachAmount": "100000000", "responseDestination": ADDRESS, "customPayload": PAYLOAD, "forwardAmount": "1", "forwardPayload": PAYLOAD, "queryId": "99" }] }),
        ),
        (
            "mixed items",
            json!({ "m": "st", "n": "-239", "vu": 1761071945, "i": [{ "t": "ton", "a": ADDRESS, "am": "100000000" }, { "t": "jetton", "ma": ADDRESS, "d": ZERO, "am": "10000000", "rd": ADDRESS, "fa": "10000" }] }),
            "sendTransaction",
            json!({ "valid_until": 1761071945, "network": "-239", "items": [{ "type": "ton", "address": ADDRESS, "amount": "100000000" }, { "type": "jetton", "master": ADDRESS, "destination": ZERO, "amount": "10000000", "responseDestination": ADDRESS, "forwardAmount": "10000" }] }),
        ),
        (
            "sign message",
            json!({ "m": "sm", "n": "-239", "vu": 1761071945, "ms": [{ "a": ADDRESS, "am": "0" }] }),
            "signMessage",
            json!({ "valid_until": 1761071945, "network": "-239", "messages": [{ "address": ADDRESS, "amount": "0" }] }),
        ),
        (
            "sign data text",
            json!({ "m": "sd", "n": "-239", "f": valid_from, "t": "text", "tx": "Hello, world!" }),
            "signData",
            json!({ "network": "-239", "from": valid_from, "type": "text", "text": "Hello, world!" }),
        ),
        (
            "sign data binary",
            json!({ "m": "sd", "t": "binary", "b": "AQIDBA==" }),
            "signData",
            json!({ "type": "binary", "bytes": "AQIDBA==" }),
        ),
        (
            "sign data cell",
            json!({ "m": "sd", "t": "cell", "s": "some_schema", "c": STATE_INIT }),
            "signData",
            json!({ "type": "cell", "schema": "some_schema", "cell": STATE_INIT }),
        ),
    ];

    for (name, wire, method, expected) in cases {
        let parameter = URL_SAFE_NO_PAD.encode(wire.to_string());
        assert!(!parameter.contains(['+', '/', '=']), "non-URL-safe {name}");
        let request = decode_embedded_request_param(&parameter)
            .map_err(|error| std::io::Error::other(format!("failed {name}: {error}")))?;
        let (actual_method, actual_payload) = decoded_embedded_payload(&request)?;
        assert_eq!(actual_method, method, "method mismatch for {name}");
        assert_eq!(actual_payload, expected, "payload mismatch for {name}");

        let encoded = encode_embedded_request_param(&request)
            .map_err(|error| std::io::Error::other(format!("failed to encode {name}: {error}")))?;
        assert!(!encoded.contains(['+', '/', '=']), "non-URL-safe {name}");
        assert_eq!(
            decode_embedded_request_param(&encoded)?,
            request,
            "round-trip mismatch for {name}"
        );
    }
    Ok(())
}

/// Ported from every protocol-relevant case in the official SDK
/// `normalize-structured-item.test.ts` at
/// `273bc3a6050e6024886ca50c12677dc42ae142a9`.
#[test]
fn official_typescript_structured_item_vectors() -> Result<(), Box<dyn std::error::Error>> {
    const ADDRESS: &str = "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs";
    const ZERO: &str = "EQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAM9c";

    let ton_json = json!({
        "type": "ton",
        "address": ZERO,
        "amount": "1000",
        "payload": EMPTY_CELL_BOC,
        "stateInit": EMPTY_CELL_BOC,
        "extra_currency": { "239": "100" }
    });
    let ton = serde_json::from_str::<StructuredItem>(&ton_json.to_string())?;
    assert_eq!(
        serde_json::to_value(&ton)?,
        json!({
            "type": "ton",
            "address": ZERO,
            "amount": "1000",
            "payload": EMPTY_CELL_BOC,
            "stateInit": EMPTY_CELL_BOC,
            "extra_currency": { "239": "100" }
        })
    );

    for amount in ["0", "340282366920938463463374607431768211455"] {
        let item = serde_json::from_value::<StructuredItem>(json!({
            "type": "ton", "address": ZERO, "amount": amount
        }))?;
        assert_eq!(
            serde_json::to_value(item)?
                .get("amount")
                .and_then(Value::as_str),
            Some(amount)
        );
    }

    for item in [
        json!({
            "type": "jetton",
            "master": ADDRESS,
            "destination": ZERO,
            "amount": "1000000",
            "customPayload": EMPTY_CELL_BOC,
            "forwardPayload": EMPTY_CELL_BOC,
            "attachAmount": "50000000",
            "forwardAmount": "1",
            "queryId": "42",
            "responseDestination": ZERO
        }),
        json!({
            "type": "nft",
            "nftAddress": ADDRESS,
            "newOwner": ZERO,
            "customPayload": EMPTY_CELL_BOC,
            "forwardPayload": EMPTY_CELL_BOC,
            "attachAmount": "100000000",
            "forwardAmount": "1",
            "queryId": "99"
        }),
        json!({ "type": "jetton", "master": ADDRESS, "destination": ZERO, "amount": "1000" }),
        json!({ "type": "nft", "nftAddress": ADDRESS, "newOwner": ZERO }),
    ] {
        let parsed = serde_json::from_str::<StructuredItem>(&item.to_string())?;
        let wire = serde_json::to_value(parsed)?;
        assert!(!wire.as_object().is_some_and(|object| {
            object.contains_key("extraCurrency") || object.contains_key("stateInit")
        }));
        assert!(!wire.as_object().is_some_and(|object| {
            object.get("customPayload") == Some(&Value::Null)
                || object.get("forwardPayload") == Some(&Value::Null)
        }));
    }

    assert!(
        serde_json::from_value::<StructuredItem>(json!({
            "type": "jetton",
            "master": ADDRESS,
            "destination": ZERO,
            "amount": "1",
            "stateInit": EMPTY_CELL_BOC
        }))
        .is_err()
    );
    Ok(())
}

/// Adapted from every `validateEmbeddedRequest` case in the official SDK
/// schema suite at `273bc3a6050e6024886ca50c12677dc42ae142a9`.
#[test]
fn official_typescript_embedded_validation_cases() {
    for valid in [
        json!({ "m": "st", "ms": [{ "a": FRIENDLY_ADDRESS, "am": "1000" }] }),
        json!({ "m": "sm", "ms": [{ "a": FRIENDLY_ADDRESS, "am": "1000" }] }),
        json!({ "m": "sd", "t": "text", "tx": "hello" }),
    ] {
        let encoded = URL_SAFE_NO_PAD.encode(valid.to_string());
        assert!(decode_embedded_request_param(&encoded).is_ok());
    }

    for (name, invalid) in [
        ("null", json!(null)),
        ("string", json!("foo")),
        (
            "extra top-level property",
            json!({ "m": "st", "ms": [{ "a": FRIENDLY_ADDRESS, "am": "1" }], "garbage": "x" }),
        ),
        (
            "missing method",
            json!({ "ms": [{ "a": FRIENDLY_ADDRESS, "am": "1" }] }),
        ),
        ("missing request body", json!({ "m": "st" })),
        ("disconnect is not embeddable", json!({ "m": "disconnect" })),
        (
            "invalid inner transaction",
            json!({ "m": "st", "n": "-239" }),
        ),
    ] {
        let encoded = URL_SAFE_NO_PAD.encode(invalid.to_string());
        assert!(
            decode_embedded_request_param(&encoded).is_err(),
            "accepted {name}"
        );
    }
}

#[test]
fn extra_currency_keys_are_canonical_and_collision_free() -> Result<(), serde_json::Error> {
    let parsed = serde_json::from_str::<ExtraCurrencies>(r#"{"0":"0","239":"100"}"#)?;
    assert_eq!(serde_json::to_string(&parsed)?, r#"{"0":"0","239":"100"}"#);

    for invalid in [
        r#"{"01":"1"}"#,
        r#"{"-1":"1"}"#,
        r#"{"4294967296":"1"}"#,
        r#"{"x":"1"}"#,
        r#"{"1":"x"}"#,
    ] {
        assert!(serde_json::from_str::<ExtraCurrencies>(invalid).is_err());
    }
    Ok(())
}
