//! Shared HTTP request construction and response-boundary validation.

use crate::{
    DomainError, ErrorCategory, ErrorCode, HttpHeader, HttpHostError, HttpHostErrorKind,
    HttpMethod, HttpRequest, HttpRequestId, HttpResponse, RetryAdvice, WalletClientConfig,
    WalletClientError,
};

use super::provider::response_error;
use crate::domain::bounded_diagnostic;

/// Maps host and provider failures, verifies the response origin, and returns the body.
pub(super) fn process_response(
    request: &HttpRequest,
    result: Result<HttpResponse, HttpHostError>,
) -> Result<Vec<u8>, DomainError> {
    let response = match result {
        Ok(response) => response,
        Err(HttpHostError::Failed { kind, diagnostic }) => {
            return Err(host_error(kind, &diagnostic));
        }
    };

    // Native transports commonly follow redirects by default. Reject a truthful
    // host report when the response came from anywhere but the requested endpoint.
    if response.final_url != request.url {
        return Err(host_error(
            HttpHostErrorKind::PolicyViolation,
            "HTTP redirect or mismatched final URL",
        ));
    }

    if let Some(error) = response_error(response.status, &response.headers, &response.body) {
        return Err(error);
    }

    Ok(response.body)
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
    })
}

/// Builds a GET request for a Toncenter v3 endpoint.
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
    let mut url = provider_base(base)?;
    for segment in path {
        url.push('/');
        url.push_str(segment);
    }
    if let Some(((first_name, first_value), remaining)) = query.split_first() {
        url.push('?');
        push_query_pair(&mut url, first_name, first_value);
        for (name, value) in remaining {
            url.push('&');
            push_query_pair(&mut url, name, value);
        }
    }
    Ok(url)
}

/// Validates only the URL surface Wallet Engine needs and preserves its path.
///
/// Provider bases are application configuration, not arbitrary navigation
/// URLs. Restricting them to an ASCII HTTP(S) authority avoids pulling a full
/// browser URL/IDNA implementation into every native wallet binary.
fn provider_base(value: &str) -> Result<String, WalletClientError> {
    let (scheme, remainder) = value
        .split_once("://")
        .ok_or(WalletClientError::InvalidProviderBaseUrl)?;
    if !matches!(scheme, "http" | "https")
        || remainder.is_empty()
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(WalletClientError::InvalidProviderBaseUrl);
    }

    let without_suffix = remainder
        .split(['?', '#'])
        .next()
        .ok_or(WalletClientError::InvalidProviderBaseUrl)?;
    let authority_end = without_suffix.find('/').unwrap_or(without_suffix.len());
    let authority = without_suffix
        .get(..authority_end)
        .ok_or(WalletClientError::InvalidProviderBaseUrl)?;
    if !valid_authority(authority) {
        return Err(WalletClientError::InvalidProviderBaseUrl);
    }
    let path = without_suffix
        .get(authority_end..)
        .ok_or(WalletClientError::InvalidProviderBaseUrl)?;
    if path.contains('\\') {
        return Err(WalletClientError::InvalidProviderBaseUrl);
    }

    let mut normalized = String::with_capacity(value.len());
    normalized.push_str(scheme);
    normalized.push_str("://");
    normalized.push_str(authority);
    push_path(&mut normalized, path.trim_end_matches('/'));
    Ok(normalized)
}

fn valid_authority(value: &str) -> bool {
    if value.is_empty() || value.contains('@') {
        return false;
    }
    if let Some(ipv6) = value.strip_prefix('[') {
        let Some((host, port)) = ipv6.split_once(']') else {
            return false;
        };
        return !host.is_empty()
            && host
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b':' || byte == b'.')
            && (port.is_empty() || port.strip_prefix(':').is_some_and(valid_decimal_port));
    }

    let (host, port) = value
        .rsplit_once(':')
        .map_or((value, None), |(host, port)| (host, Some(port)));
    !host.is_empty()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        && port.is_none_or(valid_decimal_port)
}

fn valid_decimal_port(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u16>().is_ok()
}

fn push_query_pair(output: &mut String, name: &str, value: &str) {
    push_query_component(output, name);
    output.push('=');
    push_query_component(output, value);
}

