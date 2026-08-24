//! Toncenter JSON-RPC requests and responses used by send.

use num_bigint::BigUint;
use serde_json::Value;

use crate::domain::bounded_diagnostic;
use crate::transport::build_toncenter_v2_request;
use crate::types::Boc;
use crate::{
    DomainError, ErrorCategory, ErrorCode, HttpHeader, HttpMethod, HttpRequest, HttpRequestId,
    RetryAdvice, WalletClientConfig, WalletClientError,
};

pub(super) enum SendBocResponse {
    Accepted,
    Rejected(String),
}

pub(super) fn build_seqno_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
) -> Result<HttpRequest, WalletClientError> {
    build_json_rpc_request(
        config,
        id,
        "runGetMethod",
        &serde_json::json!({
            "address": config.address,
            "method": "seqno",
            "stack": []
        }),
    )
}

pub(super) fn build_public_key_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    address: &crate::TonAddressString,
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

pub(super) fn build_send_boc_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    boc: &Boc,
) -> Result<HttpRequest, WalletClientError> {
    build_json_rpc_request(config, id, "sendBoc", &serde_json::json!({ "boc": boc }))
}

pub(super) fn parse_seqno(body: &[u8]) -> Result<u32, DomainError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| invalid_json(error.to_string()))?;

    if let Some(error) = value.get("error") {
        return Err(invalid_json(error.to_string()));
    }

    let first = value
        .pointer("/result/stack/0")
        .ok_or_else(|| invalid_json("missing seqno stack"))?;
    let encoded = first
        .as_array()
        .and_then(|items| match items.as_slice() {
            [kind, value] if kind.as_str() == Some("num") => value.as_str(),
            _ => None,
        })
        .or_else(|| {
            (first.get("type").and_then(Value::as_str) == Some("num"))
                .then(|| first.get("value").and_then(Value::as_str))
                .flatten()
        })
        .ok_or_else(|| invalid_json("invalid seqno value"))?;

    if let Some(hex) = encoded.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).map_err(|error| invalid_json(error.to_string()))
    } else {
        encoded
            .parse::<u32>()
            .map_err(|error| invalid_json(error.to_string()))
    }
}

pub(super) fn parse_public_key(body: &[u8]) -> Result<[u8; 32], DomainError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| invalid_json(error.to_string()))?;

    if let Some(error) = value.get("error") {
        return Err(invalid_json(error.to_string()));
    }

    let first = value
        .pointer("/result/stack/0")
        .ok_or_else(|| invalid_json("missing public-key stack"))?;
    let encoded =
        stack_number(first).ok_or_else(|| invalid_json("invalid public-key stack value"))?;
    let number = if let Some(hex) = encoded.strip_prefix("0x") {
        BigUint::parse_bytes(hex.as_bytes(), 16)
    } else {
        BigUint::parse_bytes(encoded.as_bytes(), 10)
    }
    .ok_or_else(|| invalid_json("invalid public-key integer"))?;
    if number == BigUint::default() {
        return Err(invalid_json("public key is not a nonzero uint256"));
    }
    let bytes = number.to_bytes_be();
    if bytes.is_empty() || bytes.len() > 32 {
        return Err(invalid_json("public key is not a nonzero uint256"));
    }

    let mut public_key = [0_u8; 32];
    let offset = 32_usize.saturating_sub(bytes.len());
    public_key
        .get_mut(offset..)
        .ok_or_else(|| invalid_json("public key is not a uint256"))?
        .copy_from_slice(&bytes);
    Ok(public_key)
}

fn stack_number(value: &Value) -> Option<&str> {
    value
        .as_array()
        .and_then(|items| match items.as_slice() {
            [kind, value] if kind.as_str() == Some("num") => value.as_str(),
            _ => None,
        })
        .or_else(|| {
            (value.get("type").and_then(Value::as_str) == Some("num"))
                .then(|| value.get("value").and_then(Value::as_str))
                .flatten()
        })
}

