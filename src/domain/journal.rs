//! Durable compare-and-swap journal records and host errors.

/// Selects one durable journal slot for one wallet record.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct JournalKey {
    /// The stable application record identifier.
    pub record_id: String,
    /// The engine-defined slot name.
    pub slot: String,
}

/// One opaque versioned journal value owned by the engine.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct JournalRecord {
    /// The positive compare-and-swap version.
    pub version: u64,
    /// The opaque engine payload. The host must preserve these bytes exactly.
    pub payload: Vec<u8>,
}

/// An atomic compare-and-swap request for a journal slot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct JournalCompareExchange {
    /// The journal slot to change.
    pub key: JournalKey,
    /// The required current version, or `None` when the slot must be absent.
    pub expected_version: Option<u64>,
    /// The complete replacement value.
    pub replacement: JournalRecord,
}

/// The result of an atomic journal compare-and-swap operation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
#[serde(rename_all = "camelCase")]
pub struct JournalCompareExchangeResult {
    /// Whether the host stored the replacement.
    pub applied: bool,
    /// The current record after the operation, if one exists.
    pub current: Option<JournalRecord>,
}

/// Classifies a durable journal failure reported by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
#[serde(rename_all = "camelCase")]
pub enum JournalHostErrorKind {
    /// Durable storage is temporarily unavailable.
    Unavailable,
    /// The stored record cannot be read without data loss.
    CorruptData,
    /// The host cancelled the storage operation.
    Cancelled,
    /// The failure does not match another kind.
    Other,
}

/// A durable journal failure returned by [`crate::WalletPlatformHost`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
#[serde(rename_all = "camelCase")]
pub enum JournalHostError {
    /// Reports a classified journal failure with a safe diagnostic message.
    #[error("journal host failure ({kind:?}): {diagnostic}")]
    Failed {
        /// The stable failure classification.
        kind: JournalHostErrorKind,
        /// A developer-facing message that contains no secret or payload bytes.
        diagnostic: String,
    },
}
