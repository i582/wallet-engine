use std::collections::BTreeMap;

use thiserror::Error;

use crate::{
    ClientId, ConnectRequest, EmbeddedRequest, EmbeddedRequestError, TraceId, ValueError,
    decode_embedded_request_param,
};

const PROTOCOL_VERSION: &str = "2";

/// Action the wallet takes after the connect prompt completes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReturnStrategy {
    /// Return to the application that opened the wallet.
    Back,
    /// Do not perform a return jump.
    None,
    /// Open an explicit absolute application URL.
    Custom(String),
}

/// A parsed TON Connect universal, unified, or custom-scheme link.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectLink {
    client_id: ClientId,
    request: Option<ConnectRequest>,
    return_strategy: ReturnStrategy,
    embedded_request: Option<EmbeddedRequest>,
    trace_id: Option<TraceId>,
    extensions: Vec<(String, String)>,
}

impl ConnectLink {
    /// Parses both a full connect link and the protocol's reduced `id`/`ret` link.
    pub fn parse(value: &str) -> Result<Self, ConnectLinkError> {
        let url = url::Url::parse(value)?;
        let mut known = BTreeMap::<String, String>::new();
        let mut extensions = Vec::new();
        for (name, value) in url.query_pairs() {
            let name = name.into_owned();
            let value = value.into_owned();
            if matches!(name.as_str(), "v" | "id" | "r" | "ret" | "e" | "trace_id") {
                if known.insert(name.clone(), value).is_some() {
                    return Err(ConnectLinkError::DuplicateParameter(name));
                }
            } else {
                extensions.push((name, value));
            }
        }

        let client_id = known
            .remove("id")
            .ok_or(ConnectLinkError::MissingParameter("id"))?
            .parse()?;
        let request = known
            .remove("r")
            .map(|request| serde_json::from_str::<ConnectRequest>(&request))
            .transpose()?;
        let version = known.remove("v");
        if let Some(version) = version.as_deref()
            && version != PROTOCOL_VERSION
        {
            return Err(ConnectLinkError::UnsupportedVersion(version.to_owned()));
        }
        if request.is_some() && version.is_none() {
            return Err(ConnectLinkError::MissingParameter("v"));
        }

        // Malformed or unsupported embedded requests are deliberately ignored.
        // The dApp SDK then falls back to ordinary bridge RPC after connect.
        let embedded_request = known
            .remove("e")
            .and_then(|parameter| decode_embedded_request_param(&parameter).ok());
        if embedded_request.is_some() && request.is_none() {
            return Err(ConnectLinkError::EmbeddedRequestWithoutConnect);
        }

        let return_parameter = known.remove("ret");
        let return_strategy = parse_return_strategy(return_parameter.as_deref())?;
        let trace_id = known
            .remove("trace_id")
            .map(TraceId::try_from)
            .transpose()?;

        Ok(Self {
            client_id,
            request,
            return_strategy,
            embedded_request,
            trace_id,
            extensions,
        })
    }

    /// Returns the dApp's bridge client identifier.
    #[must_use]
    pub const fn client_id(&self) -> ClientId {
        self.client_id
    }

    /// Returns the connect request, absent only for reduced `id`/`ret` links.
    #[must_use]
    pub const fn request(&self) -> Option<&ConnectRequest> {
        self.request.as_ref()
    }

    /// Returns the post-action navigation strategy.
    #[must_use]
    pub const fn return_strategy(&self) -> &ReturnStrategy {
        &self.return_strategy
    }

    /// Returns the optional one-tap request.
    #[must_use]
    pub const fn embedded_request(&self) -> Option<&EmbeddedRequest> {
        self.embedded_request.as_ref()
    }

    /// Returns the optional analytics correlation identifier.
    #[must_use]
    pub const fn trace_id(&self) -> Option<&TraceId> {
        self.trace_id.as_ref()
    }

    /// Returns unknown query parameters retained for forward compatibility.
    #[must_use]
    pub fn extensions(&self) -> &[(String, String)] {
        &self.extensions
    }
}

