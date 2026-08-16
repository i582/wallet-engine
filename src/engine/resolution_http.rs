//! Toncenter v3 requests and parsers used by pending-send resolution.

use serde::Deserialize;
use serde_json::Value;

use crate::domain::bounded_diagnostic;
use crate::{
    Base64Hash, DomainError, ErrorCategory, ErrorCode, HttpRequest, HttpRequestId, RetryAdvice,
    UnsignedDecimalString, WalletClientConfig, WalletClientError,
};

use super::http::build_toncenter_v3_request;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExecutedMessage {
    pub(super) transaction_hash: Base64Hash,
    pub(super) transaction_lt: UnsignedDecimalString,
}

#[derive(Debug, Deserialize)]
struct TransactionsResponse {
    #[serde(default)]
    transactions: Vec<Transaction>,
}

#[derive(Debug, Deserialize)]
struct Transaction {
    hash: String,
    lt: Value,
    #[serde(default)]
    in_msg: Option<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    hash: Option<String>,
    #[serde(default)]
    hash_norm: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WalletStatesResponse {
    #[serde(default)]
    wallets: Vec<WalletState>,
}

#[derive(Debug, Deserialize)]
struct WalletState {
    #[serde(default)]
    seqno: Option<Value>,
}

/// Builds the strongest resolution lookup: a transaction containing our
/// external message as its inbound message.
pub(super) fn build_executed_by_message_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    message_hash: &Base64Hash,
) -> Result<HttpRequest, WalletClientError> {
    // The signed wallet external message is the transaction's inbound message.
    // Using `out` here would search messages emitted by the wallet transaction
    // and could leave an already executed transfer unresolved forever.
    build_toncenter_v3_request(
        config,
        id,
        "transactionsByMessage",
        &[
            ("msg_hash", message_hash.as_str()),
            ("direction", "in"),
            ("limit", "1"),
            ("offset", "0"),
        ],
    )
}

/// Builds the optional mempool lookup used to distinguish a live message from
/// one that is merely absent from indexed transactions.
pub(super) fn build_pending_transactions_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
) -> Result<HttpRequest, WalletClientError> {
    // ton-indexer exposes pending transactions by account, not by message hash.
    // The parser therefore scans this wallet-scoped response for our message.
    build_toncenter_v3_request(
        config,
        id,
        "pendingTransactions",
        &[("account", config.address.as_str())],
    )
}

/// Builds an indexed wallet-state lookup used to prove that another external
/// message consumed the persisted send's seqno.
pub(super) fn build_wallet_state_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
) -> Result<HttpRequest, WalletClientError> {
    build_toncenter_v3_request(
        config,
        id,
        "walletStates",
        &[("address", config.address.as_str())],
    )
}

/// Extracts transaction identity from a successful `transactionsByMessage`
/// response, or returns `None` when the message has not been indexed.
pub(super) fn parse_executed_message(body: &[u8]) -> Result<Option<ExecutedMessage>, DomainError> {
    let response: TransactionsResponse =
        serde_json::from_slice(body).map_err(|error| invalid_response(error.to_string()))?;
    let Some(transaction) = response.transactions.into_iter().next() else {
        return Ok(None);
    };

    Ok(Some(ExecutedMessage {
        transaction_hash: parse_hash(&transaction.hash, "transaction hash")?,
        transaction_lt: parse_u64_string(&transaction.lt, "transaction logical time")?,
    }))
}

