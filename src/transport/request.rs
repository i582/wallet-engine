//! Toncenter HTTP request construction.

use url::Url;

use crate::{WalletClientConfig, WalletClientError};

use super::http::{HttpHeader, HttpMethod, HttpRequest, HttpRequestId};

pub(crate) fn build_toncenter_v2_request(
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
pub(crate) fn build_toncenter_v3_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    path: &str,
    query: &[(&str, &str)],
) -> Result<HttpRequest, WalletClientError> {
    let mut path_segments = vec!["api", "v3"];
    path_segments.extend(path.split('/').filter(|segment| !segment.is_empty()));
    Ok(HttpRequest {
        id,
        method: HttpMethod::Get,
        url: build_toncenter_url(config, &path_segments, query)?,
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
pub(crate) fn build_toncenter_url(
    config: &WalletClientConfig,
    path: &[&str],
    query: &[(&str, &str)],
) -> Result<String, WalletClientError> {
    build_provider_url(&config.providers.toncenter_base_url, path, query)
}

fn build_provider_url(
    base: &str,
    path: &[&str],
    query: &[(&str, &str)],
) -> Result<String, WalletClientError> {
    let mut url = Url::parse(base).map_err(|_| WalletClientError::InvalidProviderBaseUrl)?;

    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| WalletClientError::InvalidProviderBaseUrl)?;
        let _ = segments.pop_if_empty();
        for segment in path {
            let _ = segments.push(segment);
        }
    }

    url.set_query(None);
    if !query.is_empty() {
        let mut query_pairs = url.query_pairs_mut();
        let _ = query_pairs.extend_pairs(query.iter().copied());
    }

    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Network, ProviderConfig};

    fn config(base_url: &str) -> WalletClientConfig {
        WalletClientConfig {
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
                toncenter_base_url: base_url.to_owned(),
                dns_root_address: None,
                request_timeout_ms: 12_345,
            },
        }
    }

    #[test]
    fn builds_a_toncenter_request_without_losing_the_base_path() {
        let request = build_toncenter_v2_request(
            &config("https://provider.example/custom/"),
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
    fn builds_v3_below_the_same_deployment_base() {
        let request = build_toncenter_v3_request(
            &config("https://provider.example/custom"),
            HttpRequestId { value: 10 },
            "transactionsByMessage",
            &[("direction", "in")],
        )
        .expect("the provider URL is valid");

        assert_eq!(
            request.url,
            "https://provider.example/custom/api/v3/transactionsByMessage?direction=in"
        );
    }

    #[test]
    fn rejects_a_base_without_hierarchical_path_segments() {
        assert_eq!(
            build_toncenter_v2_request(
                &config("mailto:provider@example.com"),
                HttpRequestId { value: 1 },
                "resource",
                &[],
            ),
            Err(WalletClientError::InvalidProviderBaseUrl)
        );
    }
}