pub(super) fn parse_send_response(body: &[u8]) -> Result<SendBocResponse, DomainError> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| invalid_json(error.to_string()))?;

    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        return Ok(SendBocResponse::Rejected(json_error_message(error)));
    }

    if value.get("ok") == Some(&Value::Bool(false)) {
        return Ok(SendBocResponse::Rejected(json_error_message(&value)));
    }

    if value.pointer("/result/@type").and_then(Value::as_str) == Some("ok") {
        return Ok(SendBocResponse::Accepted);
    }

    Err(invalid_json("invalid sendBoc success response"))
}

pub(super) fn is_explicit_send_rejection(error: &DomainError) -> bool {
    error
        .provider_status
        .is_some_and(|status| matches!(status, 400 | 401 | 403 | 404 | 405 | 413 | 422 | 429))
}

pub(super) fn build_json_rpc_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    method: &str,
    params: &Value,
) -> Result<HttpRequest, WalletClientError> {
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.value.to_string(),
        "method": method,
        "params": params,
    }))
    .map_err(|_| WalletClientError::StateUnavailable)?;

    let mut request = build_toncenter_v2_request(config, id, "jsonRPC", &[])?;

    request.method = HttpMethod::Post;
    request.headers.push(HttpHeader {
        name: "Content-Type".to_owned(),
        value: "application/json".to_owned(),
    });
    request.body = body;

    Ok(request)
}

fn json_error_message(value: &Value) -> String {
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))
        .or_else(|| value.get("description").and_then(Value::as_str))
        .or_else(|| value.as_str())
        .map_or_else(|| value.to_string(), str::to_owned);

    bounded_diagnostic(message)
}

fn invalid_json(message: impl Into<String>) -> DomainError {
    DomainError {
        code: ErrorCode::InvalidProviderResponse,
        category: ErrorCategory::ProviderProtocol,
        retry: RetryAdvice::None,
        developer_message: bounded_diagnostic(message.into()),
        provider_status: None,
        retry_after_ms: None,
        host_kind: None,
    }
}

#[cfg(test)]
mod tests {
    use ton::ton_core::cell::TonCell;

    use super::{
        SendBocResponse, build_public_key_request, build_send_boc_request,
        is_explicit_send_rejection, parse_public_key, parse_send_response, parse_seqno,
    };
    use crate::{
        Boc, DomainError, ErrorCategory, ErrorCode, HttpRequestId, Network, NonEmptyString,
        ProviderConfig, RetryAdvice, TonAddressString, WalletClientConfig,
    };

    const ADDRESS: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn builds_send_boc_from_the_validated_boc() {
        let boc = Boc::try_from(TonCell::EMPTY_BOC.to_vec()).expect("valid BOC fixture");
        let request = build_send_boc_request(&config(), HttpRequestId { value: 7 }, &boc)
            .expect("sendBoc request must build");
        let body: serde_json::Value =
            serde_json::from_slice(&request.body).expect("request body must be JSON");

        assert_eq!(body["method"], "sendBoc");
        assert_eq!(body["params"]["boc"], boc.to_base64());
    }

