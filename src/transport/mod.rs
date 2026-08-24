//! Host-owned network transport boundaries.

use async_trait::async_trait;

use crate::DomainError;

mod http;
mod request;

pub use http::*;

pub(crate) use http::HttpTransport;
pub(crate) use request::{
    build_toncenter_url, build_toncenter_v2_request, build_toncenter_v3_request,
};

/// Internal boundary between wallet operations and a concrete provider transport.
#[async_trait]
pub(crate) trait ProviderTransport: Send + Sync {
    /// Executes one provider request and returns its validated response body.
    async fn execute(&self, request: &HttpRequest) -> Result<Vec<u8>, DomainError>;

    /// Requests cancellation of an in-flight provider request.
    async fn cancel(&self, request_id: HttpRequestId);
}
