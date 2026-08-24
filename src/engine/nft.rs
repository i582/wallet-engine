//! Account NFT loading and offset pagination.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use serde::Deserialize;
use serde_json::Value;
use ton::ton_core::types::TonAddress;

use crate::transport::{build_toncenter_v3_request, process_response};
use crate::{
    DomainError, ErrorCode, HttpRequest, HttpRequestId, Network, NftCollectionDescriptor, NftItem,
    ResourceState, TonAddressString, UnsignedDecimalString, WalletClientConfig, WalletClientError,
    WalletOperationOutcome, WalletUpdate,
};

use super::WalletClient;
use super::provider::invalid_response;
use super::state::{OperationFamily, State, ensure_running, nft_update};

/// Matches the account NFT batch used by Actonscan's explorer UI.
pub(super) const NFT_PAGE_SIZE: u32 = 60;

#[uniffi::export]
impl WalletClient {
    /// Loads the newest page of NFTs owned by this wallet.
    ///
    /// This operation is independent from [`Self::refresh`], so applications
    /// can load account, activity, and NFT data concurrently. A newer NFT
    /// refresh supersedes the older one and cancels its host request.
    pub async fn refresh_nfts(&self) -> Result<WalletUpdate, WalletClientError> {
        let (generation, request, previous_request_ids, network) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;

            let id = state.allocate_request_id()?;
            let request = build_nft_page_request(&state.config, id, 0)?;

            state.nft_refresh_generation = state
                .nft_refresh_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.nft_refresh_generation;

            let mut previous_request_ids = state
                .active_nft_refresh
                .replace((generation, id))
                .map(|active| vec![active.1])
                .unwrap_or_default();
            if let Some((_, page_request_id)) = state.active_nft_pagination.take() {
                previous_request_ids.push(page_request_id);
            }

            state.snapshot.nfts.resource = ResourceState::loading();
            state.snapshot.nfts.pagination_resource = ResourceState::idle();
            state.next_revision()?;

            (
                generation,
                request,
                previous_request_ids,
                state.config.network,
            )
        };

        for request_id in previous_request_ids {
            self.http_host.cancel_http(request_id).await;
        }

