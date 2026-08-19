//! Immutable wallet snapshots and operation results.

use super::{
    AccountSnapshot, ActivityCursor, ActivityItem, Network, NftItem, ResourceState, SendPhase,
    SendSnapshot, WalletClientConfig,
};
use crate::{NonEmptyString, TonAddressString};

/// Paginated wallet activity and its independent load states.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ActivityList {
    /// Activity in descending logical-time order with duplicate items removed.
    pub items: Vec<ActivityItem>,
    /// The load state for the first activity page.
    pub resource: ResourceState,
    /// The load state for an additional activity page.
    pub pagination_resource: ResourceState,
    /// Whether the provider can have another activity page.
    pub has_more: bool,
}

/// Paginated NFT items owned by the wallet and their independent load states.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NftList {
    /// NFT items in descending last-transaction order.
    pub items: Vec<NftItem>,
    /// The load state for the first NFT page.
    pub resource: ResourceState,
    /// The load state for an additional NFT page.
    pub pagination_resource: ResourceState,
    /// Whether the provider can have another NFT page.
    pub has_more: bool,
}

/// An immutable view of all observable state in one wallet client.
///
/// Each published change increments `revision`. A failed refresh preserves
/// the last successful resource value and changes its resource state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletSnapshot {
    /// A client-local counter that increases for every published state change.
    pub revision: u64,
    /// The stable application record identifier.
    pub record_id: NonEmptyString,
    /// The wallet address.
    pub address: TonAddressString,
    /// The wallet network.
    pub network: Network,
    /// The latest successful account value, if available.
    pub account: Option<AccountSnapshot>,
    /// The load state for `account`.
    pub account_resource: ResourceState,
    /// Paginated wallet activity.
    pub activity: ActivityList,
    /// The cursor for the next older activity page.
    pub activity_cursor: Option<ActivityCursor>,
    /// Paginated NFT items owned by the wallet.
    pub nfts: NftList,
    /// The current transfer workflow state.
    pub send: SendSnapshot,
}

impl WalletSnapshot {
    pub(crate) fn empty(config: &WalletClientConfig) -> Self {
        Self {
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
        }
    }
}

/// The terminal result of a wallet client operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum WalletOperationOutcome {
    /// All requested resources completed successfully.
    Completed,
    /// Some refresh resources succeeded and some failed.
    PartiallyCompleted,
    /// The requested operation failed.
    Failed,
    /// The host or caller cancelled the operation.
    Cancelled,
    /// A newer operation replaced this operation before publication completed.
    Superseded,
    /// Preconditions did not permit work, so the client made no request.
    Skipped,
}

/// The result of a refresh or pagination operation and its resulting snapshot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletUpdate {
    /// The terminal operation outcome.
    pub outcome: WalletOperationOutcome,
    /// The number of new unique activity items added by pagination.
    pub activity_items_added: u64,
    /// The number of new unique NFT items added by pagination.
    pub nft_items_added: u64,
    /// The immutable snapshot after the operation.
    pub snapshot: WalletSnapshot,
}
