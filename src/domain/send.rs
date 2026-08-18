//! Public transfer requests, phases, snapshots, and results.

use std::ops::Add;

use crate::{
    Base64Hash, Boc, NonEmptyString, TonAddressString, UnsignedDecimalString,
    UnsignedDecimalStringError,
};

/// The transfer value policy applied by the wallet contract.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SendAmount {
    /// Send one exact nonnegative value and pay network fees separately.
    Exact {
        /// The exact value, in nanograms.
        nanograms: UnsignedDecimalString,
    },
    /// Send the complete remaining wallet balance after network fees.
    All,
}

impl SendAmount {
    /// Creates an exact-value transfer intent.
    ///
    /// Validation rejects signed, noncanonical, and nonnumeric values before
    /// they can become part of the transfer intent. Zero is valid.
    pub fn exact(nanograms: impl Into<String>) -> Result<Self, UnsignedDecimalStringError> {
        Ok(Self::Exact {
            nanograms: UnsignedDecimalString::try_from(nanograms.into())?,
        })
    }
}

/// The body encoded into one outgoing internal TON message.
///
/// Select `Empty` for a value-only transfer, `Comment` for a standard text
/// comment, or `RawPayload` for a pre-serialized contract call.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SendMessageBody {
    /// Encode no body bits or references.
    Empty,
    /// Encode a zero opcode followed by the UTF-8 text as TON snake data.
    Comment {
        /// The plaintext UTF-8 comment, including an intentionally empty one.
        text: String,
    },
    /// Preserve one caller-built cell as the internal-message body.
    RawPayload {
        /// The complete body cell encoded as a validated BOC.
        boc: Boc,
    },
}

/// One outgoing internal TON message before wallet-contract serialization.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendMessage {
    /// A friendly or raw TON destination address.
    pub destination: TonAddressString,
    /// The exact-value or whole-balance transfer policy.
    pub amount: SendAmount,
    /// The single body representation encoded into the message.
    pub body: SendMessageBody,
    /// Optional destination-contract `StateInit` attached independently of the body.
    pub state_init: Option<Boc>,
}

/// The policy used to select the wallet message expiration boundary.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SendExpiration {
    /// Derive expiration from fresh provider time and engine configuration.
    EngineDefault,
    /// Preserve a caller-selected Unix expiration timestamp.
    Exact {
        /// The Unix expiration timestamp in seconds.
        #[serde(rename = "unixTimestamp", alias = "unix_timestamp")]
        unix_timestamp: u64,
    },
}

/// The complete transfer intent shared by preview and signed-send operations.
///
/// Applications can preview this value and then pass the same value to a
/// signed send after the user confirms it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendIntent {
    /// The expiration policy for the wallet message.
    pub expiration: SendExpiration,
    /// The non-empty ordered outgoing internal-message batch.
    ///
    /// Wallet V5 accepts at most 255 actions. The engine validates that limit
    /// before emulation or secret authorization.
    pub messages: Vec<SendMessage>,
}

impl SendIntent {
    /// Validates wallet-level constraints and returns the sum of exact message values.
    ///
    /// `None` identifies the single whole-balance form, whose final value can
    /// only be determined by the wallet contract after fees are charged.
    pub(crate) fn exact_value_total(
        &self,
    ) -> Result<Option<UnsignedDecimalString>, SendIntentError> {
        if self.messages.is_empty() || self.messages.len() > 255 {
            return Err(SendIntentError::InvalidMessageCount);
        }
        if self.messages.len() > 1
            && self
                .messages
                .iter()
                .any(|message| matches!(message.amount, SendAmount::All))
        {
            return Err(SendIntentError::WholeBalanceInBatch);
        }

        let mut total = UnsignedDecimalString::from(0_u64);
        for message in &self.messages {
            let SendAmount::Exact { nanograms } = &message.amount else {
                return Ok(None);
            };
            total = Add::add(&total, nanograms);
        }
        Ok(Some(total))
    }
}

/// Reports a transfer batch that Wallet V5 cannot represent unambiguously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SendIntentError {
    /// Wallet V5 action lists contain between one and 255 outgoing messages.
    #[error("wallet message batch must contain between 1 and 255 messages")]
    InvalidMessageCount,
    /// Carry-all-balance mode cannot be combined with another outgoing message.
    #[error("whole-balance transfer must be the only message in its batch")]
    WholeBalanceInBatch,
}

/// Requests one signed wallet transfer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    /// A unique idempotency identifier chosen by the application.
    pub operation_id: NonEmptyString,
    /// Allows this send to replace an earlier signed send whose on-chain outcome is unresolved.
    ///
    /// The earlier send can still execute. Require explicit user confirmation
    /// before setting this flag because both transfers can affect the balance.
    #[serde(default)]
    pub force: bool,
    /// The immutable message and expiration choices for this operation.
    pub intent: SendIntent,
}

