//! The on-chain `get_public_key` read that resolves an unrecognized account.
//!
//! A verifier that cannot parse an account's `walletStateInit` reads
//! `get_public_key` from the deployed contract instead, then finishes with
//! [`TonProof::verify_with_fetched_key`](crate::TonProof::verify_with_fetched_key).
//! This module models the wire format of that read for the TON HTTP API, whose
//! `runGetMethod` request and TVM stack every common provider serves.
//!
//! Like the rest of this crate, it performs no I/O. The caller posts the body
//! to the endpoint it already uses for chain access and hands the response back
//! for parsing, so a verifier is never tied to one provider.

use serde_json::Value;
use thiserror::Error;

use crate::{Ed25519PublicKey, RawAccountAddress};

/// Builds the `runGetMethod` request body that reads `get_public_key`.
///
/// `id` correlates the JSON-RPC response and is opaque to the method itself.
/// Post the returned body as `application/json` to the provider's
/// `/api/v2/jsonRPC` endpoint.
#[must_use]
pub fn get_public_key_request(address: &RawAccountAddress, id: &str) -> String {
    // `Value` renders itself infallibly, so the body needs no fallible encode.
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "runGetMethod",
        "params": {
            "address": address.to_string(),
            "method": "get_public_key",
            "stack": []
        }
    })
    .to_string()
}

/// Parses the public key returned by a [`get_public_key_request`] response.
///
/// The get-method returns a TVM integer, so the value is read as a big-endian
/// unsigned number and left-padded to the 32 bytes of an Ed25519 key.
///
/// An account that does not implement the method fails inside the TVM instead
/// of returning a value, and an account that is not deployed yet cannot run a
/// get-method at all. Both surface here as an error rather than a key.
pub fn parse_get_public_key_response(body: &[u8]) -> Result<Ed25519PublicKey, GetPublicKeyError> {
    let value =
        serde_json::from_slice::<Value>(body).map_err(|_| GetPublicKeyError::InvalidResponse)?;

    if value.get("error").is_some_and(|error| !error.is_null())
        || value.get("ok") == Some(&Value::Bool(false))
    {
        return Err(GetPublicKeyError::ProviderRejected);
    }

    if let Some(exit_code) = value.pointer("/result/exit_code").and_then(Value::as_i64)
        && exit_code != 0
    {
        return Err(GetPublicKeyError::ExitCode(exit_code));
    }

    let entry = value
        .pointer("/result/stack/0")
        .ok_or(GetPublicKeyError::InvalidStackValue)?;
    let encoded = stack_integer(entry).ok_or(GetPublicKeyError::InvalidStackValue)?;

    decode_key(encoded)
        .map(Ed25519PublicKey::from_bytes)
        .ok_or(GetPublicKeyError::InvalidPublicKey)
}

/// Failure to read a public key from a `get_public_key` response.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum GetPublicKeyError {
    /// The body is not the JSON shape a get-method result uses.
    #[error("get_public_key response is not a readable JSON-RPC result")]
    InvalidResponse,
    /// The provider reported a failure instead of running the method.
    #[error("provider rejected the get_public_key request")]
    ProviderRejected,
    /// The method failed inside the TVM, so the account cannot answer it.
    #[error("get_public_key exited with TVM code {0}")]
    ExitCode(i64),
    /// The first stack entry is missing or is not a TVM integer.
    #[error("get_public_key did not return an integer")]
    InvalidStackValue,
    /// The returned integer is negative, empty, or wider than 256 bits.
    #[error("get_public_key returned a value that is not a 256-bit key")]
    InvalidPublicKey,
}

/// Borrows the text of a TVM integer at one stack position.
///
/// Providers return a stack entry either as a `[kind, value]` pair or as a
/// tagged object, so both shapes are accepted. Integers arrive as text because
/// a 257-bit TVM value does not fit a JSON number.
fn stack_integer(entry: &Value) -> Option<&str> {
    entry
        .as_array()
        .and_then(|items| match items.as_slice() {
            [kind, value] if kind.as_str() == Some("num") => value.as_str(),
            _ => None,
        })
        .or_else(|| {
            (entry.get("type").and_then(Value::as_str) == Some("num"))
                .then(|| entry.get("value").and_then(Value::as_str))
                .flatten()
        })
}

/// Decodes a hexadecimal or decimal TVM integer into a 256-bit key.
fn decode_key(encoded: &str) -> Option<[u8; 32]> {
    encoded
        .strip_prefix("0x")
        .map_or_else(|| decode_decimal(encoded), decode_hex)
}

/// Reads hexadecimal digits, left-padding a value whose leading zero bytes the
/// provider stripped.
fn decode_hex(digits: &str) -> Option<[u8; 32]> {
    if digits.is_empty() || digits.len() > 64 {
        return None;
    }
    let mut key = [0_u8; 32];
    hex::decode_to_slice(format!("{digits:0>64}"), &mut key).ok()?;
    Some(key)
}

