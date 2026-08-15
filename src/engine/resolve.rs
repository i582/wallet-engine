//! Public single-shot recovery of the durable outgoing-send journal.

use super::WalletClient;
use super::http::{build_toncenter_v2_request, evaluate_response};
use super::provider::parse_account;
use super::resolution::ResolutionRequests;
use super::state::ensure_running;
use crate::wallet::send::{pending_send_record, send_snapshot_from_journal};
use crate::{JournalKey, SendSnapshot, WalletClientError};
use futures_timer::Delay;
use std::time::{Duration, Instant};

#[uniffi::export]
impl WalletClient {
    /// Performs startup recovery and actively follows an unresolved send for the
    /// configured polling budget.
    ///
    /// The delay budget affects only how long this call waits. Every terminal
    /// conclusion still comes from provider time or chain evidence.
    pub async fn start(&self) -> Result<SendSnapshot, WalletClientError> {
        self.resolve_pending_active().await
    }

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
            let source = config.parsed_address()?;
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
                state.allocate_request_id()?,
                state.allocate_request_id()?,
            ];
            state.active_resolution = Some((generation, Vec::new()));
            (
                generation,
                config.clone(),
                source,
                JournalKey {
                    record_id: config.record_id,
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
            let snapshot = send_snapshot_from_journal(
                &journal,
                &config.record_id,
                &source,
                config.resolution_poll_interval_ms,
            )
            .map_err(|error| self.fail_standalone_resolution(generation, error.to_string()))?;
            return self.complete_standalone_resolution(generation, Some(snapshot));
        };

        // Expiration uses provider synchronization time, never device time. A
        // wrong local clock must not unlock the wallet for a replacement while
        // the original signed message can still be accepted on-chain.
        let account_response = self
            .execute_tracked_standalone_resolution_request(generation, &account_request)
            .await?;
        let account = evaluate_response(&account_request, account_response, parse_account)
            .map_err(|error| {
                self.fail_standalone_resolution(generation, error.developer_message)
            })?;
        let provider_time = account.sync_utime.ok_or_else(|| {
            self.fail_standalone_resolution(
                generation,
                "fresh account state did not include provider synchronization time",
            )
        })?;
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
        let snapshot = pending.snapshot(&resolved.resolution, config.resolution_poll_interval_ms);
        self.complete_standalone_resolution(generation, Some(snapshot))
    }
}

impl WalletClient {
    /// Repeats single-shot resolution until evidence becomes terminal or the
    /// configured active budget is exhausted.
    pub(super) async fn resolve_pending_active(&self) -> Result<SendSnapshot, WalletClientError> {
        let (interval_ms, budget_ms) = {
            let state = self.lock()?;
            ensure_running(&state)?;
            (
                state.config.resolution_poll_interval_ms,
                state.config.resolution_active_budget_ms,
            )
        };
        let started_at = Instant::now();
        let mut latest = None;
        let mut last_error = None;

        loop {
            match self.resolve_pending().await {
                Ok(snapshot) => {
                    let terminal = snapshot
                        .resolution
                        .as_ref()
                        .and_then(|resolution| resolution.pending_reason)
                        .is_none();
                    latest = Some(snapshot);
                    if terminal {
                        return Ok(latest.expect("the successful snapshot was just stored"));
                    }
                }
                Err(
                    error @ (WalletClientError::Shutdown | WalletClientError::StateUnavailable),
                ) => {
                    return Err(error);
                }
                Err(error) => {
                    // A temporary provider failure is not chain evidence. Keep
                    // the last honest pending snapshot and retry within the
                    // active budget instead of turning a flaky poll into a
                    // terminal startup failure.
                    last_error = Some(error);
                }
            }

            let elapsed_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
            if elapsed_ms >= budget_ms {
                break;
            }
            let remaining_ms = budget_ms - elapsed_ms;
            Delay::new(Duration::from_millis(interval_ms.min(remaining_ms))).await;
            if u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX) >= budget_ms {
                break;
            }
        }

        latest.map_or_else(
            || Err(last_error.unwrap_or(WalletClientError::StateUnavailable)),
            Ok,
        )
    }
}
