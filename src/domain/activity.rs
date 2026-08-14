//! Account state and paginated wallet activity.

/// The lifecycle state of a TON account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum AccountStatus {
    /// No account exists at the address.
    Nonexistent,
    /// The account exists but its contract state is not initialized.
    Uninitialized,
    /// The account contract is active.
    Active,
    /// The account contract is frozen.
    Frozen,
    /// The provider returned an unrecognized account state.
    Unknown,
}

/// The latest parsed balance and status for a wallet account.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    /// The exact unsigned balance in nanograms as a canonical decimal string.
    /// Hosts can parse it with their arbitrary-precision integer type.
    pub balance_nanograms: String,
    /// The account lifecycle state.
    pub status: AccountStatus,
    /// The provider synchronization time as a Unix timestamp, if available.
    pub sync_utime: Option<u64>,
}

/// The value flow direction for one activity item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDirection {
    /// Value left the wallet.
    Sent,
    /// Value entered the wallet.
    Received,
}

/// One nonzero incoming or outgoing value transfer in wallet activity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    /// A stable item key derived from the transaction, direction, and message index.
    pub id: String,
    /// The transaction hash.
    ///
    /// A valid 256-bit provider hash is normalized to standard padded Base64.
    /// If the provider returns another representation, this field preserves it.
    pub transaction_hash: String,
    /// The transaction logical time as a canonical unsigned decimal string.
    pub logical_time: String,
    /// The transaction Unix timestamp.
    pub timestamp: u64,
    /// The value flow direction relative to this wallet.
    pub direction: ActivityDirection,
    /// The exact transferred value in nanograms as a canonical decimal string.
    /// Hosts can parse it with their arbitrary-precision integer type.
    pub amount_nanograms: String,
    /// The source or destination address, if the provider supplies it.
    pub counterparty: Option<String>,
}

/// The provider cursor for the next older activity page.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ActivityCursor {
    /// The oldest loaded transaction logical time as a canonical decimal string.
    pub logical_time: String,
    /// The oldest loaded transaction hash.
    ///
    /// This uses the same encoding rules as [`ActivityItem::transaction_hash`].
    pub hash: String,
}
