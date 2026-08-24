//! Toncenter response parsing and domain-error normalization.
//!
//! The wallet client uses this private module to convert provider JSON into
//! stable account and activity records. It also sanitizes external diagnostics.

use num_bigint::BigUint;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use std::str::FromStr;
use ton::block_tlb::{CommonMsgInfo, Msg as TonMessage, Tx, TxDescr};
use ton::tep::snake_data::SnakeData;
use ton::ton_core::cell::TonCell;
use ton::ton_core::traits::tlb::TLB;
use ton::ton_core::types::TonAddress;

use crate::domain::bounded_diagnostic;
use crate::{
    AccountSnapshot, AccountStatus, ActivityCursor, ActivityDirection, ActivityItem,
    ActivityStatus, Base64Hash, Boc, DomainError, ErrorCategory, ErrorCode, HttpHeader, Network,
    RetryAdvice, TonAddressString,
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
    sync_utime: u64,
}

#[derive(Debug, Deserialize)]
struct Transaction {
    utime: u64,
    data: String,
    fee: Value,
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
    /// The stable row identifier.
    /// It combines the transaction hash, direction, and deterministic message index.
    pub id: String,

    /// The validated transaction hash in standard padded Base64.
    pub transaction_hash: Base64Hash,

    /// The transaction logical time as an arbitrary-precision integer.
    /// Activity sorting uses this value before the timestamp and row identifier.
    pub logical_time: BigUint,

    /// The transaction Unix timestamp from Toncenter.
    /// This is the transaction time, not the creation time of one outgoing message.
    pub timestamp: u64,

    /// The value direction relative to the configured wallet address.
    pub direction: ActivityDirection,

    /// The exact message value in nanograms.
    /// This amount excludes transaction fees. The parser removes zero-value messages before it creates a record.
    pub amount_nanograms: BigUint,

    /// The total transaction fee in nanograms.
    /// Multi-message transactions repeat this value on each public row.
    pub transaction_fee_nanograms: BigUint,

    /// The transaction and message outcome parsed from the raw on-chain data.
    pub status: ActivityStatus,

    /// A zero-opcode plaintext comment decoded from the message body.
    pub comment: Option<String>,

    /// A complete encrypted-comment body retained for explicit decryption.
    pub encrypted_comment: Option<Boc>,

    /// The source of a received transfer or the destination of a sent transfer.
    /// The Toncenter parser rejects a nonzero transfer when this address is absent or invalid.
    /// The optional form permits future providers that cannot supply a counterparty.
    pub counterparty: Option<TonAddress>,
}

impl ActivityRecord {
    /// Creates the portable public representation used by generated bindings.
    ///
    /// The public wrapper remains numeric inside Rust and becomes a decimal
    /// string only when Serde or a generated language binding lowers it.
    pub(crate) fn snapshot(&self, network: Network) -> ActivityItem {
        ActivityItem {
            id: self.id.clone(),
            transaction_hash: self.transaction_hash.clone(),
            logical_time: (&self.logical_time).into(),
            timestamp: self.timestamp,
            direction: self.direction,
            amount_nanograms: (&self.amount_nanograms).into(),
            transaction_fee_nanograms: (&self.transaction_fee_nanograms).into(),
            status: self.status,
            comment: self.comment.clone(),
            encrypted_comment: self.encrypted_comment.clone(),
            counterparty: self
                .counterparty
                .as_ref()
                .map(|address| TonAddressString::from_address(address, network)),
        }
    }
}

/// Internal pagination cursor with a numeric logical time.
#[derive(Debug, Clone)]
pub(crate) struct ActivityPageCursor {
    /// The logical time of the oldest raw transaction in the provider page.
    /// A transaction can produce no activity item, so this value does not come from the last visible row.
    pub logical_time: BigUint,

    /// The matching transaction hash.
    /// The validated transaction hash in standard padded Base64.
    pub hash: Base64Hash,
}

