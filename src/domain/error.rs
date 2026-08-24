//! Resource state and errors exposed by wallet operations.

use super::AccountStatus;
use crate::{HttpHostErrorKind, UnsignedDecimalString};

/// Replaces control characters and bounds diagnostics stored in public errors.
///
/// This function does not remove secrets. Hosts and providers must never put
/// credential or secret values in diagnostic text.
pub(crate) fn bounded_diagnostic(message: impl AsRef<str>) -> String {
    const DIAGNOSTIC_MAX_CHARS: usize = 512;

    message
        .as_ref()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(DIAGNOSTIC_MAX_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The current state of one independently loaded resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ResourcePhase {
    /// No load is active and no successful value is available yet.
    Idle,
    /// A load is active.
    Loading,
    /// The latest load for this resource succeeded.
    Ready,
    /// The latest load for this resource failed.
    Failed,
}

/// A resource phase and its optional failure details.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ResourceState {
    /// The current resource phase.
    pub phase: ResourcePhase,
    /// The last error. This value is present only for [`ResourcePhase::Failed`].
    pub error: Option<DomainError>,
}

impl ResourceState {
    pub(crate) const fn idle() -> Self {
        Self {
            phase: ResourcePhase::Idle,
            error: None,
        }
    }

    pub(crate) const fn loading() -> Self {
        Self {
            phase: ResourcePhase::Loading,
            error: None,
        }
    }

    pub(crate) const fn ready() -> Self {
        Self {
            phase: ResourcePhase::Ready,
            error: None,
        }
    }

    pub(crate) const fn failed(error: DomainError) -> Self {
        Self {
            phase: ResourcePhase::Failed,
            error: Some(error),
        }
    }
}

/// The broad source of a provider or host error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCategory {
    /// Network transport failed.
    Transport,
    /// The provider rejected a request or returned invalid data.
    ProviderProtocol,
    /// The provider applied a request limit.
    RateLimit,
    /// The host cancelled the request.
    Cancellation,
    /// The host rejected the request because of a security or size policy.
    HostPolicy,
}

/// A stable machine-readable domain error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// The provider response did not match the expected schema.
    InvalidProviderResponse,
    /// The provider returned an HTTP or protocol rejection.
    HttpRejected,
    /// The provider applied a request limit.
    RateLimited,
    /// The host reported a transport failure.
    TransportFailed,
    /// The host cancelled the request.
    HostCancelled,
    /// The response exceeded a configured bound.
    ResponseTooLarge,
    /// The request violated a host security policy.
    HostPolicyViolation,
}

/// States whether the same operation can be retried safely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum RetryAdvice {
    /// Do not retry the operation without a user or configuration change.
    None,
    /// The same read-only operation can be retried.
    Safe,
    /// Retry after [`DomainError::retry_after_ms`].
    AfterDelay,
}

