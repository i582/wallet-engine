//! Public single-shot recovery of the durable outgoing-send journal.

use super::WalletClient;
use super::http::build_toncenter_v2_request;
use super::provider::parse_account;
use super::resolution::ResolutionRequests;
use super::state::ensure_running;
use crate::wallet::send::{pending_send_record, send_snapshot_from_journal};
use crate::{JournalKey, SendSnapshot, WalletClientError};

#[uniffi::export]
impl WalletClient {
    /// Resolves the durable outgoing message from chain evidence without signing.
    ///
    /// The operation is idempotent. It never reads protected secret storage and
    /// commits terminal evidence with compare-and-swap journal transitions.
    pub async fn resolve_pending(&self) -> Result<SendSnapshot, WalletClientError> {
        // Reserve all request IDs before leaving the mutex. This keeps shutdown
        // and late HTTP callbacks generation-safe without holding the state lock
        // across host calls.
        let (generation, config, source, journal_key, account_request, resolution_ids) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            if state.active_send.is_some() || state.active_resolution.is_some() {
                return Err(WalletClientError::SendAlreadyInProgress);
            }
            state.resolution_generation = state
                .resolution_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.resolution_generation;
            let config = state.config.clone();
            let source = config.address.as_address().clone();
            let account_request = build_toncenter_v2_request(
                &config,
                state.allocate_request_id()?,
                "getAddressInformation",
                &[("address", config.address.as_str())],
            )?;
            let resolution_ids = [
                state.allocate_request_id()?,
                state.allocate_request_id()?,
                state.allocate_request_id()?,
                state.allocate_request_id()?,
            ];
            state.active_resolution = Some((generation, Vec::new()));
            (
                generation,
                config.clone(),
                source,
                JournalKey {
                    record_id: config.record_id.to_string(),
                    slot: crate::wallet::send::SEND_SLOT.to_owned(),
                },
                account_request,
                resolution_ids,
            )
        };

        let journal = self
            .platform_host
            .load_journal(journal_key)
            .await
            .map_err(|error| self.fail_standalone_resolution(generation, error.to_string()))?;
        let Some(journal) = journal else {
            return self.complete_standalone_resolution(generation, None);
        };

        let pending = pending_send_record(&journal, &config.record_id, &source)
            .map_err(|error| self.fail_standalone_resolution(generation, error.to_string()))?;
        let Some(pending) = pending else {
            let snapshot = send_snapshot_from_journal(&journal, &config.record_id, &source)
                .map_err(|error| self.fail_standalone_resolution(generation, error.to_string()))?;
            return self.complete_standalone_resolution(generation, Some(snapshot));
        };

        // Expiration uses provider synchronization time, never device time. A
        // wrong local clock must not unlock the wallet for a replacement while
        // the original signed message can still be accepted on-chain.
        let account = self
            .execute_tracked_standalone_resolution_request(generation, &account_request)
            .await?
            .and_then(|body| parse_account(&body))
            .map_err(|error| {
                self.fail_standalone_resolution(generation, error.developer_message)
            })?;

        let provider_time = account.sync_utime;
        let requests = ResolutionRequests::new(&config, &pending, resolution_ids)
            .map_err(|error| self.fail_standalone_resolution(generation, error.to_string()))?;
        let resolved = self
            .resolve_pending_standalone(
                generation,
                &config,
                pending.clone(),
                provider_time,
                &requests,
            )
            .await?;
        let snapshot = pending.snapshot(&resolved.resolution);
        self.complete_standalone_resolution(generation, Some(snapshot))
    }
}