fn push_path(output: &mut String, value: &str) {
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'/' | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b'-'
                    | b'.'
                    | b':'
                    | b';'
                    | b'='
                    | b'@'
                    | b'_'
                    | b'~'
                    | b'%'
            )
        {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(hex_digit(byte >> 4));
            output.push(hex_digit(byte & 0x0f));
        }
    }
}

fn push_query_component(output: &mut String, value: &str) {
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'*' | b'-' | b'.' | b'_') {
            output.push(char::from(byte));
        } else if byte == b' ' {
            output.push('+');
        } else {
            output.push('%');
            output.push(hex_digit(byte >> 4));
            output.push(hex_digit(byte & 0x0f));
        }
    }
}

fn hex_digit(value: u8) -> char {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    HEX.get(usize::from(value)).copied().map_or('?', char::from)
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

    #[test]
    fn returns_the_body_of_a_successful_response() {
        let request = request();
        let body = process_response(&request, Ok(response(&request, b"wallet")))
            .expect("a successful response must expose its body");

        assert_eq!(body, b"wallet");
    }

    #[test]
    fn rejects_a_redirect_before_parsing_the_body() {
        let request = request();
        let mut redirected = response(&request, b"ignored");
        redirected.final_url = "https://attacker.example/response".to_owned();

        let error = process_response(&request, Ok(redirected))
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
            let error = process_response(
                &request,
                Err(HttpHostError::Failed {
                    kind,
                    diagnostic: format!("{kind:?} diagnostic"),
                }),
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
        let request = request();
        let mut rejected = response(&request, br#"{"error":"denied"}"#);
        rejected.status = 403;

        let error = process_response(&request, Ok(rejected))
            .expect_err("a provider rejection must not reach the parser");

        assert_eq!(error.code, ErrorCode::HttpRejected);
        assert_eq!(error.category, ErrorCategory::ProviderProtocol);
        assert_eq!(error.provider_status, Some(403));
    }

    #[test]
    fn builds_a_toncenter_request_without_losing_the_base_path() {
        let config = WalletClientConfig {
            record_id: crate::NonEmptyString::try_from("record").expect("valid record identifier"),
            address: crate::TonAddressString::try_from(
                "0:1111111111111111111111111111111111111111111111111111111111111111",
            )
            .expect("valid TON address"),
            public_key: vec![0; 32],
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
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
    }

    #[test]
    fn rejects_a_provider_base_that_cannot_hold_path_segments() {
        let config = WalletClientConfig {
            record_id: crate::NonEmptyString::try_from("record").expect("valid record identifier"),
            address: crate::TonAddressString::try_from(
                "0:1111111111111111111111111111111111111111111111111111111111111111",
            )
            .expect("valid TON address"),
            public_key: vec![0; 32],
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig {
                toncenter_base_url: "mailto:provider@example.com".to_owned(),
                request_timeout_ms: 15_000,
            },
        };

        assert_eq!(
            build_toncenter_v2_request(&config, HttpRequestId { value: 1 }, "resource", &[],),
            Err(WalletClientError::InvalidProviderBaseUrl)
        );
    }

    #[test]
    fn provider_builder_encodes_query_without_a_browser_url_stack() {
        assert_eq!(
            build_provider_url(
                "https://provider.example:8443/custom/?discarded=yes#fragment",
                &["api", "v3", "lookup"],
                &[("hash", "+ /=é"), ("address", "0:abc")],
            ),
            Ok(concat!(
                "https://provider.example:8443/custom/api/v3/lookup?",
                "hash=%2B+%2F%3D%C3%A9&address=0%3Aabc"
            )
            .to_owned())
        );
    }

    #[test]
    fn provider_builder_accepts_ipv6_and_rejects_ambiguous_authorities() {
        assert_eq!(
            provider_base("http://[::1]:8080/toncenter/"),
            Ok("http://[::1]:8080/toncenter".to_owned())
        );
        for invalid in [
            "",
            "mailto:provider@example.com",
            "https://",
            "https://user@provider.example",
            "https://provider..example",
            "https://provider.example:99999",
            "https://провайдер.example",
        ] {
            assert_eq!(
                provider_base(invalid),
                Err(WalletClientError::InvalidProviderBaseUrl),
                "accepted {invalid:?}"
            );
        }
        assert_eq!(
            provider_base("https://provider.example/path with spaces/ü/"),
            Ok("https://provider.example/path%20with%20spaces/%C3%BC".to_owned())
        );
    }
}
