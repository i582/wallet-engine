//! Shared HTTP request construction and response-boundary validation.

use url::Url;

use crate::{
    DomainError, ErrorCategory, ErrorCode, HttpHeader, HttpHostError, HttpHostErrorKind,
    HttpMethod, HttpRequest, HttpRequestId, HttpResponse, RetryAdvice, WalletClientConfig,
    WalletClientError,
};

use super::provider::response_error;
use crate::domain::bounded_diagnostic;

const MAX_RESPONSE_BODY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESPONSE_HEADER_BYTES: u64 = 64 * 1024;

pub(super) fn evaluate_response<T>(
    request: &HttpRequest,
    result: Result<HttpResponse, HttpHostError>,
    parse: impl FnOnce(&[u8]) -> Result<T, DomainError>,
) -> Result<T, DomainError> {
    if let Ok(response) = &result
        && response.final_url != request.url
    {
        return Err(host_error(
            HttpHostErrorKind::PolicyViolation,
            "HTTP redirect or mismatched final URL",
        ));
    }

    if let Ok(response) = &result {
        if response.body.len() as u64 > request.max_response_body_bytes {
            return Err(host_error(
                HttpHostErrorKind::ResponseTooLarge,
                "HTTP response exceeded the requested limit",
            ));
        }

        let header_bytes = response.headers.iter().fold(0_u64, |size, header| {
            size.saturating_add((header.name.len() + header.value.len()) as u64)
        });
        if header_bytes > request.max_response_header_bytes {
            return Err(host_error(
                HttpHostErrorKind::ResponseTooLarge,
                "HTTP response headers exceeded the requested limit",
            ));
        }
    }

    evaluate(result, parse)
}

pub(super) fn build_toncenter_v2_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    path: &str,
    query: &[(&str, &str)],
) -> Result<HttpRequest, WalletClientError> {
    let path = ["api", "v2", path];
    Ok(HttpRequest {
        id,
        method: HttpMethod::Get,
        url: build_toncenter_url(config, &path, query)?,
        headers: vec![HttpHeader {
            name: "Accept".to_owned(),
            value: "application/json".to_owned(),
        }],
        body: Vec::new(),
        timeout_ms: config.providers.request_timeout_ms,
        max_response_header_bytes: MAX_RESPONSE_HEADER_BYTES,
        max_response_body_bytes: MAX_RESPONSE_BODY_BYTES,
    })
}

/// Builds a bounded GET request for a Toncenter v3 endpoint.
///
/// The caller supplies only the endpoint suffix. Keeping `/api/v3` here lets
/// one configured deployment base serve both the existing v2 reads and the v3
/// resolution evidence without rewriting or guessing the base URL.
pub(super) fn build_toncenter_v3_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    path: &str,
    query: &[(&str, &str)],
) -> Result<HttpRequest, WalletClientError> {
    let path = ["api", "v3", path];
    Ok(HttpRequest {
        id,
        method: HttpMethod::Get,
        url: build_toncenter_url(config, &path, query)?,
        headers: vec![HttpHeader {
            name: "Accept".to_owned(),
            value: "application/json".to_owned(),
        }],
        body: Vec::new(),
        timeout_ms: config.providers.request_timeout_ms,
        max_response_header_bytes: MAX_RESPONSE_HEADER_BYTES,
        max_response_body_bytes: MAX_RESPONSE_BODY_BYTES,
    })
}

/// Builds a Toncenter URL below the configured deployment base.
///
/// Callers provide the complete API-specific path as individual segments. This
/// keeps API version selection at the request site and avoids deriving one API
/// root from another.
pub(super) fn build_toncenter_url(
    config: &WalletClientConfig,
    path: &[&str],
    query: &[(&str, &str)],
) -> Result<String, WalletClientError> {
    build_provider_url(&config.providers.toncenter_base_url, path, query)
}

fn evaluate<T>(
    result: Result<HttpResponse, HttpHostError>,
    parse: impl FnOnce(&[u8]) -> Result<T, DomainError>,
) -> Result<T, DomainError> {
    let response = match result {
        Ok(response) => response,
        Err(HttpHostError::Failed { kind, diagnostic }) => {
            return Err(host_error(kind, &diagnostic));
        }
    };

    if let Some(error) = response_error(response.status, &response.headers, &response.body) {
        return Err(error);
    }

    parse(&response.body)
}

