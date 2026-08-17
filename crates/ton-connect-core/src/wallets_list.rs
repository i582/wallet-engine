//! Validated TON Connect wallets-list registry models.

use serde::{Deserialize, Deserializer, Serialize, de};
use thiserror::Error;

use crate::{Feature, HttpsUrl, NonEmptyVec, StructuredItemType};

/// Non-empty validated TON Connect wallet registry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WalletsList(Vec<WalletInfo>);

impl WalletsList {
    /// Returns the validated registry entries.
    #[must_use]
    pub fn as_slice(&self) -> &[WalletInfo] {
        &self.0
    }
}

impl TryFrom<Vec<WalletInfo>> for WalletsList {
    type Error = WalletsListError;

    fn try_from(entries: Vec<WalletInfo>) -> Result<Self, Self::Error> {
        if entries.is_empty() {
            Err(WalletsListError::EmptyList)
        } else {
            Ok(Self(entries))
        }
    }
}

impl<'de> Deserialize<'de> for WalletsList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(Vec::<WalletInfo>::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// One validated wallet entry from `wallets-v2.json`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WalletInfo {
    app_name: String,
    name: String,
    image: HttpsUrl,
    #[serde(skip_serializing_if = "Option::is_none")]
    tondns: Option<String>,
    about_url: HttpsUrl,
    #[serde(skip_serializing_if = "Option::is_none")]
    universal_url: Option<HttpsUrl>,
    #[serde(rename = "deepLink", skip_serializing_if = "Option::is_none")]
    deep_link: Option<String>,
    bridge: NonEmptyVec<WalletBridge>,
    platforms: NonEmptyVec<WalletPlatform>,
    features: NonEmptyVec<Feature>,
}

impl WalletInfo {
    /// Returns the runtime `DeviceInfo.appName` identity.
    #[must_use]
    pub fn app_name(&self) -> &str {
        &self.app_name
    }

    /// Returns the human-readable wallet name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the configured bridges.
    #[must_use]
    pub const fn bridges(&self) -> &NonEmptyVec<WalletBridge> {
        &self.bridge
    }

    /// Returns the statically advertised protocol features.
    #[must_use]
    pub const fn features(&self) -> &NonEmptyVec<Feature> {
        &self.features
    }
}

/// Unvalidated fields used to construct a [`WalletInfo`].
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalletInfoConfig {
    /// Wallet identifier matching runtime `DeviceInfo.appName`.
    pub app_name: String,
    /// Human-readable wallet name.
    pub name: String,
    /// HTTPS PNG icon URL.
    pub image: HttpsUrl,
    /// Optional reserved TON DNS name.
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null"
    )]
    pub tondns: Option<String>,
    /// HTTPS wallet information page.
    pub about_url: HttpsUrl,
    /// HTTPS universal-link base required by an SSE bridge.
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null"
    )]
    pub universal_url: Option<HttpsUrl>,
    /// Optional custom wallet deep-link prefix.
    #[serde(
        rename = "deepLink",
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null"
    )]
    pub deep_link: Option<String>,
    /// One or two distinct bridge transports.
    pub bridge: NonEmptyVec<WalletBridge>,
    /// Non-empty unique platform list.
    pub platforms: NonEmptyVec<WalletPlatform>,
    /// Non-empty validated feature list.
    pub features: NonEmptyVec<Feature>,
}

impl TryFrom<WalletInfoConfig> for WalletInfo {
    type Error = WalletsListError;

    fn try_from(config: WalletInfoConfig) -> Result<Self, Self::Error> {
        validate_wallet_info(&config)?;
        Ok(Self {
            app_name: config.app_name,
            name: config.name,
            image: config.image,
            tondns: config.tondns,
            about_url: config.about_url,
            universal_url: config.universal_url,
            deep_link: config.deep_link,
            bridge: config.bridge,
            platforms: config.platforms,
            features: config.features,
        })
    }
}

impl<'de> Deserialize<'de> for WalletInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::try_from(WalletInfoConfig::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// Bridge transport published by a wallet registry entry.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum WalletBridge {
    /// Injected JavaScript bridge at `window.<key>.tonconnect`.
    Js {
        /// Non-empty JavaScript bridge key.
        key: String,
    },
    /// HTTPS server-sent-events bridge.
    Sse {
        /// Published HTTP bridge base URL.
        url: HttpsUrl,
    },
}

/// Platform identifier allowed by `wallets-v2.json`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WalletPlatform {
    /// Apple iOS.
    Ios,
    /// Google Android.
    Android,
    /// Chrome extension/browser.
    Chrome,
    /// Firefox extension/browser.
    Firefox,
    /// Safari extension/browser.
    Safari,
    /// Apple macOS.
    Macos,
    /// Microsoft Windows.
    Windows,
    /// Linux desktop.
    Linux,
}

/// Wallet registry entry violates `wallets-v2.schema.json` semantics.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum WalletsListError {
    /// Registry has no wallet entries.
    #[error("wallets list must contain at least one entry")]
    EmptyList,
    /// Required wallet identity text is empty.
    #[error("wallet app_name and name must be non-empty")]
    EmptyIdentity,
    /// Wallet image is not an HTTPS URL ending in lowercase `.png`.
    #[error("wallet image must be an HTTPS .png URL")]
    InvalidImage,
    /// Optional TON DNS or deep-link field is malformed.
    #[error("wallet optional identity field is malformed")]
    InvalidOptionalIdentity,
    /// Bridge list violates cardinality, uniqueness, or universal-link rules.
    #[error("wallet bridge configuration is invalid")]
    InvalidBridge,
    /// Platform entries are not unique.
    #[error("wallet platforms must be unique")]
    DuplicatePlatform,
    /// Feature limits, arrays, or uniqueness are invalid.
    #[error("wallet feature configuration is invalid")]
    InvalidFeature,
}

