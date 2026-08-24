//! Provider transport for hosts that cannot expose HTTP response metadata.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::domain::bounded_diagnostic;
use crate::{DomainError, ErrorCategory, ErrorCode, RetryAdvice};

use super::{HttpHostErrorKind, HttpRequest, HttpRequestId, ProviderTransport};

/// Classifies a failure reported by a status-less provider host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum StatuslessHostErrorKind {
    /// No network connection is available.
    Offline,
    /// The request exceeded the host deadline.
    Timeout,
    /// An established relay or proxy connection ended before completion.
    ConnectionLost,
    /// The request violated a host security policy.
    PolicyViolation,
    /// The response exceeded a limit imposed by the host.
    ResponseTooLarge,
    /// The host cancelled the request.
    Cancelled,
    /// The failure does not match another kind.
    Other,
}

/// A failure returned by [`WalletStatuslessHost`].
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
pub enum StatuslessHostError {
    /// Reports a classified transport failure with a safe diagnostic message.
    #[error("status-less host failure ({kind:?}): {diagnostic}")]
    Failed {
        /// The stable failure classification.
        kind: StatuslessHostErrorKind,
        /// A developer-facing message without secrets or credential values.
        ///
        /// Opaque RPC error information can be included here when no stable
        /// classification is available.
        diagnostic: String,
    },
}

/// Executes provider requests through a transport without HTTP response metadata.
///
/// The request URL is the logical Toncenter destination and does not require a
/// direct connection to that origin. The host can route it through a trusted
/// relay or protocol proxy, but must not follow or emulate provider redirects.
/// A successful callback returns only the provider body: it makes no claim
/// about an HTTP status, response headers, or final URL.
#[uniffi::export(foreign)]
#[async_trait]
pub trait WalletStatuslessHost: Send + Sync {
    /// Executes one complete logical provider request.
    ///
    /// The host must enforce `request.timeout_ms`. It reports timeout and
    /// cancellation explicitly; opaque RPC failures can use
    /// [`StatuslessHostErrorKind::Other`] with a bounded diagnostic.
    async fn execute_statusless(
        &self,
        request: HttpRequest,
    ) -> Result<Vec<u8>, StatuslessHostError>;

    /// Requests cancellation of the request with `request_id`.
    ///
    /// This callback has the same idempotency and early-cancellation contract
    /// as [`crate::WalletHttpHost::cancel_http`].
    async fn cancel_statusless(&self, request_id: HttpRequestId);
}

pub(crate) struct StatuslessTransport {
    host: Arc<dyn WalletStatuslessHost>,
}

impl StatuslessTransport {
    pub(crate) fn new(host: Arc<dyn WalletStatuslessHost>) -> Self {
        Self { host }
    }
}

#[async_trait]
impl ProviderTransport for StatuslessTransport {
    async fn execute(&self, request: &HttpRequest) -> Result<Vec<u8>, DomainError> {
        let body = self
            .host
            .execute_statusless(request.clone())
            .await
            .map_err(map_host_error)?;

        if let Some(error) = naked_provider_error(&body) {
            return Err(error);
        }

        Ok(body)
    }

    async fn cancel(&self, request_id: HttpRequestId) {
        self.host.cancel_statusless(request_id).await;
    }
}

fn map_host_error(error: StatuslessHostError) -> DomainError {
    let StatuslessHostError::Failed { kind, diagnostic } = error;
    let kind = http_kind(kind);
    let cancelled = kind == HttpHostErrorKind::Cancelled;
    let policy = kind == HttpHostErrorKind::PolicyViolation;
    let too_large = kind == HttpHostErrorKind::ResponseTooLarge;

    DomainError {
        code: if cancelled {
            ErrorCode::HostCancelled
        } else if policy {
            ErrorCode::HostPolicyViolation
        } else if too_large {
            ErrorCode::ResponseTooLarge
        } else {
            ErrorCode::TransportFailed
        },
        category: if cancelled {
            ErrorCategory::Cancellation
        } else if policy || too_large {
            ErrorCategory::HostPolicy
        } else {
            ErrorCategory::Transport
        },
        retry: if cancelled || policy || too_large {
            RetryAdvice::None
        } else {
            RetryAdvice::Safe
        },
        developer_message: bounded_diagnostic(diagnostic),
        provider_status: None,
        retry_after_ms: None,
        host_kind: Some(kind),
    }
}

const fn http_kind(kind: StatuslessHostErrorKind) -> HttpHostErrorKind {
    match kind {
        StatuslessHostErrorKind::Offline => HttpHostErrorKind::Offline,
        StatuslessHostErrorKind::Timeout => HttpHostErrorKind::Timeout,
        StatuslessHostErrorKind::ConnectionLost => HttpHostErrorKind::ConnectionLost,
        StatuslessHostErrorKind::PolicyViolation => HttpHostErrorKind::PolicyViolation,
        StatuslessHostErrorKind::ResponseTooLarge => HttpHostErrorKind::ResponseTooLarge,
        StatuslessHostErrorKind::Cancelled => HttpHostErrorKind::Cancelled,
        StatuslessHostErrorKind::Other => HttpHostErrorKind::Other,
    }
}