fn host_error(kind: HttpHostErrorKind, message: &str) -> DomainError {
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
        developer_message: bounded_diagnostic(message),
        provider_status: None,
        retry_after_ms: None,
        host_kind: Some(kind),
    }
}

fn build_provider_url(
    base: &str,
    path: &[&str],
    query: &[(&str, &str)],
) -> Result<String, WalletClientError> {
    let mut url = Url::parse(base).map_err(|_| WalletClientError::InvalidConfig)?;

    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| WalletClientError::InvalidConfig)?;
        segments.pop_if_empty();
        for segment in path {
            segments.push(segment);
        }
    }

    url.set_query(None);
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query.iter().copied());
    }

    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Network, ProviderConfig};

    fn request() -> HttpRequest {
        HttpRequest {
            id: HttpRequestId { value: 7 },
            method: HttpMethod::Get,
            url: "https://provider.example/api/v2/resource?limit=10".to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout_ms: 15_000,
            max_response_header_bytes: 32,
            max_response_body_bytes: 16,
        }
    }

    fn response(request: &HttpRequest, body: &[u8]) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: body.to_vec(),
            final_url: request.url.clone(),
        }
    }

    fn parse_text(body: &[u8]) -> Result<String, DomainError> {
        Ok(String::from_utf8_lossy(body).into_owned())
    }

    #[test]
    fn accepts_a_bounded_response_and_invokes_the_parser() {
        let request = request();
        let parsed = evaluate_response(&request, Ok(response(&request, b"wallet")), parse_text);

        assert_eq!(parsed, Ok("wallet".to_owned()));
    }

    #[test]
    fn rejects_a_redirect_before_parsing_the_body() {
        let request = request();
        let mut redirected = response(&request, b"ignored");
        redirected.final_url = "https://attacker.example/response".to_owned();

        let error = evaluate_response(&request, Ok(redirected), parse_text)
            .expect_err("a changed final URL must be rejected");

        assert_eq!(error.code, ErrorCode::HostPolicyViolation);
        assert_eq!(error.category, ErrorCategory::HostPolicy);
        assert_eq!(error.retry, RetryAdvice::None);
        assert_eq!(error.host_kind, Some(HttpHostErrorKind::PolicyViolation));
        assert_eq!(
            error.developer_message,
            "HTTP redirect or mismatched final URL"
        );
    }

    #[test]
    fn rejects_a_body_above_the_request_limit() {
        let mut request = request();
        request.max_response_body_bytes = 3;

        let error = evaluate_response(&request, Ok(response(&request, b"four")), parse_text)
            .expect_err("an oversized body must be rejected");

        assert_response_too_large(error, "HTTP response exceeded the requested limit");
    }

    #[test]
    fn rejects_headers_above_the_combined_request_limit() {
        let mut request = request();
        request.max_response_header_bytes = 5;
        let mut oversized = response(&request, b"ok");
        oversized.headers = vec![
            HttpHeader {
                name: "A".to_owned(),
                value: "123".to_owned(),
            },
            HttpHeader {
                name: "B".to_owned(),
                value: "45".to_owned(),
            },
        ];

        let error = evaluate_response(&request, Ok(oversized), parse_text)
            .expect_err("the combined header size must be enforced");

        assert_response_too_large(error, "HTTP response headers exceeded the requested limit");
    }

    #[test]
    fn maps_every_host_failure_family_to_a_stable_domain_error() {
        let cases = [
            (
                HttpHostErrorKind::Cancelled,
                ErrorCode::HostCancelled,
                ErrorCategory::Cancellation,
                RetryAdvice::None,
            ),
            (
                HttpHostErrorKind::PolicyViolation,
                ErrorCode::HostPolicyViolation,
                ErrorCategory::HostPolicy,
                RetryAdvice::None,
            ),
            (
                HttpHostErrorKind::ResponseTooLarge,
                ErrorCode::ResponseTooLarge,
                ErrorCategory::HostPolicy,
                RetryAdvice::None,
            ),
            (
                HttpHostErrorKind::Offline,
                ErrorCode::TransportFailed,
                ErrorCategory::Transport,
                RetryAdvice::Safe,
            ),
            (
                HttpHostErrorKind::Timeout,
                ErrorCode::TransportFailed,
                ErrorCategory::Transport,
                RetryAdvice::Safe,
            ),
            (
                HttpHostErrorKind::ConnectionLost,
                ErrorCode::TransportFailed,
                ErrorCategory::Transport,
                RetryAdvice::Safe,
            ),
            (
                HttpHostErrorKind::Dns,
                ErrorCode::TransportFailed,
                ErrorCategory::Transport,
                RetryAdvice::Safe,
            ),
            (
                HttpHostErrorKind::Tls,
                ErrorCode::TransportFailed,
                ErrorCategory::Transport,
                RetryAdvice::Safe,
            ),
            (
                HttpHostErrorKind::Other,
                ErrorCode::TransportFailed,
                ErrorCategory::Transport,
                RetryAdvice::Safe,
            ),
        ];

        for (kind, code, category, retry) in cases {
            let request = request();
            let error = evaluate_response(
                &request,
                Err(HttpHostError::Failed {
                    kind,
                    diagnostic: format!("{kind:?} diagnostic"),
                }),
                parse_text,
            )
            .expect_err("a host error must not reach the parser");

            assert_eq!(error.code, code, "wrong code for {kind:?}");
            assert_eq!(error.category, category, "wrong category for {kind:?}");
            assert_eq!(error.retry, retry, "wrong retry advice for {kind:?}");
            assert_eq!(error.host_kind, Some(kind));
            assert_eq!(error.provider_status, None);
            assert_eq!(error.retry_after_ms, None);
        }
    }

    #[test]
    fn forwards_provider_rejections_before_parsing() {
        let mut request = request();
        request.max_response_body_bytes = 64;
        let mut rejected = response(&request, br#"{"error":"denied"}"#);
        rejected.status = 403;

        let error = evaluate_response(&request, Ok(rejected), parse_text)
            .expect_err("a provider rejection must not reach the parser");

        assert_eq!(error.code, ErrorCode::HttpRejected);
        assert_eq!(error.category, ErrorCategory::ProviderProtocol);
        assert_eq!(error.provider_status, Some(403));
    }

    #[test]
    fn builds_a_toncenter_request_without_losing_the_base_path() {
        let config = WalletClientConfig {
            record_id: "record".to_owned(),
            address: "address".to_owned(),
            public_key: vec![0; 32],
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            resolution_poll_interval_ms: 4_000,
            resolution_active_budget_ms: 60_000,
            providers: ProviderConfig {
                toncenter_base_url: "https://provider.example/custom/".to_owned(),
                request_timeout_ms: 12_345,
            },
        };

        let request = build_toncenter_v2_request(
            &config,
            HttpRequestId { value: 9 },
            "getTransactions",
            &[("address", "0:abc"), ("limit", "10")],
        )
        .expect("the provider URL is valid");

        assert_eq!(request.id, HttpRequestId { value: 9 });
        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(
            request.url,
            "https://provider.example/custom/api/v2/getTransactions?address=0%3Aabc&limit=10"
        );
        assert_eq!(
            request.headers,
            vec![HttpHeader {
                name: "Accept".to_owned(),
                value: "application/json".to_owned(),
            }]
        );
        assert!(request.body.is_empty());
        assert_eq!(request.timeout_ms, 12_345);
        assert_eq!(request.max_response_header_bytes, MAX_RESPONSE_HEADER_BYTES);
        assert_eq!(request.max_response_body_bytes, MAX_RESPONSE_BODY_BYTES);
    }

    #[test]
    fn rejects_a_provider_base_that_cannot_hold_path_segments() {
        let config = WalletClientConfig {
            record_id: "record".to_owned(),
            address: "address".to_owned(),
            public_key: vec![0; 32],
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            resolution_poll_interval_ms: 4_000,
            resolution_active_budget_ms: 60_000,
            providers: ProviderConfig {
                toncenter_base_url: "mailto:provider@example.com".to_owned(),
                request_timeout_ms: 15_000,
            },
        };

        assert_eq!(
            build_toncenter_v2_request(&config, HttpRequestId { value: 1 }, "resource", &[],),
            Err(WalletClientError::InvalidConfig)
        );
    }

    fn assert_response_too_large(error: DomainError, diagnostic: &str) {
        assert_eq!(error.code, ErrorCode::ResponseTooLarge);
        assert_eq!(error.category, ErrorCategory::HostPolicy);
        assert_eq!(error.retry, RetryAdvice::None);
        assert_eq!(error.host_kind, Some(HttpHostErrorKind::ResponseTooLarge));
        assert_eq!(error.developer_message, diagnostic);
    }
}
