//! Callback interfaces implemented by the embedding application.

use async_trait::async_trait;

use crate::{
    HttpHostError, HttpRequest, HttpRequestId, HttpResponse, JournalCompareExchange,
    JournalCompareExchangeResult, JournalHostError, JournalKey, JournalRecord,
    ProtectedSecretHostError, ProtectedSecretRead, ProtectedSecretRef, ProtectedSecretStore,
};

/// Executes bounded HTTP work for the engine.
///
/// The host must enforce each response limit while it reads the response.
/// It must reject redirects and return the observed URL in `final_url`.
#[uniffi::export(foreign)]
#[async_trait]
pub trait WalletHttpHost: Send + Sync {
    /// Executes one complete HTTP request and returns a bounded response.
    ///
    /// The host can add its Toncenter credential according to the actual URL
    /// and its local security policy.
    async fn execute_http(&self, request: HttpRequest) -> Result<HttpResponse, HttpHostError>;

    /// Requests cancellation of the request with `request_id`.
    ///
    /// This callback must be idempotent. It can run before `execute_http`
    /// registers the request, so the host must remember an early cancellation.
    async fn cancel_http(&self, request_id: HttpRequestId);
}

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