/// Scans an account-scoped pending response for the persisted external-message
/// hash, accepting either the raw or normalized representation.
pub(super) fn parse_pending_message(
    body: &[u8],
    expected_hash: &Base64Hash,
) -> Result<bool, DomainError> {
    let response: TransactionsResponse =
        serde_json::from_slice(body).map_err(|error| invalid_response(error.to_string()))?;

    for message in response
        .transactions
        .into_iter()
        .filter_map(|transaction| transaction.in_msg)
    {
        // ton-indexer can index either the serialized hash or its normalized
        // counterpart. Accepting both avoids a false absence caused solely by
        // representation differences in external-message serialization.
        for candidate in [message.hash.as_deref(), message.hash_norm.as_deref()]
            .into_iter()
            .flatten()
        {
            if Base64Hash::try_from(candidate).is_ok_and(|hash| hash == *expected_hash) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Parses the indexed seqno while accepting Toncenter's observed numeric and
/// decimal-string JSON shapes.
pub(super) fn parse_wallet_seqno(body: &[u8]) -> Result<u32, DomainError> {
    let response: WalletStatesResponse =
        serde_json::from_slice(body).map_err(|error| invalid_response(error.to_string()))?;
    let wallet = response
        .wallets
        .into_iter()
        .next()
        .ok_or_else(|| invalid_response("walletStates did not return the configured wallet"))?;
    let seqno = wallet
        .seqno
        .ok_or_else(|| invalid_response("walletStates did not include seqno"))?;
    parse_u32(&seqno, "wallet seqno")
}

/// Validates that provider evidence contains a canonical 256-bit hash.
fn parse_hash(value: &str, field: &str) -> Result<Base64Hash, DomainError> {
    Base64Hash::try_from(value).map_err(|_| invalid_response(format!("invalid {field}")))
}

/// Parses a provider integer that may be encoded as JSON number or string.
fn parse_u32(value: &Value, field: &str) -> Result<u32, DomainError> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| invalid_response(format!("invalid {field}"))),
        Value::String(value) => value
            .parse::<u32>()
            .map_err(|_| invalid_response(format!("invalid {field}"))),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => {
            Err(invalid_response(format!("invalid {field}")))
        }
    }
}

/// Normalizes provider logical time to the public decimal-string form without
/// losing precision in language bindings.
fn parse_u64_string(value: &Value, field: &str) -> Result<UnsignedDecimalString, DomainError> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .map(UnsignedDecimalString::from)
            .ok_or_else(|| invalid_response(format!("invalid {field}"))),
        Value::String(value) => value
            .parse::<u64>()
            .map(UnsignedDecimalString::from)
            .map_err(|_| invalid_response(format!("invalid {field}"))),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => {
            Err(invalid_response(format!("invalid {field}")))
        }
    }
}

/// Maps malformed v3 evidence into the same bounded provider-protocol error
/// contract used by the rest of the engine.
fn invalid_response(message: impl Into<String>) -> DomainError {
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
    use super::*;

    const ZERO_HASH: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const ONE_HASH: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";

    #[test]
    fn parses_confirmed_transaction_evidence() {
        let evidence = parse_executed_message(
            format!(r#"{{"transactions":[{{"hash":"{ONE_HASH}","lt":"42"}}]}}"#).as_bytes(),
        )
        .expect("valid v3 response")
        .expect("one transaction");

        assert_eq!(evidence.transaction_hash.as_str(), ONE_HASH);
        assert_eq!(evidence.transaction_lt, UnsignedDecimalString::from(42_u64));
        assert_eq!(parse_executed_message(br#"{"transactions":[]}"#), Ok(None));
    }

    #[test]
    fn pending_lookup_matches_raw_or_normalized_inbound_hash_only() {
        let expected = Base64Hash::try_from(ZERO_HASH).expect("fixture hash");
        let matching = format!(
            r#"{{"transactions":[{{"hash":"{ONE_HASH}","lt":"1","in_msg":{{"hash_norm":"{ZERO_HASH}"}}}}]}}"#
        );
        assert_eq!(
            parse_pending_message(matching.as_bytes(), &expected),
            Ok(true)
        );

        let missing = format!(
            r#"{{"transactions":[{{"hash":"{ONE_HASH}","lt":"1","in_msg":{{"hash":"{ONE_HASH}"}}}}]}}"#
        );
        assert_eq!(
            parse_pending_message(missing.as_bytes(), &expected),
            Ok(false)
        );
    }

    #[test]
    fn parses_wallet_seqno_from_supported_v3_shapes() {
        assert_eq!(parse_wallet_seqno(br#"{"wallets":[{"seqno":7}]}"#), Ok(7));
        assert_eq!(parse_wallet_seqno(br#"{"wallets":[{"seqno":"8"}]}"#), Ok(8));
    }
}
