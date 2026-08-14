//! Public transfer requests, phases, snapshots, and results.

use super::ProtectedSecretRef;

/// Requests one signed V5R1 transfer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendRequest {
    /// A unique idempotency identifier chosen by the application.
    pub operation_id: String,
    /// A friendly or raw TON destination address.
    pub destination: String,
    /// A positive canonical unsigned amount in nanograms.
    pub amount_nanograms: String,
    /// The protected mnemonic reference for the source wallet.
    pub secret_ref: ProtectedSecretRef,
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
    /// The send failed before an ambiguous submission result.
    Failed,
    /// The send was cancelled before its durable commit boundary.
    Cancelled,
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
}

/// A public summary of a signed transfer before submission.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PreparedSend {
    /// The application operation identifier.
    pub operation_id: String,
    /// The TON `uint32` Unix timestamp after which validators reject the transfer.
    pub valid_until: u32,
    /// The destination TON address.
    pub destination: String,
    /// The exact transfer amount in nanograms.
    pub amount_nanograms: String,
}

/// The result returned after the send workflow reaches a terminal phase.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    /// The application operation identifier.
    pub operation_id: String,
    /// The normalized signed external-message hash in standard padded Base64.
    pub message_hash: String,
    /// The terminal phase. This can be [`SendPhase::SubmissionUnknown`].
    pub phase: SendPhase,
}