/// Requests an owner-signed Wallet V5 message for delivery by a relayer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SignMessageRequest {
    /// A unique idempotency identifier chosen by the application.
    pub operation_id: NonEmptyString,
    /// Allows this request to replace an earlier signed message whose outcome is unresolved.
    ///
    /// The earlier message remains valid until its expiration time and can win
    /// the sequence-number race. Require explicit user confirmation before use.
    #[serde(default)]
    pub force: bool,
    /// The immutable messages and expiration boundary covered by the signature.
    pub intent: SendIntent,
}

/// A durable owner-signed internal message returned to its caller for relaying.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SignMessageResult {
    /// The application operation identifier.
    pub operation_id: NonEmptyString,
    /// The complete relaxed internal message encoded as a validated BOC.
    pub internal_boc: Boc,
    /// The Unix expiration timestamp covered by the signature.
    pub valid_until: u64,
    /// The durable workflow phase after the message becomes available to the caller.
    pub phase: SendPhase,
}

/// Public transfer intent used to preview a send without unlocking its secret.
///
/// A preview uses fresh account state and a fake signature. It does not reserve
/// an operation identifier, read the send journal, or request the mnemonic.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendPreviewRequest {
    /// The same immutable intent accepted by the signed-send operation.
    pub intent: SendIntent,
}

/// An informational transfer preview produced without unlocking the wallet secret.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendPreview {
    /// The complete ordered outgoing message batch that was emulated.
    pub messages: Vec<SendMessage>,
    /// The resolved wallet message expiration timestamp used by this emulation.
    /// A signed send resolves `EngineDefault` again from fresh provider time
    /// and preserves the same timestamp for `Exact`.
    pub valid_until: u64,
    /// The complete fake-signed external message submitted for emulation.
    /// The value is a standard padded Base64-encoded BOC. Clients can pass it
    /// to an independent emulator or explorer without reconstructing the message.
    pub message_boc_base64: Boc,
    /// The bounded Toncenter emulation summary shown before authorization.
    pub emulation: SendEmulation,
}

/// A validated internal-message signing preview without submission fee claims.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SignMessagePreview {
    /// The complete ordered outgoing message batch covered by the future signature.
    pub messages: Vec<SendMessage>,
    /// The resolved Unix expiration timestamp that the signer will preserve.
    pub valid_until: u64,
    /// Whether fresh account state requires the relayed message to carry wallet `StateInit`.
    pub needs_state_init: bool,
}

/// The public phase of the current or last send workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum SendPhase {
    /// No send has started.
    Idle,
    /// The engine validates the request, journal, and chain state.
    Validating,
    /// The host authorizes protected-secret access.
    Authorizing,
    /// Rust constructs and signs the transfer.
    Preparing,
    /// The host writes a durable journal record.
    Persisting,
    /// The exact signed BOC is durable and ready for submission.
    ReadyToSubmit,
    /// The HTTP host submits the signed BOC.
    Submitting,
    /// Submission can have succeeded, but the engine has no definite response.
    SubmissionUnknown,
    /// The provider accepted the signed BOC.
    Submitted,
    /// A signed internal message was durably returned for submission by a relayer.
    HandedOff,
    /// The external message was found as the inbound message of an on-chain transaction.
    Confirmed,
    /// Another external message consumed the sequence number reserved by this send.
    Replaced,
    /// The signed sequence number was consumed, but available provider evidence
    /// cannot identify which competing internal request executed.
    SequenceNumberConsumed,
    /// Provider time passed the signed validity window and the message was not observed.
    Expired,
    /// An explicit same-sequence-number resend replaced this journal attempt.
    Superseded,
    /// The send failed before an ambiguous submission result.
    Failed,
    /// The send was cancelled before its durable commit boundary.
    Cancelled,
}

/// Why a durable outgoing message cannot yet be resolved to a final outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum PendingReason {
    /// Toncenter still exposes an emulated pending transaction for the message.
    InMempool,
    /// No terminal evidence exists and the signed validity window is still open.
    AwaitingWindow,
}

/// Chain evidence and retry guidance for a durable outgoing message.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionInfo {
    /// The confirmed transaction hash, when the message was executed.
    pub transaction_hash: Option<Base64Hash>,
    /// The confirmed transaction logical time.
    pub transaction_lt: Option<UnsignedDecimalString>,
    /// Why the message remains unresolved, when no terminal evidence exists.
    pub pending_reason: Option<PendingReason>,
    /// Whether a new send with `force` can replace this unresolved signed send.
    pub can_force_retry: bool,
    /// A UI polling hint. This is not a correctness deadline.
    pub retry_after_hint_ms: Option<u64>,
}

/// The observable state of the send workflow.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendSnapshot {
    /// The active or last operation identifier, if a send started.
    pub operation_id: Option<NonEmptyString>,
    /// The current public phase.
    pub phase: SendPhase,
    /// A sanitized diagnostic for failed or unknown submission states.
    pub error_message: Option<String>,
    /// Resolution evidence or pending guidance for the durable outgoing message.
    #[serde(default)]
    pub resolution: Option<ResolutionInfo>,
}

