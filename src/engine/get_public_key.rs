//! The on-chain `get_public_key` fallback for TON Connect account verification.
//!
//! TON Connect requires a verifier that cannot parse an account's
//! `walletStateInit` locally to read `get_public_key` from the deployed
//! contract instead. This module owns the provider half of that step: it builds
//! the Toncenter `runGetMethod` request and parses the returned TVM stack. The
//! embedding application performs the HTTP call, as it does for every other
//! engine request, and passes the key to
//! [`TonProof::verify_with_fetched_key`](ton_connect_core::TonProof::verify_with_fetched_key)
//! or
//! [`SignDataResult::verify_with_fetched_key`](ton_connect_core::SignDataResult::verify_with_fetched_key).

use num_bigint::BigUint;
use serde_json::Value;

use crate::{
    DomainError, HttpRequest, HttpRequestId, TonAddressString, WalletClientConfig,
    WalletClientError,
};

use super::send_http::{build_json_rpc_request, invalid_json, stack_num};

/// Builds the request that reads `get_public_key` from `address`.
///
/// `address` is the account being verified, which is unrelated to the wallet
/// this client sends from. Only the provider endpoint and request limits come
/// from `config`.
pub fn build_get_public_key_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    address: &TonAddressString,
) -> Result<HttpRequest, WalletClientError> {
    build_json_rpc_request(
        config,
        id,
        "runGetMethod",
        &serde_json::json!({
            "address": address,
            "method": "get_public_key",
            "stack": []
        }),
    )
}

/// Parses the Ed25519 public key returned by `get_public_key`.
///
/// `body` is the provider response body for a request built by
/// [`build_get_public_key_request`]. The returned value is the raw 32-byte key,
/// which `ton_connect_core::Ed25519PublicKey::from_bytes` accepts directly.
///
/// The get-method returns a TVM integer, so the value is decoded as a
/// big-endian unsigned number and left-padded to 32 bytes. A contract that does
/// not implement the method fails with a non-zero exit code, which this
/// function reports rather than treating as a malformed response.
pub fn parse_get_public_key_response(body: &[u8]) -> Result<[u8; 32], DomainError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| invalid_json(error.to_string()))?;

    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        return Err(invalid_json(error.to_string()));
    }

    if let Some(exit_code) = value.pointer("/result/exit_code").and_then(Value::as_i64)
        && exit_code != 0
    {
        return Err(invalid_json(format!(
            "get_public_key exited with code {exit_code}"
        )));
    }

    let first = value
        .pointer("/result/stack/0")
        .ok_or_else(|| invalid_json("missing get_public_key stack"))?;
    let encoded = stack_num(first).ok_or_else(|| invalid_json("invalid get_public_key value"))?;

    decode_public_key(encoded)
}

/// Decodes a hexadecimal or decimal TVM integer into a 256-bit key.
///
/// A provider strips leading zero bytes, so a shorter value is left-padded. A
/// negative or wider value cannot be a public key and is rejected.
fn decode_public_key(encoded: &str) -> Result<[u8; 32], DomainError> {
    let value = if let Some(hex) = encoded.strip_prefix("0x") {
        BigUint::parse_bytes(hex.as_bytes(), 16)
    } else {
        BigUint::parse_bytes(encoded.as_bytes(), 10)
    }
    .ok_or_else(|| invalid_json("invalid get_public_key value"))?;

    let bytes = value.to_bytes_be();
    let mut key = [0_u8; 32];
    let padding = key
        .len()
        .checked_sub(bytes.len())
        .ok_or_else(|| invalid_json("get_public_key returned more than 256 bits"))?;
    key.get_mut(padding..)
        .ok_or_else(|| invalid_json("get_public_key returned more than 256 bits"))?
        .copy_from_slice(&bytes);

    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::{build_get_public_key_request, parse_get_public_key_response};
    use crate::{
        ErrorCategory, ErrorCode, HttpRequestId, Network, NonEmptyString, ProviderConfig,
        TonAddressString, WalletClientConfig,
    };

    const WALLET: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";
    const VERIFIED: &str = "0:2222222222222222222222222222222222222222222222222222222222222222";

    #[test]
    fn reads_the_get_method_of_the_verified_account_not_the_client_wallet() {
        let address = TonAddressString::try_from(VERIFIED).expect("valid raw address");
        let request = build_get_public_key_request(&config(), HttpRequestId { value: 9 }, &address)
            .expect("get_public_key request must build");
        let body: serde_json::Value =
            serde_json::from_slice(&request.body).expect("request body must be JSON");

        assert_eq!(body["method"], "runGetMethod");
        assert_eq!(body["params"]["method"], "get_public_key");
        assert_eq!(body["params"]["address"], VERIFIED);
        assert_eq!(body["params"]["stack"], serde_json::json!([]));
    }

    #[test]
    fn parses_supported_stack_shapes_and_radices() {
        let key = [0xab_u8; 32];
        let hex = format!("0x{}", "ab".repeat(32));
        let decimal = num_bigint::BigUint::from_bytes_be(&key).to_str_radix(10);

        for body in [
            format!(r#"{{"result":{{"exit_code":0,"stack":[["num","{hex}"]]}}}}"#),
            format!(r#"{{"result":{{"stack":[{{"type":"num","value":"{hex}"}}]}}}}"#),
            format!(r#"{{"result":{{"stack":[["num","{decimal}"]]}}}}"#),
        ] {
            assert_eq!(parse_get_public_key_response(body.as_bytes()), Ok(key));
        }
    }

    #[test]
    fn left_pads_a_key_whose_leading_zero_bytes_the_provider_stripped() {
        let body = br#"{"result":{"stack":[["num","0x2a"]]}}"#;
        let mut expected = [0_u8; 32];
        expected[31] = 42;

        assert_eq!(parse_get_public_key_response(body), Ok(expected));
    }

    #[test]
    fn rejects_responses_that_carry_no_usable_key() {
        for body in [
            // A contract without the get-method fails inside the TVM.
            r#"{"result":{"exit_code":11,"stack":[]}}"#,
            r#"{"error":{"message":"method failed"}}"#,
            r#"{"result":{"stack":[]}}"#,
            r#"{"result":{"stack":[["cell","1"]]}}"#,
            r#"{"result":{"stack":[["num","not-a-number"]]}}"#,
            r#"{"result":{"stack":[["num","-0x2a"]]}}"#,
            // 257 bits cannot be an Ed25519 key.
            r#"{"result":{"stack":[["num","0x1ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"]]}}"#,
            "not json",
        ] {
            let error = parse_get_public_key_response(body.as_bytes())
                .expect_err("response carries no public key");
            assert_eq!(error.code, ErrorCode::InvalidProviderResponse);
            assert_eq!(error.category, ErrorCategory::ProviderProtocol);
        }
    }

    fn config() -> WalletClientConfig {
        WalletClientConfig {
            record_id: NonEmptyString::try_from("record").expect("non-empty record id"),
            address: TonAddressString::try_from(WALLET).expect("valid raw address"),
            public_key: vec![0; 32],
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig {
                toncenter_base_url: "https://provider.example".to_owned(),
                request_timeout_ms: 15_000,
            },
        }
    }
}