fn parse_return_strategy(value: Option<&str>) -> Result<ReturnStrategy, ConnectLinkError> {
    match value {
        None | Some("back") => Ok(ReturnStrategy::Back),
        Some("none") => Ok(ReturnStrategy::None),
        Some(custom) => {
            let parsed = url::Url::parse(custom)
                .map_err(|_| ConnectLinkError::InvalidReturnStrategy(custom.to_owned()))?;
            if parsed.scheme().is_empty() {
                return Err(ConnectLinkError::InvalidReturnStrategy(custom.to_owned()));
            }
            Ok(ReturnStrategy::Custom(custom.to_owned()))
        }
    }
}

/// Failure to parse a TON Connect link.
#[derive(Debug, Error)]
pub enum ConnectLinkError {
    /// The outer link itself is not an absolute URL.
    #[error("invalid TON Connect URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    /// A required query parameter is absent.
    #[error("missing TON Connect query parameter: {0}")]
    MissingParameter(&'static str),
    /// A singleton protocol query parameter appeared more than once.
    #[error("duplicate TON Connect query parameter: {0}")]
    DuplicateParameter(String),
    /// The link requests a protocol version other than version 2.
    #[error("unsupported TON Connect protocol version: {0}")]
    UnsupportedVersion(String),
    /// A compact request appeared without a connect request.
    #[error("embedded request requires a connect request")]
    EmbeddedRequestWithoutConnect,
    /// The custom return target is not an absolute URL.
    #[error("invalid TON Connect return strategy: {0}")]
    InvalidReturnStrategy(String),
    /// A scalar protocol value is malformed.
    #[error(transparent)]
    InvalidValue(#[from] ValueError),
    /// The URL-decoded connect request violates its JSON schema.
    #[error("invalid TON Connect request: {0}")]
    InvalidConnectRequest(#[from] serde_json::Error),
    /// The compact embedded request is malformed.
    #[error(transparent)]
    InvalidEmbeddedRequest(#[from] EmbeddedRequestError),
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose};

    use super::*;

    const CLIENT_ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn parses_unified_link_and_percent_decoded_connect_request() {
        let request = r#"{"manifestUrl":"https://app.example/tonconnect-manifest.json","items":[{"name":"ton_addr"}]}"#;
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("v", "2")
            .append_pair("id", CLIENT_ID)
            .append_pair("r", request)
            .append_pair("ret", "back")
            .finish();
        let parsed = ConnectLink::parse(&format!("tc://?{query}"));
        assert!(parsed.as_ref().is_ok_and(|link| link.request().is_some()));
        assert!(matches!(
            parsed.as_ref().map(ConnectLink::return_strategy),
            Ok(ReturnStrategy::Back)
        ));
    }

    #[test]
    fn supports_reduced_link_but_not_embedded_action_without_connect() {
        let reduced = format!("tc://?id={CLIENT_ID}&ret=none");
        let embedded = general_purpose::URL_SAFE_NO_PAD.encode(r#"{"m":"sd","t":"text","tx":"x"}"#);
        let invalid = format!("tc://?id={CLIENT_ID}&e={embedded}");
        assert!(ConnectLink::parse(&reduced).is_ok_and(|link| link.request().is_none()));
        assert!(matches!(
            ConnectLink::parse(&invalid),
            Err(ConnectLinkError::EmbeddedRequestWithoutConnect)
        ));
    }

    #[test]
    fn rejects_duplicate_identifiers_and_unknown_versions() {
        let duplicate = format!("tc://?id={CLIENT_ID}&id={CLIENT_ID}");
        let version = format!("tc://?id={CLIENT_ID}&v=3");
        assert!(matches!(
            ConnectLink::parse(&duplicate),
            Err(ConnectLinkError::DuplicateParameter(parameter)) if parameter == "id"
        ));
        assert!(matches!(
            ConnectLink::parse(&version),
            Err(ConnectLinkError::UnsupportedVersion(version)) if version == "3"
        ));
    }

    #[test]
    fn malformed_embedded_request_is_ignored_for_bridge_fallback() {
        let request = r#"{"manifestUrl":"https://app.example/tonconnect-manifest.json","items":[{"name":"ton_addr"}]}"#;
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("v", "2")
            .append_pair("id", CLIENT_ID)
            .append_pair("r", request)
            .append_pair("e", "not-base64url")
            .finish();
        let parsed = ConnectLink::parse(&format!("tc://?{query}"));

        assert!(parsed.is_ok_and(|link| link.embedded_request().is_none()));
    }
}
