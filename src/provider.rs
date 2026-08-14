//! Toncenter response parsing and domain-error normalization.
//!
//! The wallet client uses this private module to convert provider JSON into
//! stable account and activity records. It also sanitizes external diagnostics.

use std::cmp::Ordering;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    AccountSnapshot, AccountStatus, ActivityCursor, ActivityDirection, ActivityItem, DomainError,
    ErrorCategory, ErrorCode, HttpHeader, RetryAdvice,
};

#[derive(Debug, Deserialize)]
struct Envelope {
    ok: bool,
    result: Option<Value>,
    error: Option<Value>,
    description: Option<String>,
    code: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct Account {
    balance: Value,
    #[serde(default)]
    state: String,
    sync_utime: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Transaction {
    utime: u64,
    transaction_id: TransactionId,
    #[serde(default)]
    in_msg: Option<Message>,
    #[serde(default)]
    out_msgs: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct TransactionId {
    lt: Value,
    hash: String,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    hash: Option<String>,
    #[serde(default)]
    created_lt: Option<Value>,
    #[serde(default)]
    source: Value,
    #[serde(default)]
    destination: Value,
    #[serde(default)]
    value: Value,
}

#[derive(Debug)]
pub(crate) struct ActivityPage {
    pub items: Vec<ActivityItem>,
    pub cursor: Option<ActivityCursor>,
    pub has_more: bool,
}

pub(crate) fn response_error(
    status: u16,
    headers: &[HttpHeader],
    body: &[u8],
) -> Option<DomainError> {
    if (200..300).contains(&status) {
        return None;
    }
    let developer_message = provider_message(body).unwrap_or_else(|| format!("HTTP {status}"));
    if status == 429 {
        let retry_after_ms = parse_retry_after_ms(headers);
        return Some(DomainError {
            code: ErrorCode::RateLimited,
            category: ErrorCategory::RateLimit,
            retry: if retry_after_ms.is_some() {
                RetryAdvice::AfterDelay
            } else {
                RetryAdvice::Safe
            },
            developer_message,
            provider_status: Some(status),
            retry_after_ms,
            host_kind: None,
        });
    }
    Some(DomainError {
        code: ErrorCode::HttpRejected,
        category: ErrorCategory::ProviderProtocol,
        retry: if status >= 500 {
            RetryAdvice::Safe
        } else {
            RetryAdvice::None
        },
        developer_message,
        provider_status: Some(status),
        retry_after_ms: None,
        host_kind: None,
    })
}

pub(crate) fn parse_account(body: &[u8]) -> Result<AccountSnapshot, DomainError> {
    let account: Account = decode_envelope(body)?;
    let balance_nanograms = decimal_string(&account.balance, "account balance")?;
    let balance_grams = format_nanograms(&balance_nanograms)?;
    let status = match account.state.to_ascii_lowercase().as_str() {
        "nonexist" | "nonexistent" => AccountStatus::Nonexistent,
        "uninit" | "uninitialized" => AccountStatus::Uninitialized,
        "active" => AccountStatus::Active,
        "frozen" => AccountStatus::Frozen,
        _ => AccountStatus::Unknown,
    };
    Ok(AccountSnapshot {
        balance_nanograms,
        balance_grams,
        status,
        sync_utime: account.sync_utime,
    })
}

pub(crate) fn parse_activity(body: &[u8], page_size: u32) -> Result<ActivityPage, DomainError> {
    let transactions: Vec<Transaction> = decode_envelope(body)?;
    let cursor = transactions
        .last()
        .map(|transaction| {
            if transaction.transaction_id.hash.is_empty() {
                return Err(invalid_response("transaction hash must not be empty"));
            }
            Ok(ActivityCursor {
                logical_time: decimal_string(&transaction.transaction_id.lt, "logical time")?,
                hash: canonical_hash(&transaction.transaction_id.hash),
            })
        })
        .transpose()?;
    let raw_count = transactions.len();
    let mut items = Vec::new();
    for transaction in transactions {
        if let Some(message) = &transaction.in_msg
            && message_address(&message.source).is_some()
            && let Some(item) =
                activity_from_message(&transaction, message, ActivityDirection::Received, 0)?
        {
            items.push(item);
        }
        let outgoing = ordered_out_messages(&transaction)?;
        for (index, (_, _, _, message)) in outgoing.into_iter().enumerate() {
            if let Some(item) =
                activity_from_message(&transaction, message, ActivityDirection::Sent, index)?
            {
                items.push(item);
            }
        }
    }
    items.sort_by(activity_item_order);
    items.dedup_by(|left, right| left.id == right.id);
    Ok(ActivityPage {
        items,
        cursor,
        has_more: raw_count >= page_size as usize,
    })
}

pub(crate) fn activity_item_order(left: &ActivityItem, right: &ActivityItem) -> Ordering {
    decimal_cmp(&right.logical_time, &left.logical_time)
        .then_with(|| right.timestamp.cmp(&left.timestamp))
        .then_with(|| left.id.cmp(&right.id))
}

pub(crate) fn decimal_cmp(left: &str, right: &str) -> Ordering {
    let left = left.trim_start_matches('0');
    let right = right.trim_start_matches('0');
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn activity_from_message(
    transaction: &Transaction,
    message: &Message,
    direction: ActivityDirection,
    index: usize,
) -> Result<Option<ActivityItem>, DomainError> {
    let amount_nanograms = decimal_string(&message.value, "message value")?;
    if amount_nanograms.bytes().all(|byte| byte == b'0') {
        return Ok(None);
    }
    let counterparty = match direction {
        ActivityDirection::Received => message_address(&message.source),
        ActivityDirection::Sent => message_address(&message.destination),
    };
    let logical_time = decimal_string(&transaction.transaction_id.lt, "logical time")?;
    let direction_name = match direction {
        ActivityDirection::Received => "received",
        ActivityDirection::Sent => "sent",
    };
    let transaction_hash = canonical_hash(&transaction.transaction_id.hash);
    if transaction_hash.is_empty() {
        return Err(invalid_response("transaction hash must not be empty"));
    }
    Ok(Some(ActivityItem {
        id: format!("{transaction_hash}:{direction_name}:{index}"),
        transaction_hash,
        logical_time,
        timestamp: transaction.utime,
        direction,
        amount_grams: format_nanograms(&amount_nanograms)?,
        amount_nanograms,
        counterparty,
    }))
}

type OrderedMessage<'a> = (Option<String>, String, usize, &'a Message);

fn ordered_out_messages(transaction: &Transaction) -> Result<Vec<OrderedMessage<'_>>, DomainError> {
    let mut messages = transaction
        .out_msgs
        .iter()
        .enumerate()
        .map(|(original_index, message)| {
            let created_lt = message
                .created_lt
                .as_ref()
                .map(|value| decimal_string(value, "message created logical time"))
                .transpose()?;
            let message_hash = message
                .hash
                .as_deref()
                .filter(|hash| !hash.is_empty())
                .map(canonical_hash)
                .unwrap_or_default();
            Ok((created_lt, message_hash, original_index, message))
        })
        .collect::<Result<Vec<_>, DomainError>>()?;
    messages.sort_by(|left, right| {
        optional_decimal_cmp(left.0.as_deref(), right.0.as_deref())
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    Ok(messages)
}

fn decode<'a, T: Deserialize<'a>>(body: &'a [u8]) -> Result<T, DomainError> {
    serde_json::from_slice(body).map_err(|error| invalid_response(error.to_string()))
}

fn decode_envelope<T: DeserializeOwned>(body: &[u8]) -> Result<T, DomainError> {
    let envelope: Envelope = decode(body)?;
    if !envelope.ok {
        return Err(provider_envelope_error(envelope));
    }
    let result = envelope
        .result
        .ok_or_else(|| invalid_response("missing provider result"))?;
    serde_json::from_value(result).map_err(|error| invalid_response(error.to_string()))
}

fn provider_envelope_error(envelope: Envelope) -> DomainError {
    let status = envelope.code.as_ref().and_then(provider_code);
    let developer_message = envelope
        .error
        .as_ref()
        .and_then(value_message)
        .or(envelope.description)
        .unwrap_or_else(|| "provider rejected request".to_owned());
    if status == Some(429) {
        return DomainError {
            code: ErrorCode::RateLimited,
            category: ErrorCategory::RateLimit,
            retry: RetryAdvice::Safe,
            developer_message: sanitize_diagnostic(&developer_message),
            provider_status: status,
            retry_after_ms: None,
            host_kind: None,
        };
    }
    DomainError {
        code: ErrorCode::HttpRejected,
        category: ErrorCategory::ProviderProtocol,
        retry: if status.is_some_and(|value| value >= 500) {
            RetryAdvice::Safe
        } else {
            RetryAdvice::None
        },
        developer_message: sanitize_diagnostic(&developer_message),
        provider_status: status,
        retry_after_ms: None,
        host_kind: None,
    }
}

fn provider_code(value: &Value) -> Option<u16> {
    value
        .as_u64()
        .and_then(|value| u16::try_from(value).ok())
        .or_else(|| value.as_str()?.parse().ok())
}

fn value_message(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|value| value.to_string()))
}

fn invalid_response(message: impl Into<String>) -> DomainError {
    DomainError {
        code: ErrorCode::InvalidProviderResponse,
        category: ErrorCategory::ProviderProtocol,
        retry: RetryAdvice::None,
        developer_message: sanitize_diagnostic(&message.into()),
        provider_status: None,
        retry_after_ms: None,
        host_kind: None,
    }
}

fn decimal_string(value: &Value, field: &str) -> Result<String, DomainError> {
    let value = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => return Err(invalid_response(format!("{field} is not a decimal value"))),
    };
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_response(format!(
            "{field} is not an unsigned decimal"
        )));
    }
    Ok(value)
}

