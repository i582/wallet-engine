//! Reqwest implementation of the engine HTTP callback boundary.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt as _;
use reqwest::header::{HeaderName, HeaderValue};
use tokio_util::sync::CancellationToken;
use url::Url;
use wallet_engine::{
    HttpHeader, HttpHostError, HttpHostErrorKind, HttpMethod, HttpRequest, HttpRequestId,
    HttpResponse, WalletHttpHost,
};

const MAX_REQUEST_BODY_BYTES: usize = 256 * 1024;
const MAX_RESPONSE_HEADERS: usize = 64;
const MAX_REQUEST_TIMEOUT_MS: u64 = 5 * 60 * 1000;

#[derive(Default)]
struct Requests {
    active: HashMap<u64, CancellationToken>,
    cancelled_before_start: HashSet<u64>,
}

pub(crate) struct ReqwestHttpHost {
    client: reqwest::Client,
    api_key: Option<String>,
    requests: Mutex<Requests>,
}

impl std::fmt::Debug for ReqwestHttpHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReqwestHttpHost")
            .finish_non_exhaustive()
    }
}

impl ReqwestHttpHost {
    pub(crate) fn new(api_key: Option<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to create the HTTP client")?;
        Ok(Self {
            client,
            api_key,
            requests: Mutex::new(Requests::default()),
        })
    }

    fn begin(&self, id: u64) -> Result<CancellationToken, HttpHostError> {
        let mut requests = self.requests.lock().map_err(|_| {
            host_error(
                HttpHostErrorKind::Other,
                "HTTP request registry is unavailable",
            )
        })?;
        if requests.cancelled_before_start.remove(&id) {
            return Err(host_error(
                HttpHostErrorKind::Cancelled,
                "HTTP request was cancelled",
            ));
        }
        if requests.active.contains_key(&id) {
            return Err(host_error(
                HttpHostErrorKind::PolicyViolation,
                "duplicate HTTP request ID",
            ));
        }

        let token = CancellationToken::new();
        requests.active.insert(id, token.clone());
        Ok(token)
    }

    fn finish(&self, id: u64) {
        if let Ok(mut requests) = self.requests.lock() {
            requests.active.remove(&id);
        }
    }

