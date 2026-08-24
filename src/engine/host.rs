//! Platform callback interfaces implemented by the embedding application.

use async_trait::async_trait;

use crate::{
    JournalCompareExchange, JournalCompareExchangeResult, JournalHostError, JournalKey,
    JournalRecord, ProtectedSecretHostError, ProtectedSecretRead, ProtectedSecretRef,
    ProtectedSecretStore,
};

/// Supplies protected storage and durable journal storage.
///
/// Callback implementations must not call the same client operation
/// recursively. The engine does not hold its wallet-state lock during calls.
#[uniffi::export(foreign)]
#[async_trait]
pub trait WalletPlatformHost: Send + Sync {
    /// Reads protected secret bytes after the required user authorization.
    ///
    /// The host must not log the bytes. Return a classified host error when
    /// authorization fails or the user cancels the prompt.
    async fn read_protected_secret(
        &self,
        request: ProtectedSecretRead,
    ) -> Result<Vec<u8>, ProtectedSecretHostError>;

    /// Stores secret bytes under the supplied reference.
    ///
    /// The host must apply the `require_user_presence` policy to later reads.
    async fn store_protected_secret(
        &self,
        request: ProtectedSecretStore,
    ) -> Result<(), ProtectedSecretHostError>;

    /// Deletes the protected secret for `secret_ref`.
    async fn delete_protected_secret(
        &self,
        secret_ref: ProtectedSecretRef,
    ) -> Result<(), ProtectedSecretHostError>;

    /// Loads the current opaque journal record for `key`.
    async fn load_journal(
        &self,
        key: JournalKey,
    ) -> Result<Option<JournalRecord>, JournalHostError>;

    /// Atomically replaces a journal record when its version matches.
    ///
    /// The host must compare and replace in one durable transaction. It must
    /// return the current record when the expected version does not match.
    async fn compare_exchange_journal(
        &self,
        mutation: JournalCompareExchange,
    ) -> Result<JournalCompareExchangeResult, JournalHostError>;
}
