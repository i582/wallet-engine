//! Toncenter HTTP request construction.

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
/// Callers provide the complete API-specific path as fixed, URL-safe segments. This
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
    // The configured base is already serialized; the host HTTP stack parses it.
    // Reject delimiters that would put the appended API path outside its path.
    let authority = base
        .strip_prefix("https://")
        .or_else(|| base.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .ok_or(WalletClientError::InvalidProviderBaseUrl)?;
    if authority.is_empty()
        || authority.starts_with(':')
        || authority.contains('@')
        || !base.is_ascii()
        || base
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b' ' | b'?' | b'#' | b'\\'))
    {
        return Err(WalletClientError::InvalidProviderBaseUrl);
    }

    let mut url = base.strip_suffix('/').unwrap_or(base).to_owned();
    for segment in path {
        url.push('/');
        url.push_str(segment);
    }
    for (index, (key, value)) in query.iter().enumerate() {
        url.push(if index == 0 { '?' } else { '&' });
        append_encoded_query_component(&mut url, key);
        url.push('=');
        append_encoded_query_component(&mut url, value);
    }

    Ok(url)
}

/// Matches `application/x-www-form-urlencoded`: spaces become `+`, literal `+`
/// becomes `%2B`, and UTF-8 bytes are escaped individually with uppercase hex.
fn append_encoded_query_component(output: &mut String, value: &str) {
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                output.push(char::from(byte));
            }
            b' ' => output.push('+'),
            _ => {
                output.push('%');
                for nibble in [byte >> 4, byte & 0x0f] {
                    output.push(char::from(if nibble < 10 {
                        b'0'.wrapping_add(nibble)
                    } else {
                        b'A'.wrapping_add(nibble.wrapping_sub(10))
                    }));
                }
            }
        }
    }
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

    #[test]
    fn encodes_query_data_without_changing_parameter_boundaries() {
        assert_eq!(
            build_provider_url(
                "https://provider.example",
                &["api", "v3", "transactionsByMessage"],
                &[
                    ("msg_hash", "a+/="),
                    ("text &", "hello 🌍?#%~"),
                    ("empty", "")
                ],
            )
            .expect("valid provider base"),
            "https://provider.example/api/v3/transactionsByMessage?msg_hash=a%2B%2F%3D&text+%26=hello+%F0%9F%8C%8D%3F%23%25%7E&empty="
        );
    }

    #[test]
    fn preserves_serialized_bases_and_nested_endpoints() {
        for base in [
            "https://provider.example/custom%20prefix",
            "https://provider.example/custom/",
            "https://provider.example/custom//",
            "http://127.0.0.1:8080",
            "http://[::1]:8080/",
        ] {
            let request = build_toncenter_v3_request(
                &config(base),
                HttpRequestId { value: 1 },
                "nft/items",
                &[],
            )
            .expect("valid provider base");
            assert_eq!(
                request.url,
                format!(
                    "{}/api/v3/nft/items",
                    base.strip_suffix('/').unwrap_or(base)
                ),
            );
        }
    }

    #[test]
    fn rejects_bases_that_are_not_serialized_http_endpoints() {
        for base in [
            "https://",
            "https:///custom",
            "https://:443",
            "https://user@provider.example",
            "https://provider.example?key=value",
            "https://provider.example/#fragment",
            "https://provider.example/custom path",
            "https://provider.example/\r\n",
            "https://provider.example/\u{7f}",
            "https://provider.example\\custom",
            "https://例子.example",
        ] {
            assert_eq!(
                build_provider_url(base, &["api", "v2", "getTransactions"], &[]),
                Err(WalletClientError::InvalidProviderBaseUrl),
                "{base:?}",
            );
        }
    }

    proptest::proptest! {
        #[test]
        fn query_encoding_matches_url_crate(key in ".*", value in ".*") {
            let base = "https://provider.example/custom/";
            let query = [(key.as_str(), value.as_str()), ("repeat", "1"), ("repeat", "2")];
            let actual = build_provider_url(base, &["api", "v2", "getTransactions"], &query)
                .expect("valid provider base");
            let mut expected = url::Url::parse(base).expect("valid provider base");
            let _ = expected.path_segments_mut().expect("hierarchical URL")
                .pop_if_empty().extend(["api", "v2", "getTransactions"]);
            let _ = expected.query_pairs_mut().extend_pairs(query);
            proptest::prop_assert_eq!(actual, expected.as_str());
        }
    }
}
