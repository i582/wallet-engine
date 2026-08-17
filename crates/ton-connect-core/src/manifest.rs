use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;
use thiserror::Error;
use url::Host;

use crate::HttpsUrl;

/// Semantic manifest validation failure beyond the scalar JSON schema.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ManifestError {
    /// The display name is empty.
    #[error("manifest name must not be empty")]
    EmptyName,
    /// The app URL cannot produce a valid external dApp domain.
    #[error("manifest app URL must contain a dotted DNS domain")]
    InvalidAppDomain,
    /// The icon URL explicitly points to an SVG image.
    #[error("manifest icon URL must not point to an SVG image")]
    SvgIcon,
    /// The reserved top-level `version` field is present.
    #[error("manifest version field is reserved and must be absent")]
    ReservedVersion,
    /// A previously validated URL could not be decomposed.
    #[error("manifest contains an invalid HTTPS URL")]
    InvalidUrl,
}

/// Validated contents of `tonconnect-manifest.json`.
///
/// Unknown fields are retained for forward compatibility. The reserved
/// `version` field is rejected as required by the current specification.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppManifest {
    /// dApp URL used as its identity and proof domain source.
    url: HttpsUrl,
    /// Human-readable dApp name.
    name: String,
    /// HTTPS raster icon URL.
    icon_url: HttpsUrl,
    /// Optional terms-of-use URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    terms_of_use_url: Option<HttpsUrl>,
    /// Optional privacy-policy URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    privacy_policy_url: Option<HttpsUrl>,
    /// Forward-compatible unknown top-level fields.
    #[serde(flatten)]
    extensions: BTreeMap<String, Value>,
}

impl AppManifest {
    /// Creates and validates a manifest assembled by the wallet host.
    pub fn new(
        url: HttpsUrl,
        name: String,
        icon_url: HttpsUrl,
        terms_of_use_url: Option<HttpsUrl>,
        privacy_policy_url: Option<HttpsUrl>,
        extensions: BTreeMap<String, Value>,
    ) -> Result<Self, ManifestError> {
        let manifest = Self {
            url,
            name,
            icon_url,
            terms_of_use_url,
            privacy_policy_url,
            extensions,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates a fully assembled manifest.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.name.is_empty() {
            return Err(ManifestError::EmptyName);
        }
        if self.extensions.contains_key("version") {
            return Err(ManifestError::ReservedVersion);
        }

        let app_url = self.url.parsed().map_err(|_| ManifestError::InvalidUrl)?;
        let valid_domain = matches!(app_url.host(), Some(Host::Domain(domain)) if {
            let mut labels = domain.split('.');
            labels.next().is_some_and(|label| !label.is_empty())
                && labels.next().is_some_and(|label| !label.is_empty())
                && labels.all(|label| !label.is_empty())
        });
        if !valid_domain {
            return Err(ManifestError::InvalidAppDomain);
        }

        let icon_url = self
            .icon_url
            .parsed()
            .map_err(|_| ManifestError::InvalidUrl)?;
        if icon_url.path().to_ascii_lowercase().ends_with(".svg") {
            return Err(ManifestError::SvgIcon);
        }
        Ok(())
    }

    /// Returns the normalized DNS domain bound into signatures.
    pub fn app_domain(&self) -> Result<String, ManifestError> {
        let url = self.url.parsed().map_err(|_| ManifestError::InvalidUrl)?;
        match url.host() {
            Some(Host::Domain(domain)) => Ok(domain.to_owned()),
            Some(Host::Ipv4(_) | Host::Ipv6(_)) | None => Err(ManifestError::InvalidAppDomain),
        }
    }

    /// Returns the dApp URL.
    #[must_use]
    pub const fn url(&self) -> &HttpsUrl {
        &self.url
    }

    /// Returns the display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the icon URL.
    #[must_use]
    pub const fn icon_url(&self) -> &HttpsUrl {
        &self.icon_url
    }

    /// Returns the optional terms-of-use URL.
    #[must_use]
    pub const fn terms_of_use_url(&self) -> Option<&HttpsUrl> {
        self.terms_of_use_url.as_ref()
    }

    /// Returns the optional privacy-policy URL.
    #[must_use]
    pub const fn privacy_policy_url(&self) -> Option<&HttpsUrl> {
        self.privacy_policy_url.as_ref()
    }

    /// Returns forward-compatible unknown fields.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<String, Value> {
        &self.extensions
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppManifestWire {
    url: HttpsUrl,
    name: String,
    icon_url: HttpsUrl,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null"
    )]
    terms_of_use_url: Option<HttpsUrl>,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null"
    )]
    privacy_policy_url: Option<HttpsUrl>,
    #[serde(flatten)]
    extensions: BTreeMap<String, Value>,
}