#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "the normative wallets-list schema requires lowercase .png and .ton suffixes"
)]
fn validate_wallet_info(config: &WalletInfoConfig) -> Result<(), WalletsListError> {
    if config.app_name.is_empty() || config.name.is_empty() {
        return Err(WalletsListError::EmptyIdentity);
    }
    if !config.image.as_str().ends_with(".png") {
        return Err(WalletsListError::InvalidImage);
    }
    if config
        .tondns
        .as_ref()
        .is_some_and(|value| value == ".ton" || !value.ends_with(".ton"))
        || config.deep_link.as_ref().is_some_and(String::is_empty)
    {
        return Err(WalletsListError::InvalidOptionalIdentity);
    }

    let bridges = config.bridge.as_slice();
    if bridges.len() > 2 {
        return Err(WalletsListError::InvalidBridge);
    }
    let js_count = bridges
        .iter()
        .filter(|bridge| matches!(bridge, WalletBridge::Js { .. }))
        .count();
    let sse_count = bridges
        .iter()
        .filter(|bridge| matches!(bridge, WalletBridge::Sse { .. }))
        .count();
    if js_count > 1
        || sse_count > 1
        || (sse_count == 1 && config.universal_url.is_none())
        || bridges
            .iter()
            .any(|bridge| matches!(bridge, WalletBridge::Js { key } if key.is_empty()))
    {
        return Err(WalletsListError::InvalidBridge);
    }
    if !all_unique(config.platforms.as_slice()) {
        return Err(WalletsListError::DuplicatePlatform);
    }
    validate_features(config.features.as_slice())
}

fn validate_features(features: &[Feature]) -> Result<(), WalletsListError> {
    let mut names = Vec::new();
    for feature in features {
        let name = match feature {
            Feature::LegacySendTransaction => return Err(WalletsListError::InvalidFeature),
            Feature::SendTransaction(value) => {
                validate_message_feature(value.max_messages(), value.item_types())?;
                "SendTransaction"
            }
            Feature::SignMessage(value) => {
                validate_message_feature(value.max_messages(), value.item_types())?;
                "SignMessage"
            }
            Feature::SignData(value) => {
                if value.types().is_empty() || !all_unique(value.types()) {
                    return Err(WalletsListError::InvalidFeature);
                }
                "SignData"
            }
            Feature::EmbeddedRequest => "EmbeddedRequest",
        };
        if names.contains(&name) {
            return Err(WalletsListError::InvalidFeature);
        }
        names.push(name);
    }
    Ok(())
}

fn validate_message_feature(
    max_messages: u32,
    item_types: Option<&[StructuredItemType]>,
) -> Result<(), WalletsListError> {
    if max_messages == 0
        || item_types.is_some_and(|values| values.is_empty() || !all_unique(values))
    {
        Err(WalletsListError::InvalidFeature)
    } else {
        Ok(())
    }
}

fn all_unique<T: Eq>(values: &[T]) -> bool {
    values.iter().enumerate().all(|(index, value)| {
        values
            .iter()
            .skip(index.saturating_add(1))
            .all(|other| other != value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"[{
        "app_name":"examplewallet",
        "name":"Example Wallet",
        "image":"https://wallet.example/icon.png",
        "about_url":"https://wallet.example/about",
        "universal_url":"https://wallet.example/connect",
        "deepLink":"examplewallet-tc://",
        "bridge":[
            {"type":"js","key":"examplewallet"},
            {"type":"sse","url":"https://bridge.wallet.example/bridge"}
        ],
        "platforms":["ios","android"],
        "features":[
            {"name":"SendTransaction","maxMessages":4,"itemTypes":["ton"]},
            {"name":"SignData","types":["text","binary","cell"]},
            {"name":"EmbeddedRequest"}
        ]
    }]"#;

    #[test]
    fn valid_registry_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let wallets = serde_json::from_str::<WalletsList>(VALID)?;
        assert_eq!(wallets.as_slice().len(), 1);
        assert_eq!(
            wallets.as_slice().first().map(WalletInfo::app_name),
            Some("examplewallet")
        );
        assert!(serde_json::from_str::<WalletsList>(&serde_json::to_string(&wallets)?).is_ok());
        Ok(())
    }

    #[test]
    fn rejects_schema_semantic_violations() {
        let cases = [
            "[]",
            &VALID.replace("icon.png", "icon.svg"),
            &VALID.replace(
                r#"        "universal_url":"https://wallet.example/connect","#,
                "",
            ),
            &VALID.replace(r#"["ios","android"]"#, r#"["ios","ios"]"#),
            &VALID.replace(r#""maxMessages":4"#, r#""maxMessages":0"#),
        ];
        for case in cases {
            assert!(
                serde_json::from_str::<WalletsList>(case).is_err(),
                "accepted invalid wallets list: {case}"
            );
        }
    }
}