    async fn execute(
        &self,
        request: &HttpRequest,
        cancellation: CancellationToken,
    ) -> Result<HttpResponse, HttpHostError> {
        let url = Url::parse(&request.url).map_err(|_| {
            host_error(HttpHostErrorKind::PolicyViolation, "request URL is invalid")
        })?;
        if url.scheme() != "https" {
            return Err(host_error(
                HttpHostErrorKind::PolicyViolation,
                "only HTTPS requests are allowed",
            ));
        }
        if request.body.len() > MAX_REQUEST_BODY_BYTES {
            return Err(host_error(
                HttpHostErrorKind::PolicyViolation,
                "request body is too large",
            ));
        }
        if request.timeout_ms == 0 || request.timeout_ms > MAX_REQUEST_TIMEOUT_MS {
            return Err(host_error(
                HttpHostErrorKind::PolicyViolation,
                "request timeout is invalid",
            ));
        }

        let mut builder = match request.method {
            HttpMethod::Get => self.client.get(url.clone()),
            HttpMethod::Post => self.client.post(url.clone()).body(request.body.clone()),
        };
        builder = builder.timeout(std::time::Duration::from_millis(request.timeout_ms));
        for header in &request.headers {
            let name = HeaderName::from_bytes(header.name.as_bytes()).map_err(|_| {
                host_error(
                    HttpHostErrorKind::PolicyViolation,
                    "request header name is invalid",
                )
            })?;
            if matches!(
                name.as_str().to_ascii_lowercase().as_str(),
                "authorization" | "cookie" | "x-api-key"
            ) {
                return Err(host_error(
                    HttpHostErrorKind::PolicyViolation,
                    "request contains a reserved header",
                ));
            }
            let value = HeaderValue::from_str(&header.value).map_err(|_| {
                host_error(
                    HttpHostErrorKind::PolicyViolation,
                    "request header value is invalid",
                )
            })?;
            builder = builder.header(name, value);
        }

        if let Some(key) = self.api_key.as_deref()
            && is_standard_toncenter_origin(&url)
        {
            builder = builder.header("X-API-Key", key);
        }

        let response = tokio::select! {
            () = cancellation.cancelled() => return Err(host_error(HttpHostErrorKind::Cancelled, "HTTP request was cancelled")),
            result = builder.send() => result.map_err(map_reqwest_error)?,
        };
        if response.status().is_redirection() {
            return Err(host_error(
                HttpHostErrorKind::PolicyViolation,
                "HTTP redirects are not allowed",
            ));
        }

        let status = response.status().as_u16();
        let final_url = response.url().to_string();
        if response.headers().len() > MAX_RESPONSE_HEADERS {
            return Err(host_error(
                HttpHostErrorKind::ResponseTooLarge,
                "response contains too many headers",
            ));
        }
        let mut headers = Vec::with_capacity(response.headers().len());
        for (name, value) in response.headers() {
            let value = value.to_str().map_err(|_| {
                host_error(
                    HttpHostErrorKind::PolicyViolation,
                    "response header value is not text",
                )
            })?;
            if self.api_key.as_deref() == Some(value) {
                continue;
            }
            headers.push(HttpHeader {
                name: name.as_str().to_owned(),
                value: value.to_owned(),
            });
        }
        let header_bytes = headers
            .iter()
            .map(|header| header.name.len() + header.value.len())
            .sum::<usize>();
        if header_bytes as u64 > request.max_response_header_bytes {
            return Err(host_error(
                HttpHostErrorKind::ResponseTooLarge,
                "response headers are too large",
            ));
        }

        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        loop {
            let chunk = tokio::select! {
                () = cancellation.cancelled() => return Err(host_error(HttpHostErrorKind::Cancelled, "HTTP request was cancelled")),
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = chunk else {
                break;
            };
            let chunk = chunk.map_err(map_reqwest_error)?;
            if body.len().saturating_add(chunk.len()) as u64 > request.max_response_body_bytes {
                return Err(host_error(
                    HttpHostErrorKind::ResponseTooLarge,
                    "response body is too large",
                ));
            }
            body.extend_from_slice(&chunk);
        }

        Ok(HttpResponse {
            status,
            headers,
            body,
            final_url,
        })
    }
}

#[async_trait]
impl WalletHttpHost for ReqwestHttpHost {
    async fn execute_http(&self, request: HttpRequest) -> Result<HttpResponse, HttpHostError> {
        let id = request.id.value;
        let cancellation = self.begin(id)?;
        let result = self.execute(&request, cancellation).await;
        self.finish(id);
        result
    }

    async fn cancel_http(&self, request_id: HttpRequestId) {
        if let Ok(mut requests) = self.requests.lock() {
            if let Some(cancellation) = requests.active.remove(&request_id.value) {
                cancellation.cancel();
            } else {
                requests.cancelled_before_start.insert(request_id.value);
            }
        }
    }
}

fn is_standard_toncenter_origin(url: &Url) -> bool {
    url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some("toncenter.com" | "testnet.toncenter.com")
        )
}

fn map_reqwest_error(error: reqwest::Error) -> HttpHostError {
    let kind = if error.is_timeout() {
        HttpHostErrorKind::Timeout
    } else if error.is_connect() {
        HttpHostErrorKind::ConnectionLost
    } else {
        HttpHostErrorKind::Other
    };
    host_error(kind, &error.to_string())
}

fn host_error(kind: HttpHostErrorKind, diagnostic: &str) -> HttpHostError {
    HttpHostError::Failed {
        kind,
        diagnostic: diagnostic.to_owned(),
    }
}