/// Recognizes a top-level provider error when no endpoint envelope owns it.
fn naked_provider_error(body: &[u8]) -> Option<DomainError> {
    let value: Value = serde_json::from_slice(body).ok()?;

    if ["ok", "result", "jsonrpc"]
        .into_iter()
        .any(|field| value.get(field).is_some())
    {
        return None;
    }

    let developer_message = value.get("error").and_then(value_message)?;
    let provider_status = value.get("code").and_then(provider_code);

    if provider_status == Some(429) {
        return Some(DomainError {
            code: ErrorCode::RateLimited,
            category: ErrorCategory::RateLimit,
            retry: RetryAdvice::Safe,
            developer_message: bounded_diagnostic(developer_message),
            provider_status,
            retry_after_ms: None,
            host_kind: None,
        });
    }

    Some(DomainError {
        code: ErrorCode::HttpRejected,
        category: ErrorCategory::ProviderProtocol,
        retry: if provider_status.is_some_and(|status| status >= 500) {
            RetryAdvice::Safe
        } else {
            RetryAdvice::None
        },
        developer_message: bounded_diagnostic(developer_message),
        provider_status,
        retry_after_ms: None,
        host_kind: None,
    })
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures::executor::block_on;

    use super::*;
    use crate::{HttpMethod, HttpRequest};

    struct BodyHost {
        body: Vec<u8>,
        requests: Mutex<Vec<HttpRequest>>,
        cancellations: Mutex<Vec<HttpRequestId>>,
    }

    #[async_trait]
    impl WalletStatuslessHost for BodyHost {
        async fn execute_statusless(
            &self,
            request: HttpRequest,
        ) -> Result<Vec<u8>, StatuslessHostError> {
            self.requests.lock().expect("request lock").push(request);
            Ok(self.body.clone())
        }

        async fn cancel_statusless(&self, request_id: HttpRequestId) {
            self.cancellations
                .lock()
                .expect("cancellation lock")
                .push(request_id);
        }
    }

    fn request() -> HttpRequest {
        HttpRequest {
            id: HttpRequestId { value: 7 },
            method: HttpMethod::Get,
            url: "https://provider.example/api/v3/nft/items".to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout_ms: 15_000,
        }
    }

    #[test]
    fn endpoint_owned_envelopes_are_not_intercepted() {
        assert!(naked_provider_error(br#"{"ok":false,"error":"denied"}"#).is_none());
        assert!(
            naked_provider_error(
                br#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"denied"}}"#
            )
            .is_none()
        );
        assert!(naked_provider_error(br#"{"nft_items":[]}"#).is_none());
    }

    #[test]
    fn bare_proxy_error_is_not_treated_as_a_successful_v3_body() {
        let error = naked_provider_error(br#"{"error":"proxy request failed"}"#)
            .expect("the bare proxy error must be recognized");

        assert_eq!(error.code, ErrorCode::HttpRejected);
        assert_eq!(error.category, ErrorCategory::ProviderProtocol);
        assert_eq!(error.retry, RetryAdvice::None);
        assert_eq!(error.developer_message, "proxy request failed");
        assert_eq!(error.provider_status, None);
        assert_eq!(error.retry_after_ms, None);
        assert_eq!(error.host_kind, None);
    }

    #[test]
    fn maps_statusless_timeout_without_inventing_provider_metadata() {
        let error = map_host_error(StatuslessHostError::Failed {
            kind: StatuslessHostErrorKind::Timeout,
            diagnostic: "proxy deadline".to_owned(),
        });

        assert_eq!(error.code, ErrorCode::TransportFailed);
        assert_eq!(error.category, ErrorCategory::Transport);
        assert_eq!(error.retry, RetryAdvice::Safe);
        assert_eq!(error.developer_message, "proxy deadline");
        assert_eq!(error.provider_status, None);
        assert_eq!(error.retry_after_ms, None);
        assert_eq!(error.host_kind, Some(HttpHostErrorKind::Timeout));
    }

    #[test]
    fn transport_returns_body_and_delegates_cancellation_without_http_metadata() {
        let host = Arc::new(BodyHost {
            body: br#"{"nft_items":[]}"#.to_vec(),
            requests: Mutex::new(Vec::new()),
            cancellations: Mutex::new(Vec::new()),
        });
        let transport = StatuslessTransport::new(host.clone());
        let request = request();

        let body = block_on(transport.execute(&request))
            .expect("a status-less success must expose its body");
        block_on(transport.cancel(request.id));

        assert_eq!(body, br#"{"nft_items":[]}"#);
        assert_eq!(
            host.requests.lock().expect("request lock").as_slice(),
            std::slice::from_ref(&request)
        );
        assert_eq!(
            host.cancellations
                .lock()
                .expect("cancellation lock")
                .as_slice(),
            &[request.id]
        );
    }

    #[test]
    fn explicit_body_code_can_supply_retry_classification_without_http_metadata() {
        let error = naked_provider_error(br#"{"error":"slow down","code":429}"#)
            .expect("the explicit provider error must be recognized");

        assert_eq!(error.code, ErrorCode::RateLimited);
        assert_eq!(error.category, ErrorCategory::RateLimit);
        assert_eq!(error.retry, RetryAdvice::Safe);
        assert_eq!(error.developer_message, "slow down");
        assert_eq!(error.provider_status, Some(429));
        assert_eq!(error.retry_after_ms, None);
        assert_eq!(error.host_kind, None);
    }
}