impl<'de> Deserialize<'de> for AppManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AppManifestWire::deserialize(deserializer)?;
        let manifest = Self {
            url: wire.url,
            name: wire.name,
            icon_url: wire.icon_url,
            terms_of_use_url: wire.terms_of_use_url,
            privacy_policy_url: wire.privacy_policy_url,
            extensions: wire.extensions,
        };
        manifest.validate().map_err(de::Error::custom)?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_preserves_extensions_and_exposes_proof_domain() {
        let json = r#"{
            "url":"https://app.example.com",
            "name":"Example",
            "iconUrl":"https://cdn.example.com/icon.png",
            "category":"defi"
        }"#;
        let manifest = serde_json::from_str::<AppManifest>(json);
        assert_eq!(
            manifest
                .as_ref()
                .ok()
                .and_then(|value| value.extensions().get("category")),
            Some(&Value::String("defi".to_owned()))
        );
        assert_eq!(
            manifest
                .as_ref()
                .ok()
                .and_then(|value| value.app_domain().ok()),
            Some("app.example.com".to_owned())
        );
    }

    #[test]
    fn manifest_rejects_reserved_version_http_and_svg() {
        let version = r#"{"url":"https://example.com","name":"App","iconUrl":"https://example.com/icon.png","version":2}"#;
        let http =
            r#"{"url":"http://example.com","name":"App","iconUrl":"https://example.com/icon.png"}"#;
        let svg = r#"{"url":"https://example.com","name":"App","iconUrl":"https://example.com/icon.SVG"}"#;
        assert!(serde_json::from_str::<AppManifest>(version).is_err());
        assert!(serde_json::from_str::<AppManifest>(http).is_err());
        assert!(serde_json::from_str::<AppManifest>(svg).is_err());
    }

    #[test]
    fn constructor_and_accessors_cover_optional_manifest_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        let url = HttpsUrl::try_from("https://app.example.com/path")?;
        let icon = HttpsUrl::try_from("https://cdn.example.com/icon.png")?;
        let terms = HttpsUrl::try_from("https://app.example.com/terms")?;
        let privacy = HttpsUrl::try_from("https://app.example.com/privacy")?;
        let mut extensions = BTreeMap::new();
        let _ = extensions.insert("category".to_owned(), Value::String("defi".to_owned()));
        let manifest = AppManifest::new(
            url.clone(),
            "Example".to_owned(),
            icon.clone(),
            Some(terms.clone()),
            Some(privacy.clone()),
            extensions,
        )?;
        assert_eq!(manifest.url(), &url);
        assert_eq!(manifest.name(), "Example");
        assert_eq!(manifest.icon_url(), &icon);
        assert_eq!(manifest.terms_of_use_url(), Some(&terms));
        assert_eq!(manifest.privacy_policy_url(), Some(&privacy));
        assert_eq!(manifest.app_domain()?, "app.example.com");

        assert_eq!(
            AppManifest::new(url, String::new(), icon, None, None, BTreeMap::new(),),
            Err(ManifestError::EmptyName)
        );
        Ok(())
    }
}
