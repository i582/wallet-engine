//! Immutable wallet snapshots and operation results.

use super::{
    AccountSnapshot, ActivityCursor, ActivityItem, Network, ResourceState, SendPhase, SendSnapshot,
    WalletClientConfig,
};

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
    pub record_id: String,
    /// The wallet address.
    pub address: String,
    /// The wallet network.
    pub network: Network,
    /// The latest successful account value, if available.
    pub account: Option<AccountSnapshot>,
    /// The load state for `account`.
    pub account_resource: ResourceState,
    /// Activity in descending logical-time order with duplicate items removed.
    pub activity: Vec<ActivityItem>,
    /// The load state for the first activity page.
    pub activity_resource: ResourceState,
    /// The load state for an additional activity page.
    pub activity_pagination_resource: ResourceState,
    /// The cursor for the next older activity page.
    pub activity_cursor: Option<ActivityCursor>,
    /// Whether the provider can have another activity page.
    pub activity_has_more: bool,
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
            activity: Vec::new(),
            activity_resource: ResourceState::idle(),
            activity_pagination_resource: ResourceState::idle(),
            activity_cursor: None,
            activity_has_more: false,
            send: SendSnapshot {
                operation_id: None,
                phase: SendPhase::Idle,
                error_message: None,
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

/// The result of a refresh or pagination call and its resulting snapshot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletUpdate {
    /// The terminal operation outcome.
    pub outcome: WalletOperationOutcome,
    /// The number of new unique activity items added by pagination.
    pub activity_items_added: u64,
    /// The immutable snapshot after the operation.
    pub snapshot: WalletSnapshot,
}
