//! Resource state and errors exposed by wallet operations.

use super::HttpHostErrorKind;

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
    /// The original host failure kind, if the error came from a callback.
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
#[serde(rename_all = "camelCase")]
/// An operational failure returned by [`crate::WalletClient`].
pub enum WalletClientError {
    /// The client configuration or operation request is invalid.
    #[error("invalid  wallet client configuration")]
    InvalidConfig,
    /// A client-local identifier or revision counter overflowed.
    #[error(" wallet client identifier space is exhausted")]
    IdentifierExhausted,
    /// The internal state lock or active operation state is unavailable.
    #[error(" wallet client state is unavailable")]
    StateUnavailable,
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
    #[error(" wallet client is shut down")]
    Shutdown,
}
