//! Toncenter response parsing and domain-error normalization.
//!
//! The wallet client uses this private module to convert provider JSON into
//! stable account and activity records. It also sanitizes external diagnostics.

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use num_bigint::BigUint;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;

use crate::diagnostic::bounded_diagnostic;
use crate::{
    AccountSnapshot, AccountStatus, ActivityCursor, ActivityDirection, ActivityItem, DomainError,
    ErrorCategory, ErrorCode, HttpHeader, RetryAdvice,
};

#[derive(Debug, Deserialize)]
struct RawEnvelope {
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
    /// The hash representation returned by Toncenter before normalization.
    hash: String,
}

#[derive(Debug, Deserialize)]
struct Message {
    /// The optional hash representation returned by Toncenter before normalization.
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

/// Parsed activity kept inside Rust before it crosses an FFI boundary.
///
/// Logical time and value remain arbitrary-precision integers here. This
/// prevents ordering and arithmetic from depending on decimal-string rules.
#[derive(Debug, Clone)]
pub(crate) struct ActivityRecord {
    pub id: String,
    pub transaction_hash: String,
    pub logical_time: BigUint,
    pub timestamp: u64,
    pub direction: ActivityDirection,
    pub amount_nanograms: BigUint,
    pub counterparty: Option<String>,
}

impl ActivityRecord {
    /// Creates the portable public representation used by generated bindings.
    ///
    /// Swift and Kotlin have no shared arbitrary-precision integer ABI with
    /// Rust, so conversion to canonical decimal strings happens only here.
    pub(crate) fn snapshot(&self) -> ActivityItem {
        ActivityItem {
            id: self.id.clone(),
            transaction_hash: self.transaction_hash.clone(),
            logical_time: self.logical_time.to_string(),
            timestamp: self.timestamp,
            direction: self.direction,
            amount_nanograms: self.amount_nanograms.to_string(),
            counterparty: self.counterparty.clone(),
        }
    }
}

/// Internal pagination cursor with a numeric logical time.
#[derive(Debug, Clone)]
pub(crate) struct ActivityPageCursor {
    pub logical_time: BigUint,
    pub hash: String,
}

impl ActivityPageCursor {
    /// Converts the internal cursor to its portable public representation.
    pub(crate) fn snapshot(&self) -> ActivityCursor {
        ActivityCursor {
            logical_time: self.logical_time.to_string(),
            hash: self.hash.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ActivityPage {
    pub items: Vec<ActivityRecord>,
    pub cursor: Option<ActivityPageCursor>,
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
    let balance_nanograms =
        parse_unsigned_decimal(&account.balance, "account balance")?.to_string();

    let status = match account.state.to_ascii_lowercase().as_str() {
        "nonexist" | "nonexistent" => AccountStatus::Nonexistent,
        "uninit" | "uninitialized" => AccountStatus::Uninitialized,
        "active" => AccountStatus::Active,
        "frozen" => AccountStatus::Frozen,
        _ => AccountStatus::Unknown,
    };

    Ok(AccountSnapshot {
        balance_nanograms,
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
            Ok(ActivityPageCursor {
                logical_time: parse_unsigned_decimal(
                    &transaction.transaction_id.lt,
                    "logical time",
                )?,
                hash: canonical_hash(&transaction.transaction_id.hash),
            })
        })
        .transpose()?;

    let raw_count = transactions.len();
    let mut items = Vec::new();

    for transaction in transactions {
        if let Some(message) = &transaction.in_msg
            && let Some(item) =
                activity_from_message(&transaction, message, ActivityDirection::Received, 0)?
        {
            items.push(item);
        }

        let outgoing = ordered_out_messages(&transaction)?;
        for (index, ordered_message) in outgoing.into_iter().enumerate() {
            if let Some(item) = activity_from_message(
                &transaction,
                ordered_message.message,
                ActivityDirection::Sent,
                index,
            )? {
                items.push(item);
            }
        }
    }

    items.sort_by(activity_record_order);

    Ok(ActivityPage {
        items,
        cursor,
        has_more: raw_count >= page_size as usize,
    })
}

pub(crate) fn activity_record_order(
    left: &ActivityRecord,
    right: &ActivityRecord,
) -> std::cmp::Ordering {
    right
        .logical_time
        .cmp(&left.logical_time)
        .then_with(|| right.timestamp.cmp(&left.timestamp))
        .then_with(|| left.id.cmp(&right.id))
}

fn activity_from_message(
    transaction: &Transaction,
    message: &Message,
    direction: ActivityDirection,
    index: usize,
) -> Result<Option<ActivityRecord>, DomainError> {
    let amount_nanograms = parse_unsigned_decimal(&message.value, "message value")?;
    if amount_nanograms == BigUint::default() {
        return Ok(None);
    }

    // A zero-value service message is not wallet activity and does not need a
    // counterparty. Every value transfer must still provide a valid address.
    let counterparty = match direction {
        ActivityDirection::Received => message_address(&message.source, "message source")?,
        ActivityDirection::Sent => message_address(&message.destination, "message destination")?,
    };

    let logical_time = parse_unsigned_decimal(&transaction.transaction_id.lt, "logical time")?;
    let direction_name = match direction {
        ActivityDirection::Received => "received",
        ActivityDirection::Sent => "sent",
    };

    let transaction_hash = canonical_hash(&transaction.transaction_id.hash);
    if transaction_hash.is_empty() {
        return Err(invalid_response("transaction hash must not be empty"));
    }

    Ok(Some(ActivityRecord {
        id: format!("{transaction_hash}:{direction_name}:{index}"),
        transaction_hash,
        logical_time,
        timestamp: transaction.utime,
        direction,
        amount_nanograms,
        counterparty: Some(counterparty),
    }))
}

struct OrderedMessage<'a> {
    created_logical_time: Option<BigUint>,
    /// The provider message hash normalized to standard padded Base64 when valid.
    hash: String,
    original_index: usize,
    message: &'a Message,
}

fn ordered_out_messages(transaction: &Transaction) -> Result<Vec<OrderedMessage<'_>>, DomainError> {
    let mut messages = transaction
        .out_msgs
        .iter()
        .enumerate()
        .map(|(original_index, message)| {
            let created_lt = message
                .created_lt
                .as_ref()
                .map(|value| parse_unsigned_decimal(value, "message created logical time"))
                .transpose()?;

            let message_hash = message
                .hash
                .as_deref()
                .filter(|hash| !hash.is_empty())
                .map(canonical_hash)
                .unwrap_or_default();

            Ok(OrderedMessage {
                created_logical_time: created_lt,
                hash: message_hash,
                original_index,
                message,
            })
        })
        .collect::<Result<Vec<_>, DomainError>>()?;

    messages.sort_by(|left, right| {
        left.created_logical_time
            .cmp(&right.created_logical_time)
            .then_with(|| left.hash.cmp(&right.hash))
            .then_with(|| left.original_index.cmp(&right.original_index))
    });

    Ok(messages)
}

fn decode_envelope<T: DeserializeOwned>(body: &[u8]) -> Result<T, DomainError> {
    let envelope: RawEnvelope =
        serde_json::from_slice(body).map_err(|error| invalid_response(error.to_string()))?;
    if !envelope.ok {
        return Err(provider_envelope_error(envelope));
    }

    let result = envelope
        .result
        .ok_or_else(|| invalid_response("missing provider result"))?;
    serde_json::from_value(result).map_err(|error| invalid_response(error.to_string()))
}

fn provider_envelope_error(envelope: RawEnvelope) -> DomainError {
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
            developer_message: bounded_diagnostic(developer_message),
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
        developer_message: bounded_diagnostic(developer_message),
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
        developer_message: bounded_diagnostic(message.into()),
        provider_status: None,
        retry_after_ms: None,
        host_kind: None,
    }
}

/// Parses Toncenter numbers without imposing a machine-integer size limit.
///
/// Toncenter can encode the same integer as a JSON string or number. Both
/// forms become one canonical numeric representation before engine logic runs.
fn parse_unsigned_decimal(value: &Value, field: &str) -> Result<BigUint, DomainError> {
    let value = match value {
        Value::String(value) => value.as_str(),
        Value::Number(value) => {
            return value
                .to_string()
                .parse()
                .map_err(|_| invalid_response(format!("{field} is not an unsigned decimal")));
        }
        _ => return Err(invalid_response(format!("{field} is not a decimal value"))),
    };

    value
        .parse()
        .map_err(|_| invalid_response(format!("{field} is not an unsigned decimal")))
}

fn message_address(value: &Value, field: &str) -> Result<String, DomainError> {
    value
        .as_str()
        .or_else(|| value.get("account_address").and_then(Value::as_str))
        .filter(|address| !address.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid_response(format!("{field} is missing or invalid")))
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
        .map(bounded_diagnostic)
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