/// Reads decimal digits by accumulating them into the key's bytes.
///
/// A carry out of the most significant byte means the value needs more than
/// 256 bits and cannot be a key.
fn decode_decimal(digits: &str) -> Option<[u8; 32]> {
    if digits.is_empty() {
        return None;
    }
    let mut key = [0_u8; 32];
    for digit in digits.chars() {
        let mut carry = digit.to_digit(10)?;
        for byte in key.iter_mut().rev() {
            let value = u32::from(*byte).checked_mul(10)?.checked_add(carry)?;
            *byte = u8::try_from(value & 0xff).ok()?;
            carry = value >> 8;
        }
        if carry != 0 {
            return None;
        }
    }
    Some(key)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{GetPublicKeyError, get_public_key_request, parse_get_public_key_response};
    use crate::{Ed25519PublicKey, RawAccountAddress};

    type TestResult = Result<(), Box<dyn Error>>;

    const ADDRESS: &str = "0:2222222222222222222222222222222222222222222222222222222222222222";

    #[test]
    fn requests_the_get_method_of_the_verified_account() -> TestResult {
        let address = ADDRESS.parse::<RawAccountAddress>()?;
        let body =
            serde_json::from_str::<serde_json::Value>(&get_public_key_request(&address, "7"))?;

        assert_eq!(body["jsonrpc"], "2.0");
        assert_eq!(body["id"], "7");
        assert_eq!(body["method"], "runGetMethod");
        assert_eq!(body["params"]["address"], ADDRESS);
        assert_eq!(body["params"]["method"], "get_public_key");
        assert_eq!(body["params"]["stack"], serde_json::json!([]));
        Ok(())
    }

    #[test]
    fn parses_supported_stack_shapes_and_radices() {
        let key = Ed25519PublicKey::from_bytes([0xab; 32]);
        let hex = "ab".repeat(32);
        // The same value in base ten, so both radices must decode identically.
        let decimal = "77648812782670860460512307594061302913\
                       369283834606025297048026922953510464427";

        for body in [
            format!(r#"{{"result":{{"exit_code":0,"stack":[["num","0x{hex}"]]}}}}"#),
            format!(r#"{{"result":{{"stack":[{{"type":"num","value":"0x{hex}"}}]}}}}"#),
            format!(r#"{{"result":{{"stack":[["num","{decimal}"]]}}}}"#),
        ] {
            assert_eq!(parse_get_public_key_response(body.as_bytes()), Ok(key));
        }
    }

    #[test]
    fn left_pads_a_key_whose_leading_zero_bytes_the_provider_stripped() {
        let mut expected = [0_u8; 32];
        expected[31] = 42;

        for body in [
            br#"{"result":{"stack":[["num","0x2a"]]}}"#.as_slice(),
            br#"{"result":{"stack":[["num","42"]]}}"#.as_slice(),
        ] {
            assert_eq!(
                parse_get_public_key_response(body),
                Ok(Ed25519PublicKey::from_bytes(expected))
            );
        }
    }

    #[test]
    fn every_response_without_a_usable_key_names_its_reason() {
        let cases = [
            (
                // An account that does not implement the method, and an account
                // that is not deployed, both fail inside the TVM.
                r#"{"result":{"exit_code":11,"stack":[]}}"#,
                GetPublicKeyError::ExitCode(11),
            ),
            (
                r#"{"error":{"message":"method failed"}}"#,
                GetPublicKeyError::ProviderRejected,
            ),
            (
                r#"{"ok":false,"description":"rejected"}"#,
                GetPublicKeyError::ProviderRejected,
            ),
            (
                r#"{"result":{"stack":[]}}"#,
                GetPublicKeyError::InvalidStackValue,
            ),
            (
                r#"{"result":{"stack":[["cell","1"]]}}"#,
                GetPublicKeyError::InvalidStackValue,
            ),
            (
                r#"{"result":{"stack":[["num","not-a-number"]]}}"#,
                GetPublicKeyError::InvalidPublicKey,
            ),
            (
                r#"{"result":{"stack":[["num","-42"]]}}"#,
                GetPublicKeyError::InvalidPublicKey,
            ),
            (
                r#"{"result":{"stack":[["num","-0x2a"]]}}"#,
                GetPublicKeyError::InvalidPublicKey,
            ),
            (
                // 257 bits cannot be an Ed25519 key.
                r#"{"result":{"stack":[["num","0x1ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"]]}}"#,
                GetPublicKeyError::InvalidPublicKey,
            ),
            (
                r#"{"result":{"stack":[["num","115792089237316195423570985008687907853269984665640564039457584007913129639936"]]}}"#,
                GetPublicKeyError::InvalidPublicKey,
            ),
            ("not json", GetPublicKeyError::InvalidResponse),
        ];

        for (body, expected) in cases {
            assert_eq!(
                parse_get_public_key_response(body.as_bytes()),
                Err(expected)
            );
        }
    }
}
