//! Account state and paginated wallet activity.

use crate::{Base64Hash, Boc, TonAddressString, UnsignedDecimalString};

/// The lifecycle state of a TON account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
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
    /// The exact account balance, in nanograms.
    pub balance_nanograms: UnsignedDecimalString,
    /// The account lifecycle state.
    pub status: AccountStatus,
    /// The provider synchronization time as a Unix timestamp.
    pub sync_utime: u64,
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

/// The on-chain result of the transaction or internal message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ActivityStatus {
    /// The transaction completed and this message was not a bounce.
    Success,
    /// The transaction was aborted while processing the message.
    Failed,
    /// The internal message has the on-chain `bounced` flag.
    Bounced,
}

/// One nonzero incoming or outgoing value transfer in wallet activity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    /// A stable item key derived from the transaction, direction, and message index.
    pub id: String,
    /// The transaction hash in standard padded Base64.
    pub transaction_hash: Base64Hash,
    /// The transaction logical time.
    pub logical_time: UnsignedDecimalString,
    /// The transaction Unix timestamp.
    pub timestamp: u64,
    /// The value flow direction relative to this wallet.
    pub direction: ActivityDirection,
    /// The exact transferred value, in nanograms.
    pub amount_nanograms: UnsignedDecimalString,
    /// The total fee charged by this transaction, in nanograms.
    ///
    /// A transaction with multiple visible messages repeats this value on each
    /// activity item; callers must not sum it per row.
    pub transaction_fee_nanograms: UnsignedDecimalString,
    /// The on-chain execution status for this message and transaction.
    pub status: ActivityStatus,
    /// A decoded zero-opcode plaintext comment, including an empty comment.
    pub comment: Option<String>,
    /// An opaque encrypted-comment body that can be passed to
    /// [`crate::WalletClient::decrypt_comment`].
    #[serde(default)]
    pub encrypted_comment: Option<Boc>,
    /// The source or destination address, if the provider supplies it.
    pub counterparty: Option<TonAddressString>,
}

/// The provider cursor for the next older activity page.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ActivityCursor {
    /// The oldest loaded transaction logical time.
    pub logical_time: UnsignedDecimalString,
    /// The oldest loaded transaction hash in standard padded Base64.
    pub hash: Base64Hash,
}
