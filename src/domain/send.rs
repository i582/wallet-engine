//! Public transfer requests, phases, snapshots, and results.

use crate::{
    Base64Hash, NonEmptyString, TonAddressString, UnsignedDecimalString, UnsignedDecimalStringError,
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

/// Requests one signed V5R1 transfer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    /// A unique idempotency identifier chosen by the application.
    pub operation_id: NonEmptyString,
    /// A friendly or raw TON destination address.
    pub destination: TonAddressString,
    /// The exact-value or whole-balance transfer policy.
    pub amount: SendAmount,
    /// An optional plaintext UTF-8 comment attached to the internal message.
    #[serde(default)]
    pub comment: Option<String>,
}

/// Public transfer intent used to preview a send without unlocking its secret.
///
/// A preview uses fresh account state and a fake signature. It does not reserve
/// an operation identifier, read the send journal, or request the mnemonic.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendPreviewRequest {
    /// A friendly or raw TON destination address.
    pub destination: TonAddressString,
    /// The exact-value or whole-balance transfer policy.
    pub amount: SendAmount,
    /// An optional plaintext UTF-8 comment attached to the emulated message.
    #[serde(default)]
    pub comment: Option<String>,
}

/// An informational transfer preview produced without unlocking the wallet secret.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendPreview {
    /// The destination that was emulated.
    pub destination: TonAddressString,
    /// The transfer value policy used by the emulated message.
    pub amount: SendAmount,
    /// The optional plaintext comment encoded into the emulated message.
    pub comment: Option<String>,
    /// The V5R1 message expiration timestamp used only by this emulation.
    /// A real send calculates a new timestamp from fresh provider state.
    pub valid_until: u64,
    /// The complete fake-signed external message submitted for emulation.
    /// The value is a standard padded Base64-encoded BOC. Clients can pass it
    /// to an independent emulator or explorer without reconstructing the message.
    pub message_boc_base64: String,
    /// The bounded Toncenter emulation summary shown before authorization.
    pub emulation: SendEmulation,
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
    /// The external message was found as the inbound message of an on-chain transaction.
    Confirmed,
    /// Another external message consumed the sequence number reserved by this send.
    Replaced,
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
    /// Whether this engine version can build an explicit same-seqno replacement.
    pub can_force_retry: bool,
    /// A UI polling hint. This is not a correctness deadline.
    pub retry_after_hint_ms: Option<u64>,
}

/// The observable state of the send workflow.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendSnapshot {
    /// The active or last operation identifier, if a send started.
    pub operation_id: Option<String>,
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
    pub kind: String,
    /// Whether Toncenter considers the complete high-level action successful.
    pub succeeded: bool,
    /// Raw TON account addresses involved in the action.
    pub accounts: Vec<String>,
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
    pub operation_id: String,
    /// The normalized signed external-message hash in standard padded Base64.
    pub message_hash: Base64Hash,
    /// The terminal phase. This can be [`SendPhase::SubmissionUnknown`].
    pub phase: SendPhase,
}

#[cfg(test)]
mod tests {
    use super::SendAmount;

    #[test]
    fn exact_amount_rejects_negative_input() {
        assert!(SendAmount::exact("-100").is_err());
    }

    #[test]
    fn serde_rejects_negative_exact_amount() {
        let result = serde_json::from_str::<SendAmount>(r#"{"kind":"exact","nanograms":"-100"}"#);
        assert!(result.is_err());
    }
}