        let result = process_response(&request, self.http_host.execute_http(request.clone()).await)
            .and_then(|body| parse_nft_page(&body, NFT_PAGE_SIZE, network));

        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::NftRefresh, generation) {
            return Ok(nft_update(WalletOperationOutcome::Superseded, 0, &state));
        }

        state.active_nft_refresh = None;
        let outcome = match result {
            Ok(page) => {
                apply_refreshed_nft_page(&mut state, page);
                state.snapshot.nfts.resource = ResourceState::ready();
                WalletOperationOutcome::Completed
            }
            Err(error) if error.code == ErrorCode::HostCancelled => {
                state.snapshot.nfts.resource = ResourceState::idle();
                WalletOperationOutcome::Cancelled
            }
            Err(error) => {
                state.snapshot.nfts.resource = ResourceState::failed(error);
                WalletOperationOutcome::Failed
            }
        };
        state.next_revision()?;

        Ok(nft_update(outcome, 0, &state))
    }

    /// Cancels the active first-page NFT refresh.
    ///
    /// This method has no effect when no NFT refresh is active.
    pub async fn cancel_refresh_nfts(&self) -> Result<(), WalletClientError> {
        let request_id = {
            let mut state = self.lock()?;
            let request_id = state.active_nft_refresh.take().map(|active| active.1);
            if request_id.is_some() {
                state.snapshot.nfts.resource = ResourceState::idle();
                state.next_revision()?;
            }
            request_id
        };

        if let Some(request_id) = request_id {
            self.http_host.cancel_http(request_id).await;
        }

        Ok(())
    }

    /// Loads the next NFT page and appends items not already present by address.
    ///
    /// The method returns `Skipped` during an NFT refresh, during another NFT
    /// page load, or after the provider returns a short page.
    pub async fn load_more_nfts(&self) -> Result<WalletUpdate, WalletClientError> {
        let (generation, request, network) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;

            if state.active_nft_refresh.is_some()
                || state.active_nft_pagination.is_some()
                || !state.nfts_has_more
            {
                return Ok(nft_update(WalletOperationOutcome::Skipped, 0, &state));
            }

            let id = state.allocate_request_id()?;
            let request = build_nft_page_request(&state.config, id, state.nft_offset)?;

            state.nft_pagination_generation = state
                .nft_pagination_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.nft_pagination_generation;
            state.active_nft_pagination = Some((generation, id));
            state.snapshot.nfts.pagination_resource = ResourceState::loading();
            state.next_revision()?;

            (generation, request, state.config.network)
        };

        let result = process_response(&request, self.http_host.execute_http(request.clone()).await)
            .and_then(|body| parse_nft_page(&body, NFT_PAGE_SIZE, network));

        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::NftPagination, generation) {
            return Ok(nft_update(WalletOperationOutcome::Superseded, 0, &state));
        }

        state.active_nft_pagination = None;
        let (outcome, added) = match result {
            Ok(page) => {
                let added = apply_nft_page(&mut state, page);
                state.snapshot.nfts.pagination_resource = ResourceState::ready();
                (WalletOperationOutcome::Completed, added)
            }
            Err(error) if error.code == ErrorCode::HostCancelled => {
                state.snapshot.nfts.pagination_resource = ResourceState::idle();
                (WalletOperationOutcome::Cancelled, 0)
            }
            Err(error) => {
                state.snapshot.nfts.pagination_resource = ResourceState::failed(error);
                (WalletOperationOutcome::Failed, 0)
            }
        };
        state.next_revision()?;

        Ok(nft_update(outcome, added, &state))
    }

    /// Cancels the active additional-page NFT load.
    ///
    /// This method has no effect when no NFT page load is active.
    pub async fn cancel_load_more_nfts(&self) -> Result<(), WalletClientError> {
        let request_id = {
            let mut state = self.lock()?;
            let request_id = state.active_nft_pagination.take().map(|active| active.1);
            if request_id.is_some() {
                state.snapshot.nfts.pagination_resource = ResourceState::idle();
                state.next_revision()?;
            }
            request_id
        };

        if let Some(request_id) = request_id {
            self.http_host.cancel_http(request_id).await;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct NftPage {
    items: Vec<NftItem>,
    raw_count: usize,
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct RawNftItemsResponse {
    nft_items: Vec<RawNftItem>,
    #[serde(default)]
    metadata: HashMap<String, RawAddressMetadata>,
}

#[derive(Debug, Deserialize)]
struct RawNftItem {
    address: String,
    #[serde(default)]
    auction_contract_address: Option<String>,
    code_hash: String,
    #[serde(default)]
    collection: Option<RawNftCollection>,
    #[serde(default)]
    collection_address: Option<String>,
    #[serde(default)]
    content: HashMap<String, Value>,
    data_hash: String,
    index: Value,
    init: bool,
    last_transaction_lt: Value,
    on_sale: bool,
    #[serde(default)]
    owner_address: Option<String>,
    #[serde(default)]
    real_owner: Option<String>,
    #[serde(default)]
    sale_contract_address: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawNftCollection {
    address: String,
    #[serde(default)]
    collection_content: HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct RawAddressMetadata {
    #[serde(default)]
    token_info: Vec<RawTokenInfo>,
}

#[derive(Debug, Deserialize)]
struct RawTokenInfo {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    is_nsfw: Option<bool>,
    #[serde(default)]
    is_scam: Option<bool>,
    #[serde(default)]
    extra: HashMap<String, Value>,
    #[serde(flatten)]
    fields: HashMap<String, Value>,
}

fn build_nft_page_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    offset: u64,
) -> Result<HttpRequest, WalletClientError> {
    let limit = NFT_PAGE_SIZE.to_string();
    let offset = offset.to_string();
    build_toncenter_v3_request(
        config,
        id,
        "nft/items",
        &[
            ("owner_address", config.address.as_str()),
            ("limit", limit.as_str()),
            ("offset", offset.as_str()),
            ("sort_by_last_transaction_lt", "true"),
        ],
    )
}

pub(super) fn build_nft_item_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    address: &TonAddressString,
) -> Result<HttpRequest, WalletClientError> {
    build_toncenter_v3_request(
        config,
        id,
        "nft/items",
        &[
            ("address", address.as_str()),
            ("limit", "2"),
            ("offset", "0"),
        ],
    )
}

pub(super) fn parse_single_nft_item(
    body: &[u8],
    expected_address: &TonAddressString,
    network: Network,
) -> Result<NftItem, DomainError> {
    let response: RawNftItemsResponse =
        serde_json::from_slice(body).map_err(|error| invalid_response(error.to_string()))?;
    if response.nft_items.len() != 1 {
        return Err(invalid_response(format!(
            "expected exactly one NFT item, provider returned {}",
            response.nft_items.len()
        )));
    }
    let raw = response
        .nft_items
        .into_iter()
        .next()
        .ok_or_else(|| invalid_response("provider returned no NFT item"))?;
    let item = parse_nft_item(raw, &response.metadata, network)?;
    if item.address != *expected_address {
        return Err(invalid_response("provider returned a different NFT item"));
    }
    Ok(item)
}

fn parse_nft_page(body: &[u8], page_size: u32, network: Network) -> Result<NftPage, DomainError> {
    let response: RawNftItemsResponse =
        serde_json::from_slice(body).map_err(|error| invalid_response(error.to_string()))?;
    let raw_count = response.nft_items.len();
    let items = response
        .nft_items
        .into_iter()
        .map(|item| parse_nft_item(item, &response.metadata, network))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(NftPage {
        items,
        raw_count,
        has_more: usize::try_from(page_size).is_ok_and(|page_size| raw_count >= page_size),
    })
}

fn parse_nft_item(
    raw: RawNftItem,
    metadata: &HashMap<String, RawAddressMetadata>,
    network: Network,
) -> Result<NftItem, DomainError> {
    let collection_address = raw
        .collection
        .as_ref()
        .map(|collection| collection.address.as_str())
        .or(raw.collection_address.as_deref());

    let item_info = token_info(metadata, &raw.address, "nft_items");
    let collection_info =
        collection_address.and_then(|address| token_info(metadata, address, "nft_collections"));
    let collection = resolve_collection_descriptor(
        collection_address,
        raw.collection.as_ref(),
        collection_info,
        network,
    )?;

    let mut content = enriched_metadata(&raw.content, item_info);
    if let Some(collection_name) = collection
        .as_ref()
        .and_then(|descriptor| descriptor.name.as_ref())
    {
        let _ = content
            .entry("collection_name".to_owned())
            .or_insert_with(|| collection_name.clone());
    }
    if !content.contains_key("name")
        && let Some(domain) = content.get("domain").cloned()
    {
        let _ = content.insert("name".to_owned(), domain);
    }

    Ok(NftItem {
        address: parse_address(&raw.address, "NFT address", network)?,
        collection_address: collection
            .as_ref()
            .map(|descriptor| descriptor.address.clone()),
        collection,
        owner_address: parse_optional_address(
            raw.owner_address.as_deref(),
            "NFT owner address",
            network,
        )?,
        real_owner: parse_optional_address(
            raw.real_owner.as_deref(),
            "NFT real owner address",
            network,
        )?,
        sale_contract_address: parse_optional_address(
            raw.sale_contract_address.as_deref(),
            "NFT sale contract address",
            network,
        )?,
        auction_contract_address: parse_optional_address(
            raw.auction_contract_address.as_deref(),
            "NFT auction contract address",
            network,
        )?,
        index: parse_decimal(&raw.index, "NFT index")?,
        last_transaction_lt: parse_decimal(
            &raw.last_transaction_lt,
            "NFT last transaction logical time",
        )?,
        initialized: raw.init,
        on_sale: raw.on_sale,
        code_hash: raw.code_hash,
        data_hash: raw.data_hash,
        content,
        is_nsfw: item_info.and_then(|info| info.is_nsfw),
        is_scam: item_info.and_then(|info| info.is_scam),
    })
}

fn resolve_collection_descriptor(
    address: Option<&str>,
    raw: Option<&RawNftCollection>,
    metadata: Option<&RawTokenInfo>,
    network: Network,
) -> Result<Option<NftCollectionDescriptor>, DomainError> {
    let Some(address) = address else {
        return Ok(None);
    };

    let empty = HashMap::new();
    let raw_content = raw.map_or(&empty, |collection| &collection.collection_content);
    let content = enriched_metadata(raw_content, metadata);

    Ok(Some(NftCollectionDescriptor {
        address: parse_address(address, "NFT collection address", network)?,
        name: resolved_metadata_field(&content, "name"),
        description: resolved_metadata_field(&content, "description"),
        image: resolved_metadata_field(&content, "image"),
        content,
    }))
}

fn token_info<'a>(
    metadata: &'a HashMap<String, RawAddressMetadata>,
    address: &str,
    kind: &str,
) -> Option<&'a RawTokenInfo> {
    metadata
        .get(address)?
        .token_info
        .iter()
        .find(|info| info.kind.as_deref() == Some(kind))
}

fn enriched_metadata(
    values: &HashMap<String, Value>,
    metadata: Option<&RawTokenInfo>,
) -> HashMap<String, String> {
    let mut content = string_values(values);
    if let Some(metadata) = metadata {
        content.extend(string_values(&metadata.extra));
        content.extend(string_values(&metadata.fields));
        for key in NFT_CONTENT_KEYS {
            if let Some(value) =
                string_field(&metadata.fields, key).or_else(|| string_field(&metadata.extra, key))
            {
                let _ = content.insert((*key).to_owned(), value.to_owned());
            }
        }
    }
    content
}

fn string_values(values: &HashMap<String, Value>) -> HashMap<String, String> {
    values
        .iter()
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_owned())))
        .collect()
}

