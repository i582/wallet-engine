//! Shared HTTP request construction and response-boundary validation.

use url::Url;

use crate::diagnostic::bounded_diagnostic;
use crate::provider::response_error;
use crate::{
    DomainError, ErrorCategory, ErrorCode, HttpHeader, HttpHostError, HttpHostErrorKind,
    HttpMethod, HttpRequest, HttpRequestId, HttpResponse, RetryAdvice, WalletClientConfig,
    WalletClientError,
};

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

pub(super) fn build_toncenter_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    path: &str,
    query: &[(&str, &str)],
) -> Result<HttpRequest, WalletClientError> {
    let mut request = build_public_request(id, &config.providers.toncenter_base_url, path, query)?;

    request
        .credential
        .clone_from(&config.providers.toncenter_credential);
    request
        .credential_origin
        .clone_from(&config.providers.toncenter_credential_origin);

    Ok(request)
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

fn build_public_request(
    id: HttpRequestId,
    base: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<HttpRequest, WalletClientError> {
    Ok(HttpRequest {
        id,
        method: HttpMethod::Get,
        url: build_provider_url(base, path, query)?,
        headers: vec![HttpHeader {
            name: "Accept".to_owned(),
            value: "application/json".to_owned(),
        }],
        body: Vec::new(),
        credential: None,
        credential_origin: None,
        max_response_header_bytes: MAX_RESPONSE_HEADER_BYTES,
        max_response_body_bytes: MAX_RESPONSE_BODY_BYTES,
    })
}

fn build_provider_url(
    base: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<String, WalletClientError> {
    let mut url = Url::parse(base).map_err(|_| WalletClientError::InvalidConfig)?;

    url.path_segments_mut()
        .map_err(|_| WalletClientError::InvalidConfig)?
        .pop_if_empty()
        .push(path);

    url.query_pairs_mut().extend_pairs(query.iter().copied());

    Ok(url.into())
}
