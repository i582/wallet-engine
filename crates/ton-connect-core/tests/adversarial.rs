//! Adversarial parser checks for every untrusted TON Connect JSON boundary.

use std::{collections::BTreeMap, panic::catch_unwind};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use proptest::prelude::*;
use serde_json::{Map, Number, Value};
use ton_connect_core::{
    AppManifest, AppMessage, AppRequest, BridgeMessage, ConnectEvent, ConnectRequest, DeviceInfo,
    WalletMessage, WalletResponse, WalletsList, decode_embedded_request_param,
};

fn json_values() -> BoxedStrategy<Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|value| Value::Number(Number::from(value))),
        ".{0,64}".prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 64, 8, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            proptest::collection::btree_map("[A-Za-z_][A-Za-z0-9_]{0,15}", inner, 0..8).prop_map(
                |values: BTreeMap<String, Value>| {
                    Value::Object(values.into_iter().collect::<Map<_, _>>())
                }
            ),
        ]
    })
    .boxed()
}

proptest! {
    #[test]
    fn arbitrary_json_never_panics_at_any_protocol_envelope(value in json_values()) {
        let json = value.to_string();
        prop_assert!(catch_unwind(|| serde_json::from_str::<AppMessage>(&json)).is_ok());
        prop_assert!(catch_unwind(|| serde_json::from_str::<WalletMessage>(&json)).is_ok());
        prop_assert!(catch_unwind(|| serde_json::from_str::<ConnectRequest>(&json)).is_ok());
        prop_assert!(catch_unwind(|| serde_json::from_str::<ConnectEvent>(&json)).is_ok());
        prop_assert!(catch_unwind(|| serde_json::from_str::<AppRequest>(&json)).is_ok());
        prop_assert!(catch_unwind(|| serde_json::from_str::<WalletResponse>(&json)).is_ok());
        prop_assert!(catch_unwind(|| serde_json::from_str::<DeviceInfo>(&json)).is_ok());
        prop_assert!(catch_unwind(|| serde_json::from_str::<AppManifest>(&json)).is_ok());
        prop_assert!(catch_unwind(|| serde_json::from_str::<BridgeMessage>(&json)).is_ok());
        prop_assert!(catch_unwind(|| serde_json::from_str::<WalletsList>(&json)).is_ok());

        if let Ok(request) = serde_json::from_str::<AppRequest>(&json) {
            prop_assert!(catch_unwind(|| request.decode()).is_ok());
        }
        let embedded = URL_SAFE_NO_PAD.encode(json.as_bytes());
        prop_assert!(catch_unwind(|| decode_embedded_request_param(&embedded)).is_ok());
    }
}
