//! NFT items owned by a wallet account.

use std::collections::HashMap;

use crate::{TonAddressString, UnsignedDecimalString};

/// Chain-derived metadata for an NFT collection.
///
/// Product-specific classification, such as Telegram gifts or usernames,
/// intentionally stays outside the engine.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NftCollectionDescriptor {
    /// The NFT collection contract address.
    pub address: TonAddressString,
    /// The standard TEP-64 collection name, when available.
    pub name: Option<String>,
    /// The standard TEP-64 collection description, when available.
    pub description: Option<String>,
    /// The standard TEP-64 collection image reference, when available.
    pub image: Option<String>,
    /// All string-valued collection metadata returned by the provider.
    pub content: HashMap<String, String>,
}

/// One NFT item returned by the configured Toncenter v3 provider.
///
/// `content` contains the string-valued on-chain and indexed metadata fields.
/// Common keys include `name`, `description`, `image`, `image_url`, `preview`,
/// `uri`, and `collection_name`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NftItem {
    /// The NFT item contract address.
    pub address: TonAddressString,
    /// The collection contract address, if this item belongs to a collection.
    pub collection_address: Option<TonAddressString>,
    /// Chain-derived collection metadata, if this item belongs to a collection.
    pub collection: Option<NftCollectionDescriptor>,
    /// The current owner reported by the NFT item contract.
    pub owner_address: Option<TonAddressString>,
    /// The effective owner while a sale contract owns the item.
    pub real_owner: Option<TonAddressString>,
    /// The active sale contract address, if the item is on sale.
    pub sale_contract_address: Option<TonAddressString>,
    /// The active auction contract address, if the item is in an auction.
    pub auction_contract_address: Option<TonAddressString>,
    /// The item index inside its collection.
    pub index: UnsignedDecimalString,
    /// The logical time of the item's latest indexed transaction.
    pub last_transaction_lt: UnsignedDecimalString,
    /// Whether the item contract has initialized NFT data.
    pub initialized: bool,
    /// Whether the provider reports an active sale or auction.
    pub on_sale: bool,
    /// The item contract code hash in the provider representation.
    pub code_hash: String,
    /// The item contract data hash in the provider representation.
    pub data_hash: String,
    /// String-valued item metadata after Toncenter metadata enrichment.
    pub content: HashMap<String, String>,
    /// The provider's content-safety classification, when available.
    pub is_nsfw: Option<bool>,
    /// The provider's scam classification, when available.
    pub is_scam: Option<bool>,
}