fn resolved_metadata_field(values: &HashMap<String, String>, key: &str) -> Option<String> {
    values.get(key).filter(|value| !value.is_empty()).cloned()
}

fn string_field<'a>(values: &'a HashMap<String, Value>, key: &str) -> Option<&'a str> {
    values
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn parse_decimal(value: &Value, field: &str) -> Result<UnsignedDecimalString, DomainError> {
    let value = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => {
            return Err(invalid_response(format!("{field} is not a decimal value")));
        }
    };
    UnsignedDecimalString::try_from(value)
        .map_err(|_| invalid_response(format!("{field} is not an unsigned decimal")))
}

fn parse_address(
    value: &str,
    field: &str,
    network: Network,
) -> Result<TonAddressString, DomainError> {
    TonAddress::from_str(value)
        .map(|address| TonAddressString::from_address(&address, network))
        .map_err(|_| invalid_response(format!("{field} is missing or invalid")))
}

fn parse_optional_address(
    value: Option<&str>,
    field: &str,
    network: Network,
) -> Result<Option<TonAddressString>, DomainError> {
    value
        .map(|value| parse_address(value, field, network))
        .transpose()
}

fn apply_refreshed_nft_page(state: &mut State, page: NftPage) {
    state.snapshot.nfts.items = deduplicate_nfts(page.items);
    state.nft_offset = u64::try_from(page.raw_count).unwrap_or(u64::MAX);
    state.nfts_has_more = page.has_more;
    state.snapshot.nfts.has_more = page.has_more;
}

