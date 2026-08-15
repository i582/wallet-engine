//! Jetton balances and untrusted token metadata.

/// Display metadata reported by the configured Toncenter indexer.
///
/// Metadata is not a token identity or proof of authenticity. Applications
/// must identify a jetton by [`JettonBalance::master_address`] and treat every
/// string in this record as untrusted external content.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct JettonMetadata {
    /// The optional display name.
    pub name: Option<String>,
    /// The optional display ticker.
    pub symbol: Option<String>,
    /// The optional display description.
    pub description: Option<String>,
    /// The optional external image URL.
    pub image_url: Option<String>,
    /// The optional number of fractional decimal places.
    pub decimals: Option<u8>,
    /// Whether the indexer marks the metadata as a scam.
    pub is_scam: Option<bool>,
    /// Whether the indexer marks the metadata as not safe for work.
    pub is_nsfw: Option<bool>,
}

/// One indexed TEP-74 jetton balance owned by the configured wallet.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct JettonBalance {
    /// The per-owner jetton wallet contract address.
    pub wallet_address: String,
    /// The jetton master contract address and stable token identity.
    pub master_address: String,
    /// The exact unsigned balance in the jetton's smallest unit.
    pub balance_units: String,
    /// Optional display metadata supplied by the indexer.
    pub metadata: Option<JettonMetadata>,
}