fn format_nanograms(value: &str) -> Result<String, DomainError> {
    let nanograms = value
        .parse::<u128>()
        .map_err(|error| invalid_response(error.to_string()))?;
    let whole = nanograms / 1_000_000_000;
    let fraction = nanograms % 1_000_000_000;
    if fraction == 0 {
        return Ok(whole.to_string());
    }
    let fraction = format!("{fraction:09}").trim_end_matches('0').to_owned();
    Ok(format!("{whole}.{fraction}"))
}

fn message_address(value: &Value) -> Option<String> {
    value
        .as_str()
        .or_else(|| value.get("account_address").and_then(Value::as_str))
        .filter(|address| !address.is_empty())
        .map(str::to_owned)
}

fn optional_decimal_cmp(left: Option<&str>, right: Option<&str>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => decimal_cmp(left, right),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn canonical_hash(value: &str) -> String {
    let decoded = STANDARD
        .decode(value)
        .or_else(|_| STANDARD_NO_PAD.decode(value))
        .or_else(|_| URL_SAFE.decode(value))
        .or_else(|_| URL_SAFE_NO_PAD.decode(value));
    match decoded {
        Ok(bytes) if bytes.len() == 32 => STANDARD.encode(bytes),
        _ => value.to_owned(),
    }
}

fn provider_message(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    ["error", "description", "message"]
        .into_iter()
        .find_map(|field| value.get(field).and_then(Value::as_str))
        .map(sanitize_diagnostic)
}

pub(crate) fn sanitize_diagnostic(message: &str) -> String {
    message
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(512)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn parse_retry_after_ms(headers: &[HttpHeader]) -> Option<u64> {
    let seconds = headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("retry-after"))?
        .value
        .trim()
        .parse::<u64>()
        .ok()?;
    seconds.checked_mul(1_000)
}