/// Structured error data for account and activity resources.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DomainError {
    /// The stable machine-readable code.
    pub code: ErrorCode,
    /// The broad source of the error.
    pub category: ErrorCategory,
    /// Retry guidance for the failed operation.
    pub retry: RetryAdvice,
    /// A sanitized developer-facing diagnostic with a bounded length.
    pub developer_message: String,
    /// The provider status code, if the provider returned one.
    pub provider_status: Option<u16>,
    /// The provider delay in milliseconds, if it returned a numeric `Retry-After` header.
    pub retry_after_ms: Option<u64>,
    /// The normalized host failure kind, if the error came from a callback.
    ///
    /// Status-less host kinds map to the corresponding legacy HTTP kind so
    /// existing consumers retain one stable classification field.
    pub host_kind: Option<HttpHostErrorKind>,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    thiserror::Error,
    serde::Serialize,
    serde::Deserialize,
    uniffi::Error,
)]
#[uniffi::export(Display)]
#[serde(rename_all = "camelCase")]
/// An operational failure returned by [`crate::WalletClient`].
pub enum WalletClientError {
    /// The configured protected-secret reference is blank.
    #[error("the local signing secret reference is blank")]
    InvalidLocalSecretReference,
    /// The configured public key cannot derive the wallet state.
    #[error("the wallet public key cannot derive the wallet state")]
    InvalidWalletPublicKey,
    /// The configured address does not belong to the public key and network.
    #[error("the wallet address does not match the public key and network")]
    WalletIdentityMismatch,
    /// The configured provider base cannot be extended with an endpoint path.
    #[error("the provider base URL cannot be used to build a request")]
    InvalidProviderBaseUrl,
    /// The transfer request has an invalid operation ID, destination, or amount.
    #[error("invalid send request")]
    InvalidSendRequest,
    /// NFT transfer validation or TEP-62 message construction failed before signing.
    #[error("NFT transfer is unavailable: {diagnostic}")]
    NftTransferUnavailable {
        /// Bounded developer-facing reason that contains no secret material.
        diagnostic: String,
    },
    /// Emulation did not prove a complete successful NFT ownership transfer.
    #[error("NFT transfer emulation was rejected: {diagnostic}")]
    NftTransferEmulationRejected {
        /// Bounded provider or validation diagnostic.
        diagnostic: String,
    },
    /// Encrypted-comment preparation or decryption failed safely.
    #[error("encrypted comment is unavailable: {diagnostic}")]
    EncryptedCommentUnavailable {
        /// Bounded developer-facing reason that contains no secret material.
        diagnostic: String,
    },
    /// TON DNS validation, provider resolution, or wallet-record parsing failed.
    #[error("TON DNS resolution is unavailable: {diagnostic}")]
    DnsResolutionUnavailable {
        /// Bounded developer-facing reason that contains no secret material.
        diagnostic: String,
    },
    /// The wallet has public identity but no protected secret configured for local signing.
    #[error("the wallet is not configured for local signing")]
    LocalSigningUnavailable,
    /// A client-local identifier or revision counter overflowed.
    #[error("wallet client identifier space is exhausted")]
    IdentifierExhausted,
    /// The internal state lock or active operation state is unavailable.
    #[error("wallet client state is unavailable")]
    StateUnavailable,
    /// Another transfer is already being prepared or submitted by this client.
    #[error("another send operation is already in progress")]
    SendAlreadyInProgress,
    /// Another send preview is already fetching or emulating current chain state.
    #[error("another send preview is already in progress")]
    SendPreviewAlreadyInProgress,
    /// A durable prior submission has no definite provider outcome.
    ///
    /// The caller must not create a replacement transfer because the stored
    /// signed message can already be on the network.
    #[error("the previous submission outcome is unresolved")]
    PreviousSubmissionUnresolved,
    /// The provider still reports the sequence number used by the previous send.
    ///
    /// Refresh chain state and retry only after the wallet sequence number advances.
    #[error("the wallet sequence number has not advanced since the previous submission")]
    WalletSeqnoNotAdvanced,
    /// The current on-chain account status does not permit a transfer.
    #[error("wallet account state {status:?} does not permit sending")]
    SendAccountUnavailable {
        /// The fresh account status returned by the provider.
        status: AccountStatus,
    },
    /// The fresh on-chain balance is smaller than the requested transfer value.
    ///
    /// This check excludes network fees. A transfer can still fail on-chain
    /// when its value fits but the remaining balance cannot pay fees.
    #[error(
        "insufficient wallet balance: requested {requested_nanograms} nanograms, available {available_nanograms}"
    )]
    InsufficientBalance {
        /// The fresh provider balance, in nanograms.
        available_nanograms: UnsignedDecimalString,
        /// The requested transfer value, in nanograms.
        requested_nanograms: UnsignedDecimalString,
    },
    /// The requested exact value fits, but the value and emulated wallet fee do not.
    #[error(
        "insufficient wallet balance including fees: requested {requested_nanograms} nanograms, estimated fee {estimated_fee_nanograms} nanograms, available {available_nanograms} nanograms"
    )]
    InsufficientBalanceForFees {
        /// The fresh provider balance, in nanograms.
        available_nanograms: UnsignedDecimalString,
        /// The exact requested value, in nanograms.
        requested_nanograms: UnsignedDecimalString,
        /// The wallet transaction fee returned by emulation.
        estimated_fee_nanograms: UnsignedDecimalString,
    },
    /// The protected secret cannot be decoded as a valid wallet recovery phrase.
    #[error("the protected wallet secret is invalid")]
    InvalidProtectedSecret,
    /// A preview failed while loading fresh wallet state or building its fake-signed message.
    #[error("send preview failed: {diagnostic}")]
    SendPreviewFailed {
        /// A bounded developer-facing explanation that contains no secret material.
        diagnostic: String,
    },
    /// Toncenter could not execute or decode the fake-signed preview emulation.
    #[error("transfer emulation failed: {diagnostic}")]
    EmulationFailed {
        /// A bounded provider or transport diagnostic that contains no secret material.
        diagnostic: String,
    },
    /// The emulator ran correctly, but the current wallet state did not accept
    /// the external message, for example because its seqno became stale.
    #[error("emulation message was not accepted: {diagnostic}")]
    EmulationMessageNotAccepted {
        /// A bounded provider diagnostic that contains no secret material.
        diagnostic: String,
    },
    /// Emulation proved that the source wallet transaction would not complete.
    #[error("transfer emulation rejected the message: {diagnostic}")]
    EmulationRejected {
        /// A bounded explanation of the failed transaction phase.
        diagnostic: String,
        /// The TVM compute exit code, when Toncenter returned one.
        compute_exit_code: Option<i32>,
        /// The action-phase result code, when Toncenter returned one.
        action_result_code: Option<i32>,
    },
    /// A send failed before submission became ambiguous.
    #[error("send failed: {diagnostic}")]
    SendFailed {
        /// A bounded developer-facing explanation that contains no secrets.
        diagnostic: String,
    },
    /// A send can have reached the provider, but no definite result is available.
    #[error("submission outcome is unknown: {diagnostic}")]
    SubmissionUnknown {
        /// A bounded developer-facing explanation that contains no secrets.
        diagnostic: String,
    },
    /// A send crossed its durable commit boundary and cannot be cancelled.
    #[error("the send has crossed its durable commit boundary and can no longer be cancelled")]
    SendCancellationTooLate,
    /// The client is shut down and accepts no new work.
    #[error("wallet client is shut down")]
    Shutdown,
}