    #[test]
    fn parses_supported_seqno_stack_shapes_and_radices() {
        let cases = [
            (r#"{"result":{"stack":[["num","0x2a"]]}}"#, 42),
            (r#"{"result":{"stack":[["num","42"]]}}"#, 42),
            (
                r#"{"result":{"stack":[{"type":"num","value":"0x2a"}]}}"#,
                42,
            ),
        ];

        for (body, expected) in cases {
            assert_eq!(parse_seqno(body.as_bytes()), Ok(expected));
        }
    }

    #[test]
    fn builds_and_parses_get_public_key_requests() {
        let address = TonAddressString::try_from(ADDRESS).expect("valid recipient");
        let request = build_public_key_request(&config(), HttpRequestId { value: 8 }, &address)
            .expect("get-public-key request must build");
        let body: serde_json::Value =
            serde_json::from_slice(&request.body).expect("request body must be JSON");
        assert_eq!(body["method"], "runGetMethod");
        assert_eq!(body["params"]["address"], ADDRESS);
        assert_eq!(body["params"]["method"], "get_public_key");

        let encoded = format!(
            r#"{{"result":{{"stack":[["num","0x{}"]]}}}}"#,
            "11".repeat(32)
        );
        assert_eq!(
            parse_public_key(encoded.as_bytes()).expect("uint256 key parses"),
            [0x11; 32]
        );
        assert_eq!(
            parse_public_key(br#"{"result":{"stack":[["num","1"]]}}"#)
                .expect("short decimal key is left padded"),
            {
                let mut expected = [0_u8; 32];
                expected[31] = 1;
                expected
            }
        );
    }

    #[test]
    fn rejects_invalid_public_keys() {
        for body in [
            r#"{"result":{"stack":[["num","0"]]}}"#.to_owned(),
            format!(
                r#"{{"result":{{"stack":[["num","0x{}"]]}}}}"#,
                "11".repeat(33)
            ),
            r#"{"result":{"stack":[["cell","1"]]}}"#.to_owned(),
        ] {
            let error = parse_public_key(body.as_bytes()).expect_err("public key must fail");
            assert_eq!(error.code, ErrorCode::InvalidProviderResponse);
        }
    }

    #[test]
    fn rejects_invalid_seqno_responses() {
        for body in [
            r#"{"error":{"message":"method failed"}}"#,
            r#"{"result":{"stack":[]}}"#,
            r#"{"result":{"stack":[["cell","1"]]}}"#,
            r#"{"result":{"stack":[["num","not-a-number"]]}}"#,
        ] {
            let error = parse_seqno(body.as_bytes()).expect_err("seqno response must fail");
            assert_eq!(error.code, ErrorCode::InvalidProviderResponse);
            assert_eq!(error.category, ErrorCategory::ProviderProtocol);
            assert_eq!(error.retry, RetryAdvice::None);
        }
    }

    #[test]
    fn parses_every_supported_send_boc_outcome() {
        assert!(matches!(
            parse_send_response(br#"{"result":{"@type":"ok"}}"#),
            Ok(SendBocResponse::Accepted)
        ));

        for body in [
            br#"{"error":{"message":"rejected"}}"#.as_slice(),
            br#"{"ok":false,"description":"rejected"}"#.as_slice(),
        ] {
            assert!(matches!(
                parse_send_response(body),
                Ok(SendBocResponse::Rejected(message)) if message == "rejected"
            ));
        }

        let Err(error) = parse_send_response(br#"{"result":null}"#) else {
            panic!("ambiguous response must fail");
        };
        assert_eq!(error.code, ErrorCode::InvalidProviderResponse);
    }

    #[test]
    fn only_definite_http_rejections_are_safe_to_replace() {
        for status in [400, 401, 403, 404, 405, 413, 422, 429] {
            assert!(is_explicit_send_rejection(&provider_error(status)));
        }
        for status in [408, 500, 502, 503, 504] {
            assert!(!is_explicit_send_rejection(&provider_error(status)));
        }
    }

    fn provider_error(status: u16) -> DomainError {
        DomainError {
            code: ErrorCode::HttpRejected,
            category: ErrorCategory::ProviderProtocol,
            retry: RetryAdvice::None,
            developer_message: "provider rejected request".to_owned(),
            provider_status: Some(status),
            retry_after_ms: None,
            host_kind: None,
        }
    }

    fn config() -> WalletClientConfig {
        WalletClientConfig {
            record_id: NonEmptyString::try_from("record").expect("valid record identifier"),
            address: TonAddressString::try_from(ADDRESS).expect("valid TON address"),
            public_key: vec![0; 32],
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig {
                toncenter_base_url: "https://provider.example".to_owned(),
                dns_root_address: None,
                request_timeout_ms: 15_000,
            },
        }
    }
}