fn apply_nft_page(state: &mut State, page: NftPage) -> u64 {
    let previous_len = state.snapshot.nfts.items.len();
    let mut seen: HashSet<String> = state
        .snapshot
        .nfts
        .items
        .iter()
        .map(|item| item.address.as_str().to_owned())
        .collect();
    state.snapshot.nfts.items.extend(
        page.items
            .into_iter()
            .filter(|item| seen.insert(item.address.as_str().to_owned())),
    );
    state.nft_offset = state
        .nft_offset
        .saturating_add(u64::try_from(page.raw_count).unwrap_or(u64::MAX));
    state.nfts_has_more = page.has_more;
    state.snapshot.nfts.has_more = page.has_more;

    u64::try_from(state.snapshot.nfts.items.len().saturating_sub(previous_len)).unwrap_or(u64::MAX)
}

fn deduplicate_nfts(items: Vec<NftItem>) -> Vec<NftItem> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.address.as_str().to_owned()))
        .collect()
}

const NFT_CONTENT_KEYS: &[&str] = &[
    "uri",
    "name",
    "description",
    "_image_small",
    "_image_medium",
    "_image_big",
    "image",
    "preview",
    "image_url",
    "symbol",
    "collection",
    "collection_name",
];

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        ActivityList, Network, NftList, NonEmptyString, ProviderConfig, ResourcePhase, SendPhase,
        SendSnapshot, TonAddressString, WalletSnapshot,
    };

    const OWNER: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";
    const ITEM: &str = "0:2222222222222222222222222222222222222222222222222222222222222222";
    const ITEM_TWO: &str = "0:3333333333333333333333333333333333333333333333333333333333333333";
    const COLLECTION: &str = "0:4444444444444444444444444444444444444444444444444444444444444444";

    #[test]
    fn request_matches_actonscan_owner_pagination_contract() {
        let state = state();
        let request = build_nft_page_request(&state.config, HttpRequestId { value: 7 }, 120)
            .expect("the provider URL is valid");

        assert_eq!(
            request.url,
            format!(
                "https://testnet.toncenter.com/api/v3/nft/items?owner_address={}&limit=60&offset=120&sort_by_last_transaction_lt=true",
                OWNER.replace(':', "%3A")
            )
        );
    }

    #[test]
    fn transfer_preflight_queries_one_exact_item() {
        let state = state();
        let item = TonAddressString::try_from(ITEM).expect("item address");
        let request = build_nft_item_request(&state.config, HttpRequestId { value: 8 }, &item)
            .expect("the provider URL is valid");

        assert_eq!(
            request.url,
            format!(
                "https://testnet.toncenter.com/api/v3/nft/items?address={}&limit=2&offset=0",
                ITEM.replace(':', "%3A")
            )
        );
    }

    #[test]
    fn transfer_preflight_rejects_missing_duplicate_and_wrong_items() {
        let expected = TonAddressString::try_from(ITEM).expect("item address");
        for items in [
            json!([]),
            json!([raw_nft(ITEM), raw_nft(ITEM)]),
            json!([raw_nft(ITEM_TWO)]),
        ] {
            let body =
                serde_json::to_vec(&json!({"nft_items": items})).expect("fixture serializes");
            let error = parse_single_nft_item(&body, &expected, Network::Testnet)
                .expect_err("ambiguous or wrong item must fail");
            assert_eq!(error.code, ErrorCode::InvalidProviderResponse);
        }
    }

    #[test]
    fn parses_items_and_enriches_content_from_metadata() {
        let body = json!({
            "nft_items": [{
                "address": ITEM,
                "code_hash": "code",
                "collection": {
                    "address": COLLECTION,
                    "collection_content": {
                        "description": "On-chain collection description",
                        "image": "ipfs://collection/image.png",
                        "ignored": true
                    }
                },
                "content": { "domain": "alice.ton", "ignored": true },
                "data_hash": "data",
                "index": "340282366920938463463374607431768211456",
                "init": true,
                "last_transaction_lt": 42,
                "on_sale": false,
                "owner_address": OWNER
            }],
            "metadata": {
                ITEM: {
                    "token_info": [{
                        "type": "nft_items",
                        "name": "Indexed name",
                        "description": "Indexed description",
                        "is_nsfw": false,
                        "is_scam": true,
                        "extra": { "image_url": "https://example.com/nft.png" }
                    }]
                },
                COLLECTION: {
                    "token_info": [{
                        "type": "nft_collections",
                        "name": "Collection",
                        "description": "Indexed collection description",
                        "custom": "provider value",
                        "extra": { "uri": "https://example.com/collection.json" }
                    }]
                }
            }
        });

        let page = parse_nft_page(
            serde_json::to_string(&body)
                .expect("fixture serializes")
                .as_bytes(),
            NFT_PAGE_SIZE,
            Network::Testnet,
        )
        .expect("valid NFT response");
        let item = page.items.first().expect("one NFT item");

        assert_eq!(
            item.address.as_address(),
            &TonAddress::from_str(ITEM).expect("valid item address")
        );
        assert_eq!(
            item.owner_address.as_ref().expect("owner").as_address(),
            &TonAddress::from_str(OWNER).expect("valid owner address")
        );
        assert_eq!(
            item.index.to_string(),
            "340282366920938463463374607431768211456"
        );
        assert_eq!(item.last_transaction_lt.to_string(), "42");
        assert_eq!(
            item.content.get("name").map(String::as_str),
            Some("Indexed name")
        );
        assert_eq!(
            item.content.get("collection_name").map(String::as_str),
            Some("Collection")
        );
        let collection = item.collection.as_ref().expect("collection descriptor");
        assert_eq!(item.collection_address.as_ref(), Some(&collection.address));
        assert_eq!(collection.name.as_deref(), Some("Collection"));
        assert_eq!(
            collection.description.as_deref(),
            Some("Indexed collection description")
        );
        assert_eq!(
            collection.image.as_deref(),
            Some("ipfs://collection/image.png")
        );
        assert_eq!(
            collection.content.get("uri").map(String::as_str),
            Some("https://example.com/collection.json")
        );
        assert_eq!(
            collection.content.get("custom").map(String::as_str),
            Some("provider value")
        );
        assert!(!collection.content.contains_key("ignored"));
        assert_eq!(
            item.content.get("image_url").map(String::as_str),
            Some("https://example.com/nft.png")
        );
        assert!(!item.content.contains_key("ignored"));
        assert_eq!(item.is_nsfw, Some(false));
        assert_eq!(item.is_scam, Some(true));
        assert!(!page.has_more);
    }

    #[test]
    fn resolves_an_address_only_collection_descriptor() {
        let mut item = raw_nft(ITEM);
        item["collection_address"] = json!(COLLECTION);
        let body = json!({ "nft_items": [item] });

        let page = parse_nft_page(
            serde_json::to_string(&body)
                .expect("fixture serializes")
                .as_bytes(),
            NFT_PAGE_SIZE,
            Network::Testnet,
        )
        .expect("address-only collection must parse");
        let item = page.items.first().expect("one NFT item");
        let collection = item.collection.as_ref().expect("collection descriptor");

        assert_eq!(item.collection_address.as_ref(), Some(&collection.address));
        assert!(collection.name.is_none());
        assert!(collection.description.is_none());
        assert!(collection.image.is_none());
        assert!(collection.content.is_empty());
    }

    fn raw_nft(address: &str) -> Value {
        json!({
            "address": address,
            "code_hash": "code",
            "content": {},
            "data_hash": "data",
            "index": "0",
            "init": true,
            "last_transaction_lt": "1",
            "on_sale": false,
            "owner_address": OWNER,
        })
    }

    #[test]
    fn rejects_malformed_core_fields() {
        for (field, value) in [
            ("address", json!("not-an-address")),
            ("index", json!(-1)),
            ("last_transaction_lt", json!(false)),
        ] {
            let mut item = json!({
                "address": ITEM,
                "code_hash": "code",
                "content": {},
                "data_hash": "data",
                "index": "1",
                "init": true,
                "last_transaction_lt": "2",
                "on_sale": false
            });
            item[field] = value;
            let body =
                serde_json::to_vec(&json!({ "nft_items": [item] })).expect("fixture serializes");

            let error = parse_nft_page(&body, NFT_PAGE_SIZE, Network::Testnet)
                .expect_err("malformed NFT data must fail");
            assert_eq!(error.code, ErrorCode::InvalidProviderResponse);
        }
    }

    #[test]
    fn pagination_advances_by_raw_rows_and_deduplicates_by_address() {
        let mut state = state();
        apply_refreshed_nft_page(
            &mut state,
            NftPage {
                items: vec![nft(ITEM, 2)],
                raw_count: 60,
                has_more: true,
            },
        );

        let added = apply_nft_page(
            &mut state,
            NftPage {
                items: vec![nft(ITEM, 2), nft(ITEM_TWO, 1)],
                raw_count: 2,
                has_more: false,
            },
        );

        assert_eq!(added, 1);
        assert_eq!(state.nft_offset, 62);
        assert_eq!(state.snapshot.nfts.items.len(), 2);
        assert!(!state.nfts_has_more);
        assert!(!state.snapshot.nfts.has_more);
    }

    fn state() -> State {
        let config = WalletClientConfig {
            record_id: NonEmptyString::try_from("nft-tests").expect("valid record identifier"),
            address: TonAddressString::try_from(OWNER).expect("valid TON address"),
            public_key: vec![0; 32],
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig::standard(Network::Testnet),
        };
        State {
            snapshot: WalletSnapshot {
                revision: 0,
                record_id: config.record_id.clone(),
                address: config.address.clone(),
                network: config.network,
                account: None,
                account_resource: ResourceState::idle(),
                activity: ActivityList {
                    items: Vec::new(),
                    resource: ResourceState::idle(),
                    pagination_resource: ResourceState::idle(),
                    has_more: false,
                },
                activity_cursor: None,
                nfts: NftList {
                    items: Vec::new(),
                    resource: ResourceState::idle(),
                    pagination_resource: ResourceState::idle(),
                    has_more: false,
                },
                send: SendSnapshot {
                    operation_id: None,
                    phase: SendPhase::Idle,
                    error_message: None,
                    resolution: None,
                },
            },
            config,
            activity: Vec::new(),
            activity_cursor: None,
            activity_has_more: false,
            nft_offset: 0,
            nfts_has_more: false,
            next_id: 1,
            refresh_generation: 0,
            pagination_generation: 0,
            nft_refresh_generation: 0,
            nft_pagination_generation: 0,
            preview_generation: 0,
            send_generation: 0,
            resolution_generation: 0,
            active_refresh: None,
            active_pagination: None,
            active_nft_refresh: None,
            active_nft_pagination: None,
            active_preview: None,
            active_send: None,
            active_resolution: None,
            send_commit_started: false,
            send_workflow: None,
            waiters: Vec::new(),
            shutdown: false,
            closing: false,
        }
    }

    fn nft(address: &str, last_transaction_lt: u64) -> NftItem {
        NftItem {
            address: TonAddressString::try_from(address).expect("valid NFT address"),
            collection_address: None,
            collection: None,
            owner_address: Some(TonAddressString::try_from(OWNER).expect("valid owner address")),
            real_owner: None,
            sale_contract_address: None,
            auction_contract_address: None,
            index: UnsignedDecimalString::from(last_transaction_lt),
            last_transaction_lt: UnsignedDecimalString::from(last_transaction_lt),
            initialized: true,
            on_sale: false,
            code_hash: "code".to_owned(),
            data_hash: "data".to_owned(),
            content: HashMap::new(),
            is_nsfw: None,
            is_scam: None,
        }
    }

    #[test]
    fn failed_nft_resource_keeps_last_successful_items() {
        let mut state = state();
        state.snapshot.nfts.items = vec![nft(ITEM, 1)];
        state.snapshot.nfts.resource = ResourceState::failed(invalid_response("bad response"));

        assert_eq!(state.snapshot.nfts.items.len(), 1);
        assert_eq!(state.snapshot.nfts.resource.phase, ResourcePhase::Failed);
    }
}
