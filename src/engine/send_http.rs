//! Toncenter JSON-RPC requests and responses used by send.

use serde_json::Value;

use crate::diagnostic::bounded_diagnostic;
use crate::{
    DomainError, ErrorCategory, ErrorCode, HttpCall, HttpCallId, HttpHeader, HttpMethod,
    RetryAdvice, WalletClientConfig, WalletClientError,
};

use super::http::build_toncenter_request;

pub(super) enum SendBocResponse {
    Accepted,
    Rejected(String),
}

pub(super) fn build_seqno_request(
    config: &WalletClientConfig,
    id: HttpCallId,
) -> Result<HttpCall, WalletClientError> {
    build_json_rpc_request(
        config,
        id,
        "runGetMethod",
        serde_json::json!({
            "address": config.address,
            "method": "seqno",
            "stack": []
        }),
    )
}

pub(super) fn build_send_boc_request(
    config: &WalletClientConfig,
    id: HttpCallId,
    boc: &[u8],
) -> Result<HttpCall, WalletClientError> {
    use base64::Engine as _;

    let encoded = base64::engine::general_purpose::STANDARD.encode(boc);

    build_json_rpc_request(config, id, "sendBoc", serde_json::json!({ "boc": encoded }))
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
        .filter(|items| items.len() == 2 && items[0].as_str() == Some("num"))
        .and_then(|items| items[1].as_str())
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

fn build_json_rpc_request(
    config: &WalletClientConfig,
    id: HttpCallId,
    method: &str,
    params: Value,
) -> Result<HttpCall, WalletClientError> {
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.value.to_string(),
        "method": method,
        "params": params,
    }))
    .map_err(|_| WalletClientError::StateUnavailable)?;

    let mut call = build_toncenter_request(config, id, "jsonRPC", &[])?;

    call.method = HttpMethod::Post;
    call.headers.push(HttpHeader {
        name: "Content-Type".to_owned(),
        value: "application/json".to_owned(),
    });
    call.body = body;

    Ok(call)
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