impl ActivityPageCursor {
    /// Converts the internal cursor to its portable public representation.
    pub(crate) fn snapshot(&self) -> ActivityCursor {
        ActivityCursor {
            logical_time: (&self.logical_time).into(),
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
    let balance_nanograms = parse_unsigned_decimal(&account.balance, "account balance")?.into();

    let status = match account.state.as_str() {
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
            Ok(ActivityPageCursor {
                logical_time: parse_unsigned_decimal(
                    &transaction.transaction_id.lt,
                    "logical time",
                )?,
                hash: parse_hash(&transaction.transaction_id.hash, "transaction hash")?,
            })
        })
        .transpose()?;

    let raw_count = transactions.len();
    let mut items = Vec::new();

    for transaction in transactions {
        let chain_data = parse_transaction_chain_data(&transaction)?;
        let transaction_fee_nanograms =
            parse_unsigned_decimal(&transaction.fee, "transaction fee")?;

        if let Some(message) = &transaction.in_msg
            && let Some(item) = activity_from_message(
                &transaction,
                message,
                chain_data.in_msg.as_ref(),
                &transaction_fee_nanograms,
                chain_data.aborted,
                ActivityDirection::Received,
                0,
            )?
        {
            items.push(item);
        }

        let outgoing = ordered_out_messages(&transaction)?;
        for (index, ordered_message) in outgoing.into_iter().enumerate() {
            if let Some(item) = activity_from_message(
                &transaction,
                ordered_message.message,
                chain_data.out_msgs.get(ordered_message.original_index),
                &transaction_fee_nanograms,
                chain_data.aborted,
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
        has_more: usize::try_from(page_size).is_ok_and(|page_size| raw_count >= page_size),
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
    chain_message: Option<&ChainMessage>,
    transaction_fee_nanograms: &BigUint,
    transaction_aborted: bool,
    direction: ActivityDirection,
    index: usize,
) -> Result<Option<ActivityRecord>, DomainError> {
    let amount_nanograms = parse_unsigned_decimal(&message.value, "message value")?;
    if amount_nanograms == BigUint::default() {
        return Ok(None);
    }

    let chain_message = chain_message
        .ok_or_else(|| invalid_response("raw transaction message does not match provider JSON"))?;

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

    let transaction_hash = parse_hash(&transaction.transaction_id.hash, "transaction hash")?;

    Ok(Some(ActivityRecord {
        id: format!("{transaction_hash}:{direction_name}:{index}"),
        transaction_hash,
        logical_time,
        timestamp: transaction.utime,
        direction,
        amount_nanograms,
        transaction_fee_nanograms: transaction_fee_nanograms.clone(),
        status: if chain_message.bounced {
            ActivityStatus::Bounced
        } else if transaction_aborted {
            ActivityStatus::Failed
        } else {
            ActivityStatus::Success
        },
        comment: chain_message.comment.clone(),
        encrypted_comment: chain_message.encrypted_comment.clone(),
        counterparty: Some(counterparty),
    }))
}

struct ChainTransaction {
    aborted: bool,
    in_msg: Option<ChainMessage>,
    out_msgs: Vec<ChainMessage>,
}

struct ChainMessage {
    bounced: bool,
    comment: Option<String>,
    encrypted_comment: Option<Boc>,
}

fn parse_transaction_chain_data(
    transaction: &Transaction,
) -> Result<ChainTransaction, DomainError> {
    let transaction = Tx::from_boc_base64(&transaction.data)
        .map_err(|error| invalid_response(format!("invalid raw transaction data: {error}")))?;
    let aborted = transaction_aborted(&transaction.descr);
    let in_msg = transaction.msgs.in_msg.as_ref().map(chain_message);
    let out_msgs = transaction
        .msgs
        .out_msgs
        .iter()
        .map(chain_message)
        .collect();

    Ok(ChainTransaction {
        aborted,
        in_msg,
        out_msgs,
    })
}

const fn transaction_aborted(description: &TxDescr) -> bool {
    match description {
        TxDescr::Ord(value) => value.aborted,
        TxDescr::Storage(_) => false,
        TxDescr::TickTock(value) => value.aborted,
        TxDescr::SplitPrepare(value) => value.aborted,
        TxDescr::SplitInstall(value) => !value.installed,
        TxDescr::MergePrepare(value) => value.aborted,
        TxDescr::MergeInstall(value) => value.aborted,
    }
}

fn chain_message(message: &TonMessage) -> ChainMessage {
    ChainMessage {
        bounced: matches!(&message.info, CommonMsgInfo::Int(info) if info.bounced),
        comment: plaintext_comment(&message.body.value),
        encrypted_comment: encrypted_comment_body(&message.body.value),
    }
}

fn encrypted_comment_body(body: &TonCell) -> Option<Boc> {
    if !crate::wallet::encrypted_comment::is_encrypted_comment_body(body) {
        return None;
    }
    Boc::try_from(body.to_boc().ok()?).ok()
}

fn plaintext_comment(body: &TonCell) -> Option<String> {
    let mut parser = body.parser();
    if parser.read_num::<u32>(32).ok()? != 0 {
        return None;
    }

    let snake = SnakeData::read(&mut parser).ok()?;
    Some(String::from_utf8_lossy(snake.as_slice()).into_owned())
}

struct OrderedMessage<'a> {
    created_logical_time: Option<BigUint>,
    /// The optional provider message hash in standard padded Base64.
    hash: Option<Base64Hash>,
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
                .map(|hash| parse_hash(hash, "message hash"))
                .transpose()?;

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

pub(super) fn decode_envelope<T: DeserializeOwned>(body: &[u8]) -> Result<T, DomainError> {
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

pub(crate) fn invalid_response(message: impl Into<String>) -> DomainError {
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
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => {
            return Err(invalid_response(format!("{field} is not a decimal value")));
        }
    };

    value
        .parse()
        .map_err(|_| invalid_response(format!("{field} is not an unsigned decimal")))
}

fn message_address(value: &Value, field: &str) -> Result<TonAddress, DomainError> {
    let address = value
        .as_str()
        .or_else(|| value.get("account_address").and_then(Value::as_str))
        .filter(|address| !address.is_empty())
        .ok_or_else(|| invalid_response(format!("{field} is missing or invalid")))?;

    TonAddress::from_str(address)
        .map_err(|_| invalid_response(format!("{field} is missing or invalid")))
}

fn parse_hash(value: &str, field: &str) -> Result<Base64Hash, DomainError> {
    Base64Hash::try_from(value)
        .map_err(|_| invalid_response(format!("{field} is not a 256-bit Base64 value")))
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

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use num_bigint::BigUint;
    use serde_json::{Value, json};
    use ton::block_tlb::{CommonMsgInfoInt, Msg as TonMessage, Tx, TxDescr, TxDescrOrd, TxMsgs};
    use ton::tep::snake_data::SnakeData;
    use ton::ton_core::cell::TonCell;
    use ton::ton_core::traits::tlb::TLB;

    use super::{parse_account, parse_activity, response_error};
    use crate::{
        AccountStatus, ActivityDirection, ActivityStatus, ErrorCategory, ErrorCode, HttpHeader,
        RetryAdvice, UnsignedDecimalString,
    };

    const ADDRESS: &str = "0:0000000000000000000000000000000000000000000000000000000000000001";

    #[test]
    fn parses_every_account_status_and_numeric_balance_shape() {
        let cases = [
            ("nonexist", AccountStatus::Nonexistent),
            ("nonexistent", AccountStatus::Nonexistent),
            ("uninit", AccountStatus::Uninitialized),
            ("uninitialized", AccountStatus::Uninitialized),
            ("active", AccountStatus::Active),
            ("frozen", AccountStatus::Frozen),
            ("future-state", AccountStatus::Unknown),
        ];

        for (state, expected) in cases {
            let body = json!({
                "ok": true,
                "result": { "balance": 42, "state": state, "sync_utime": 123 }
            });
            let account = parse_account(&encode(body)).expect("account must parse");
            assert_eq!(
                account.balance_nanograms,
                UnsignedDecimalString::from(42_u64)
            );
            assert_eq!(account.status, expected);
            assert_eq!(account.sync_utime, 123);
        }
    }

    #[test]
    fn account_status_is_case_sensitive() {
        let body = json!({
            "ok": true,
            "result": { "balance": "42", "state": "FROZEN", "sync_utime": 123 }
        });

        let account = parse_account(&encode(body)).expect("the account envelope must parse");

        assert_eq!(account.status, AccountStatus::Unknown);
    }

    #[test]
    fn normalizes_http_and_envelope_rejections() {
        let limited =
            response_error(429, &[], br#"{"message":"slow down"}"#).expect("429 must be an error");
        assert_eq!(limited.code, ErrorCode::RateLimited);
        assert_eq!(limited.retry, RetryAdvice::Safe);
        assert_eq!(limited.retry_after_ms, None);

        let rejected = response_error(400, &[], b"not-json").expect("400 must be an error");
        assert_eq!(rejected.code, ErrorCode::HttpRejected);
        assert_eq!(rejected.retry, RetryAdvice::None);
        assert_eq!(rejected.developer_message, "HTTP 400");

        let server = response_error(
            503,
            &[HttpHeader {
                name: "Retry-After".to_owned(),
                value: "invalid".to_owned(),
            }],
            br#"{"description":"temporarily unavailable"}"#,
        )
        .expect("503 must be an error");
        assert_eq!(server.retry, RetryAdvice::Safe);
        assert_eq!(server.developer_message, "temporarily unavailable");

        let envelope = parse_account(&encode(json!({
            "ok": false,
            "error": -32005,
            "code": "429"
        })))
        .expect_err("provider envelope must fail");
        assert_eq!(envelope.code, ErrorCode::RateLimited);
        assert_eq!(envelope.category, ErrorCategory::RateLimit);
        assert_eq!(envelope.developer_message, "-32005");

        let fallback = parse_account(&encode(json!({ "ok": false })))
            .expect_err("empty provider envelope must fail");
        assert_eq!(fallback.developer_message, "provider rejected request");
    }

    #[test]
    fn provider_server_envelope_is_retryable() {
        let error = parse_account(&encode(json!({
            "ok": false,
            "code": 503,
            "error": { "message": "provider unavailable" }
        })))
        .expect_err("a provider server envelope must fail");

        assert_eq!(error.code, ErrorCode::HttpRejected);
        assert_eq!(error.category, ErrorCategory::ProviderProtocol);
        assert_eq!(error.retry, RetryAdvice::Safe);
        assert_eq!(error.provider_status, Some(503));
        assert_eq!(error.developer_message, "provider rejected request");
    }

    #[test]
    fn rate_limit_retry_after_is_converted_to_milliseconds() {
        let error = response_error(
            429,
            &[HttpHeader {
                name: "retry-after".to_owned(),
                value: "7".to_owned(),
            }],
            br#"{"error":"slow down"}"#,
        )
        .expect("429 must be an error");

        assert_eq!(error.code, ErrorCode::RateLimited);
        assert_eq!(error.retry, RetryAdvice::AfterDelay);
        assert_eq!(error.retry_after_ms, Some(7_000));
        assert_eq!(error.developer_message, "slow down");
    }

    #[test]
    fn rejects_non_decimal_json_types_before_publishing_provider_data() {
        let account = parse_account(&encode(json!({
            "ok": true,
            "result": { "balance": true, "state": "active", "sync_utime": 1 }
        })))
        .expect_err("a boolean balance must fail");
        assert_eq!(account.code, ErrorCode::InvalidProviderResponse);
        assert_eq!(
            account.developer_message,
            "account balance is not a decimal value"
        );

        let activity = parse_activity(
            &encode(json!({
                "ok": true,
                "result": [{
                    "utime": 1,
                    "data": transaction_data(false, Some(empty_message(false)), Vec::new()),
                    "fee": "0",
                    "transaction_id": { "lt": {}, "hash": hash(1) },
                    "in_msg": { "source": ADDRESS, "value": "1" }
                }]
            })),
            10,
        )
        .expect_err("an object logical time must fail");
        assert_eq!(activity.code, ErrorCode::InvalidProviderResponse);
        assert_eq!(
            activity.developer_message,
            "logical time is not a decimal value"
        );
    }

    #[test]
    fn rejects_an_account_response_without_sync_utime() {
        let error = parse_account(&encode(json!({
            "ok": true,
            "result": { "balance": "1", "state": "active" }
        })))
        .expect_err("Toncenter v2 must provide sync_utime");

        assert_eq!(error.code, ErrorCode::InvalidProviderResponse);
        assert_eq!(error.developer_message, "missing field `sync_utime`");
    }

    #[test]
    fn activity_uses_bigints_and_stable_outgoing_order() {
        let transaction_hash = hash(9);
        let body = json!({
            "ok": true,
            "result": [{
                "utime": 123,
                "data": transaction_data(
                    false,
                    None,
                    vec![empty_message(false), empty_message(false), empty_message(false)],
                ),
                "fee": "42",
                "transaction_id": {
                    "lt": "340282366920938463463374607431768211456",
                    "hash": transaction_hash,
                },
                "in_msg": { "source": ADDRESS, "value": "0" },
                "out_msgs": [
                    {
                        "hash": hash(3),
                        "created_lt": "30",
                        "destination": { "account_address": ADDRESS },
                        "value": "300000000000000000000000000000000000000"
                    },
                    {
                        "hash": hash(1),
                        "created_lt": "10",
                        "destination": ADDRESS,
                        "value": 100
                    },
                    {
                        "hash": hash(2),
                        "created_lt": "20",
                        "destination": ADDRESS,
                        "value": "200"
                    }
                ]
            }]
        });

        let page = parse_activity(&encode(body), 1).expect("activity must parse");
        assert!(page.has_more);
        assert_eq!(page.items.len(), 3);
        assert_eq!(
            page.items
                .iter()
                .map(|item| item.direction)
                .collect::<Vec<_>>(),
            vec![ActivityDirection::Sent; 3]
        );
        assert_eq!(
            page.items
                .iter()
                .map(|item| item.amount_nanograms.to_string())
                .collect::<Vec<_>>(),
            ["100", "200", "300000000000000000000000000000000000000"]
        );
        assert!(
            page.items
                .iter()
                .all(|item| item.transaction_fee_nanograms == BigUint::from(42_u8))
        );
        assert!(
            page.items
                .iter()
                .all(|item| item.status == ActivityStatus::Success && item.comment.is_none())
        );
        assert_eq!(
            page.cursor
                .expect("cursor must exist")
                .logical_time
                .to_string(),
            "340282366920938463463374607431768211456"
        );
    }

    #[test]
    fn enriches_activity_with_comments_fees_and_chain_status() {
        let long_comment = "snake-data ".repeat(20);
        let body = json!({
            "ok": true,
            "result": [
                {
                    "utime": 3,
                    "data": transaction_data(
                        false,
                        Some(comment_message(false, long_comment.as_bytes())),
                        Vec::new(),
                    ),
                    "fee": "340282366920938463463374607431768211456",
                    "transaction_id": { "lt": "3", "hash": hash(3) },
                    "in_msg": { "source": ADDRESS, "value": "30" },
                },
                {
                    "utime": 2,
                    "data": transaction_data(
                        true,
                        Some(comment_message(false, b"failed")),
                        Vec::new(),
                    ),
                    "fee": "20",
                    "transaction_id": { "lt": "2", "hash": hash(2) },
                    "in_msg": { "source": ADDRESS, "value": "20" },
                },
                {
                    "utime": 1,
                    "data": transaction_data(
                        true,
                        Some(comment_message(true, &[0xff])),
                        Vec::new(),
                    ),
                    "fee": 10,
                    "transaction_id": { "lt": "1", "hash": hash(1) },
                    "in_msg": { "source": ADDRESS, "value": "10" },
                },
            ]
        });

        let page = parse_activity(&encode(body), 10).expect("enriched activity must parse");
        assert_eq!(page.items.len(), 3);

        let success = &page.items[0];
        assert_eq!(success.status, ActivityStatus::Success);
        assert_eq!(success.comment.as_deref(), Some(long_comment.as_str()));
        assert_eq!(
            success.transaction_fee_nanograms.to_string(),
            "340282366920938463463374607431768211456"
        );

        let failed = &page.items[1];
        assert_eq!(failed.status, ActivityStatus::Failed);
        assert_eq!(failed.comment.as_deref(), Some("failed"));

        let bounced = &page.items[2];
        assert_eq!(bounced.status, ActivityStatus::Bounced);
        assert_eq!(bounced.comment.as_deref(), Some("�"));
    }

    #[test]
    fn retains_encrypted_comment_bodies_for_explicit_decryption() {
        let body = json!({
            "ok": true,
            "result": [{
                "utime": 1,
                "data": transaction_data(
                    false,
                    Some(encrypted_comment_message(false)),
                    Vec::new(),
                ),
                "fee": "1",
                "transaction_id": { "lt": "1", "hash": hash(9) },
                "in_msg": { "source": ADDRESS, "value": "10" },
            }]
        });

        let page = parse_activity(&encode(body), 10).expect("encrypted activity must parse");
        let item = page.items.first().expect("one activity item");
        assert!(item.comment.is_none());
        let encrypted = item
            .encrypted_comment
            .as_ref()
            .expect("encrypted body must be retained");
        let cell = TonCell::from_boc(encrypted.as_bytes().to_vec()).expect("body BOC parses");
        let mut parser = cell.parser();
        assert_eq!(parser.read_num::<u32>(32).expect("opcode"), 0x2167_da4b);
    }

    #[test]
    fn rejects_missing_results_and_invalid_transfer_fields() {
        let missing =
            parse_account(&encode(json!({ "ok": true }))).expect_err("missing result must fail");
        assert_eq!(missing.code, ErrorCode::InvalidProviderResponse);

        for mut transaction in [
            json!({
                "utime": 1,
                "transaction_id": { "lt": "1", "hash": "not-a-hash" },
                "in_msg": { "source": ADDRESS, "value": "1" }
            }),
            json!({
                "utime": 1,
                "transaction_id": { "lt": "1", "hash": hash(1) },
                "in_msg": { "source": null, "value": "1" }
            }),
            json!({
                "utime": 1,
                "transaction_id": { "lt": "1", "hash": hash(1) },
                "in_msg": { "source": ADDRESS, "value": -1 }
            }),
            json!({
                "utime": 1,
                "transaction_id": { "lt": "1", "hash": hash(1) },
                "out_msgs": [{
                    "hash": "bad",
                    "created_lt": "1",
                    "destination": ADDRESS,
                    "value": "1"
                }]
            }),
        ] {
            transaction["data"] = json!(transaction_data(
                false,
                Some(empty_message(false)),
                vec![empty_message(false)],
            ));
            transaction["fee"] = json!("0");
            let error = parse_activity(&encode(json!({ "ok": true, "result": [transaction] })), 10)
                .expect_err("invalid transfer field must fail");
            assert_eq!(error.code, ErrorCode::InvalidProviderResponse);
            assert_eq!(error.retry, RetryAdvice::None);
        }
    }

    fn hash(byte: u8) -> String {
        STANDARD.encode([byte; 32])
    }

    fn encode(value: Value) -> Vec<u8> {
        serde_json::to_vec(&value).expect("test JSON must serialize")
    }

    fn transaction_data(
        aborted: bool,
        in_msg: Option<TonMessage>,
        out_msgs: Vec<TonMessage>,
    ) -> String {
        Tx {
            msgs: TxMsgs { in_msg, out_msgs }.into(),
            descr: TxDescr::Ord(TxDescrOrd {
                aborted,
                ..TxDescrOrd::default()
            })
            .into(),
            ..Tx::default()
        }
        .to_boc_base64()
        .expect("raw transaction fixture must serialize")
    }

    fn empty_message(bounced: bool) -> TonMessage {
        let body = TonCell::builder()
            .build()
            .expect("empty body fixture must build");
        message(bounced, body)
    }

    fn comment_message(bounced: bool, comment: &[u8]) -> TonMessage {
        let mut body = TonCell::builder();
        body.write_bits([0_u8; 4], 32)
            .expect("comment opcode fixture must write");
        SnakeData::new(comment.to_vec())
            .write(&mut body)
            .expect("comment snake fixture must write");
        let body = body.build().expect("comment body fixture must build");
        message(bounced, body)
    }

    fn encrypted_comment_message(bounced: bool) -> TonMessage {
        let mut body = TonCell::builder();
        body.write_bits(0x2167_da4b_u32.to_be_bytes(), 32)
            .expect("encrypted-comment opcode fixture must write");
        SnakeData::new(vec![0_u8; 64])
            .write(&mut body)
            .expect("encrypted-comment snake fixture must write");
        let body = body
            .build()
            .expect("encrypted-comment body fixture must build");
        message(bounced, body)
    }

    fn message(bounced: bool, body: TonCell) -> TonMessage {
        TonMessage::new(
            CommonMsgInfoInt {
                bounced,
                ..CommonMsgInfoInt::default()
            },
            body,
        )
    }
}