/// A bounded summary of the Toncenter trace emulated before authorization.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendEmulation {
    /// The masterchain block used as the emulation state snapshot.
    pub mc_block_seqno: u32,
    /// Fees charged by the source wallet transaction, in nanograms.
    pub wallet_fees_nanograms: UnsignedDecimalString,
    /// Sum of fees across every transaction currently present in the trace.
    pub trace_fees_nanograms: UnsignedDecimalString,
    /// Number of transactions currently present in the emulated trace.
    pub transaction_count: u64,
    /// High-level actions recognized by Toncenter in the emulated trace.
    pub actions: Vec<SendEmulationAction>,
    /// Whether every returned transaction completed without an observed phase failure.
    ///
    /// A false value does not by itself block submission. A recipient can
    /// reject or bounce after the source wallet transaction succeeds.
    pub trace_succeeded: bool,
    /// Whether Toncenter reports that the trace still has unresolved messages.
    pub is_incomplete: bool,
}

/// One high-level action recognized by Toncenter during emulation.
///
/// `details_json` preserves action-specific fields without making the stable
/// wallet API depend on Toncenter's growing list of action schemas.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendEmulationAction {
    /// The validated action identifier in standard padded Base64.
    pub action_id: Base64Hash,
    /// Toncenter's action kind, for example `ton_transfer` or `call_contract`.
    pub kind: NonEmptyString,
    /// Whether Toncenter considers the complete high-level action successful.
    pub succeeded: bool,
    /// Validated TON account addresses involved in the action.
    pub accounts: Vec<TonAddressString>,
    /// Validated transaction hashes associated with the action.
    pub transaction_hashes: Vec<Base64Hash>,
    /// Action-specific details serialized as a JSON object.
    pub details_json: String,
}

/// The result returned after the send workflow reaches a terminal phase.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    /// The application operation identifier.
    pub operation_id: NonEmptyString,
    /// The normalized signed external-message hash in standard padded Base64.
    pub message_hash: Base64Hash,
    /// The exact signed external-message `BoC` submitted to the provider.
    pub signed_boc: Boc,
    /// The terminal phase. This can be [`SendPhase::SubmissionUnknown`].
    pub phase: SendPhase,
}

#[cfg(test)]
mod tests {
    use super::{
        SendAmount, SendExpiration, SendIntent, SendMessage, SendMessageBody, SendRequest,
    };
    use crate::{Boc, NonEmptyString, TonAddressString};
    use ton::ton_core::cell::TonCell;

    #[test]
    fn exact_amount_rejects_negative_input() {
        assert!(SendAmount::exact("-100").is_err());
    }

    #[test]
    fn serde_rejects_negative_exact_amount() {
        let result = serde_json::from_str::<SendAmount>(r#"{"kind":"exact","nanograms":"-100"}"#);
        assert!(result.is_err());
    }

    /// Verifies the JSON boundary for every body and expiration variant.
    #[test]
    fn send_request_json_preserves_typed_intent_variants() {
        let payload = Boc::try_from(TonCell::EMPTY_BOC.to_vec()).expect("valid payload BOC");
        let bodies = [
            SendMessageBody::Empty,
            SendMessageBody::Comment {
                text: "hello".to_owned(),
            },
            SendMessageBody::RawPayload { boc: payload },
        ];
        let expirations = [
            SendExpiration::EngineDefault,
            SendExpiration::Exact {
                unix_timestamp: 1_900_000_000,
            },
        ];

        for expiration in expirations {
            for body in bodies.clone() {
                let request = send_request(expiration.clone(), body);
                let json = serde_json::to_value(&request).expect("send request serializes");
                let decoded: SendRequest =
                    serde_json::from_value(json).expect("send request deserializes");

                assert_eq!(decoded, request);
            }
        }
    }

    /// Keeps the Web request field aligned with the camel-case TypeScript API.
    #[test]
    fn exact_expiration_serializes_with_unix_timestamp_in_camel_case() {
        let json = serde_json::to_value(SendExpiration::Exact {
            unix_timestamp: 1_900_000_000,
        })
        .expect("expiration serializes");

        assert_eq!(
            json,
            serde_json::json!({"kind": "exact", "unixTimestamp": 1_900_000_000_u64})
        );
    }

    /// Keeps older JSON callers on the safe non-forced send policy.
    #[test]
    fn send_request_defaults_force_to_false_when_the_field_is_absent() {
        let mut json = serde_json::to_value(send_request(
            SendExpiration::EngineDefault,
            SendMessageBody::Empty,
        ))
        .expect("send request serializes");
        json.as_object_mut()
            .expect("send request is an object")
            .remove("force");

        let decoded: SendRequest =
            serde_json::from_value(json).expect("legacy send request deserializes");

        assert!(!decoded.force);
    }

    /// Builds one public send request for serialization tests.
    fn send_request(expiration: SendExpiration, body: SendMessageBody) -> SendRequest {
        SendRequest {
            operation_id: NonEmptyString::try_from("operation").expect("valid operation id"),
            force: false,
            intent: SendIntent {
                expiration,
                messages: vec![SendMessage {
                    destination: TonAddressString::try_from(
                        "0:2222222222222222222222222222222222222222222222222222222222222222",
                    )
                    .expect("valid destination"),
                    amount: SendAmount::exact("1").expect("valid amount"),
                    body,
                    state_init: None,
                }],
            },
        }
    }
}
