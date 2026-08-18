//! Read-only chain resolution and durable CAS transitions for outgoing sends.

use crate::domain::bounded_diagnostic;
use crate::wallet::send::{
    PendingSendRecord, SendResolution, SendWorkflowError, SignedMessageKind, pending_send_record,
    terminal_send_resolution,
};
use crate::{
    DomainError, HttpRequest, JournalRecord, PendingReason, WalletClientConfig, WalletClientError,
};

use super::WalletClient;
use super::http::process_response;
use super::resolution_http::{
    build_executed_by_message_request, build_pending_transactions_request,
    build_wallet_state_request, parse_executed_message, parse_pending_message, parse_wallet_seqno,
};
use super::state::OperationFamily;

pub(super) struct ResolutionRequests {
    executed: HttpRequest,
    pending: HttpRequest,
    wallet_state: HttpRequest,
    executed_recheck: HttpRequest,
}

pub(super) struct ResolvedPending {
    pub(super) resolution: SendResolution,
    pub(super) journal: JournalRecord,
}

#[derive(Clone, Copy)]
enum ResolutionOwner {
    Send(u64),
    Standalone(u64),
}

impl ResolutionRequests {
    /// Materializes every possible request before resolution starts.
    ///
    /// Preallocation gives each request a stable ID so cancellation and stale
    /// callback rejection work identically in inline and standalone modes.
    pub(super) fn new(
        config: &WalletClientConfig,
        pending: &PendingSendRecord,
        ids: [crate::HttpRequestId; 4],
    ) -> Result<Self, WalletClientError> {
        Ok(Self {
            executed: build_executed_by_message_request(config, ids[0], &pending.message_hash)?,
            pending: build_pending_transactions_request(config, ids[1])?,
            wallet_state: build_wallet_state_request(config, ids[2])?,
            executed_recheck: build_executed_by_message_request(
                config,
                ids[3],
                &pending.message_hash,
            )?,
        })
    }
}

impl WalletClient {
    /// Executes one standalone resolver request under its resolution generation.
    /// Shutdown can therefore cancel it without treating it as a send request.
    pub(super) async fn execute_tracked_standalone_resolution_request(
        &self,
        generation: u64,
        request: &HttpRequest,
    ) -> Result<Result<Vec<u8>, DomainError>, WalletClientError> {
        self.execute_resolution_request(ResolutionOwner::Standalone(generation), request)
            .await
    }

    /// Releases the standalone single-flight slot and publishes the result only
    /// if this generation is still current.
    pub(super) fn complete_standalone_resolution(
        &self,
        generation: u64,
        snapshot: Option<crate::SendSnapshot>,
    ) -> Result<crate::SendSnapshot, WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Resolution, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.active_resolution = None;
        if let Some(snapshot) = snapshot
            && state.snapshot.send != snapshot
        {
            state.snapshot.send = snapshot;
            state.next_revision()?;
        }
        Ok(state.snapshot.send.clone())
    }

    /// Converts a standalone failure into the public bounded error and releases
    /// its active-operation slot.
    pub(super) fn fail_standalone_resolution(
        &self,
        generation: u64,
        message: impl Into<String>,
    ) -> WalletClientError {
        self.resolution_failed_error(ResolutionOwner::Standalone(generation), message)
    }

    /// Resolves an older journal entry inside `send()`, reusing send generation
    /// tracking so no new signature is produced until the result is terminal.
    pub(super) async fn resolve_pending_for_send(
        &self,
        generation: u64,
        config: &WalletClientConfig,
        pending: PendingSendRecord,
        provider_time: u64,
        requests: &ResolutionRequests,
    ) -> Result<ResolvedPending, WalletClientError> {
        self.resolve_pending_with_owner(
            ResolutionOwner::Send(generation),
            config,
            pending,
            provider_time,
            requests,
        )
        .await
    }

    /// Runs the same evidence algorithm for explicit or startup recovery without
    /// reading protected secret storage.
    pub(super) async fn resolve_pending_standalone(
        &self,
        generation: u64,
        config: &WalletClientConfig,
        pending: PendingSendRecord,
        provider_time: u64,
        requests: &ResolutionRequests,
    ) -> Result<ResolvedPending, WalletClientError> {
        self.resolve_pending_with_owner(
            ResolutionOwner::Standalone(generation),
            config,
            pending,
            provider_time,
            requests,
        )
        .await
    }

    /// Classifies one persisted message from ordered provider evidence and
    /// durably records terminal conclusions.
    async fn resolve_pending_with_owner(
        &self,
        owner: ResolutionOwner,
        config: &WalletClientConfig,
        pending: PendingSendRecord,
        provider_time: u64,
        requests: &ResolutionRequests,
    ) -> Result<ResolvedPending, WalletClientError> {
        if pending.kind == SignedMessageKind::Internal {
            let indexed_seqno = self
                .execute_resolution_request(owner, &requests.wallet_state)
                .await?
                .and_then(|body| parse_wallet_seqno(&body))
                .map_err(|error| self.resolution_failed_error(owner, error.developer_message))?;

            if indexed_seqno > pending.seqno {
                return self
                    .persist_send_resolution(owner, pending, SendResolution::SequenceNumberConsumed)
                    .await;
            }

            if resolution_window_expired(
                provider_time,
                pending.valid_until,
                config.resolution_margin_seconds,
            ) {
                return self
                    .persist_send_resolution(owner, pending, SendResolution::Expired)
                    .await;
            }

            return Ok(ResolvedPending {
                resolution: SendResolution::StillPending(PendingReason::AwaitingWindow),
                journal: pending.current_journal(),
            });
        }

        // Evidence is deliberately checked from strongest to weakest. In
        // particular, our own successful message also increments wallet seqno,
        // so looking at seqno before the message hash would mislabel a confirmed
        // transfer as Replaced.
        let executed = self
            .execute_resolution_request(owner, &requests.executed)
            .await?
            .and_then(|body| parse_executed_message(&body))
            .map_err(|error| self.resolution_failed_error(owner, error.developer_message))?;

        if let Some(executed) = executed {
            return self
                .persist_send_resolution(
                    owner,
                    pending,
                    SendResolution::Confirmed {
                        transaction_hash: executed.transaction_hash,
                        transaction_lt: executed.transaction_lt,
                    },
                )
                .await;
        }

        // pendingTransactions is optional across Toncenter-compatible providers.
        // 404/405 means the capability is absent and is safe to skip. Any other
        // protocol/transport failure suppresses Expired: in that case we failed
        // to prove that the message is absent from the provider's pending set.
        let pending_response = self
            .execute_resolution_request(owner, &requests.pending)
            .await?;

        let (is_pending, pending_observed) = match pending_response
            .and_then(|body| parse_pending_message(&body, &pending.message_hash))
        {
            Ok(is_pending) => (is_pending, true),
            Err(error) if matches!(error.provider_status, Some(404 | 405)) => (false, true),
            Err(_) => (false, false),
        };

        if is_pending {
            return Ok(ResolvedPending {
                resolution: SendResolution::StillPending(PendingReason::InMempool),
                journal: pending.current_journal(),
            });
        }

        let indexed_seqno = self
            .execute_resolution_request(owner, &requests.wallet_state)
            .await?
            .and_then(|body| parse_wallet_seqno(&body))
            .map_err(|error| self.resolution_failed_error(owner, error.developer_message))?;

        if indexed_seqno > pending.seqno {
            // The transaction and wallet-state requests are separate DB snapshots.
            // Recheck after observing the increment so a commit between the first
            // two reads cannot be misclassified as a replacement.
            let executed = self
                .execute_resolution_request(owner, &requests.executed_recheck)
                .await?
                .and_then(|body| parse_executed_message(&body))
                .map_err(|error| self.resolution_failed_error(owner, error.developer_message))?;

            if let Some(executed) = executed {
                return self
                    .persist_send_resolution(
                        owner,
                        pending,
                        SendResolution::Confirmed {
                            transaction_hash: executed.transaction_hash,
                            transaction_lt: executed.transaction_lt,
                        },
                    )
                    .await;
            }

            return self
                .persist_send_resolution(owner, pending, SendResolution::Replaced)
                .await;
        }

        if pending_observed
            && resolution_window_expired(
                provider_time,
                pending.valid_until,
                config.resolution_margin_seconds,
            )
        {
            return self
                .persist_send_resolution(owner, pending, SendResolution::Expired)
                .await;
        }

        Ok(ResolvedPending {
            resolution: SendResolution::StillPending(PendingReason::AwaitingWindow),
            journal: pending.current_journal(),
        })
    }

    /// Persists terminal evidence with optimistic concurrency and reconciles a
    /// competing writer instead of overwriting its decision.
    async fn persist_send_resolution(
        &self,
        owner: ResolutionOwner,
        mut pending: PendingSendRecord,
        resolution: SendResolution,
    ) -> Result<ResolvedPending, WalletClientError> {
        // A resolver can race another resolver or a restarted process. Never
        // overwrite their conclusion blindly: advance the exact journal version
        // we inspected, then accept an already persisted terminal result or
        // retry only if the same operation is still pending.
        for _ in 0..3 {
            let mutation = pending
                .terminal_mutation(&resolution)
                .map_err(|error| self.resolution_workflow_error(owner, &error))?
                .ok_or_else(|| {
                    self.resolution_failed_error(owner, "pending resolution was not terminal")
                })?;
            let replacement = mutation.replacement.clone();
            let result = self
                .platform_host
                .compare_exchange_journal(mutation)
                .await
                .map_err(|error| self.resolution_failed_error(owner, error.to_string()))?;
            self.ensure_current_resolution(owner)?;

            if result.applied {
                return Ok(ResolvedPending {
                    resolution,
                    journal: replacement,
                });
            }

            let current = result.current.ok_or_else(|| {
                self.resolution_failed_error(owner, "send journal disappeared during resolution")
            })?;
            if let Some(current_resolution) =
                terminal_send_resolution(&current, &pending.record_id, &pending.source)
                    .map_err(|error| self.resolution_workflow_error(owner, &error))?
            {
                return Ok(ResolvedPending {
                    resolution: current_resolution,
                    journal: current,
                });
            }

            let current_pending =
                pending_send_record(&current, &pending.record_id, &pending.source)
                    .map_err(|error| self.resolution_workflow_error(owner, &error))?
                    .ok_or_else(|| {
                        self.resolution_failed_error(
                            owner,
                            "send journal changed during resolution",
                        )
                    })?;
            if current_pending.operation_id != pending.operation_id
                || current_pending.message_hash != pending.message_hash
            {
                return Err(self.resolution_failed_error(
                    owner,
                    "another send replaced the journal during resolution",
                ));
            }
            pending = current_pending;
        }

        Err(self
            .resolution_failed_error(owner, "send journal remained contended during resolution"))
    }

    /// Dispatches HTTP through the owning operation's tracker and returns its
    /// engine-checked body.
    ///
    /// Cancellation and stale-generation checks remain correct in both entry
    /// paths, and response processing starts only after tracking is finished.
    async fn execute_resolution_request(
        &self,
        owner: ResolutionOwner,
        request: &HttpRequest,
    ) -> Result<Result<Vec<u8>, DomainError>, WalletClientError> {
        match owner {
            ResolutionOwner::Send(generation) => {
                self.execute_tracked_send_request(generation, request).await
            }
            ResolutionOwner::Standalone(generation) => {
                self.start_resolution_http_request(generation, request.id)?;
                let result = self.http_host.execute_http(request.clone()).await;
                self.finish_resolution_http_request(generation, request.id)?;
                Ok(process_response(request, result))
            }
        }
    }

    /// Registers a standalone request before invoking the host, making it
    /// visible to shutdown cancellation.
    fn start_resolution_http_request(
        &self,
        generation: u64,
        request_id: crate::HttpRequestId,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        let Some((active_generation, request_ids)) = state.active_resolution.as_mut() else {
            return Err(WalletClientError::StateUnavailable);
        };
        if *active_generation != generation {
            return Err(WalletClientError::StateUnavailable);
        }
        request_ids.push(request_id);
        Ok(())
    }

    /// Removes a completed request only from the generation that started it.
    fn finish_resolution_http_request(
        &self,
        generation: u64,
        request_id: crate::HttpRequestId,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        let Some((active_generation, request_ids)) = state.active_resolution.as_mut() else {
            return Err(WalletClientError::StateUnavailable);
        };
        if *active_generation != generation {
            return Err(WalletClientError::StateUnavailable);
        }
        request_ids.retain(|active| *active != request_id);
        Ok(())
    }

    /// Rejects late callbacks after cancellation, shutdown, or supersession.
    fn ensure_current_resolution(&self, owner: ResolutionOwner) -> Result<(), WalletClientError> {
        match owner {
            ResolutionOwner::Send(generation) => self.ensure_current_send(generation),
            ResolutionOwner::Standalone(generation) => {
                let state = self.lock()?;
                if state.is_current(OperationFamily::Resolution, generation) {
                    Ok(())
                } else {
                    Err(WalletClientError::StateUnavailable)
                }
            }
        }
    }

    /// Preserves send-specific state transitions for inline resolution while
    /// mapping standalone journal failures to its own operation lifecycle.
    fn resolution_workflow_error(
        &self,
        owner: ResolutionOwner,
        error: &SendWorkflowError,
    ) -> WalletClientError {
        match owner {
            ResolutionOwner::Send(generation) => self.send_workflow_error(generation, error),
            ResolutionOwner::Standalone(_) => {
                self.resolution_failed_error(owner, error.to_string())
            }
        }
    }

    /// Produces a bounded public failure and releases the matching active slot.
    fn resolution_failed_error(
        &self,
        owner: ResolutionOwner,
        message: impl Into<String>,
    ) -> WalletClientError {
        match owner {
            ResolutionOwner::Send(generation) => self.send_failed_error(generation, message),
            ResolutionOwner::Standalone(generation) => {
                let diagnostic = bounded_diagnostic(message.into());
                if let Ok(mut state) = self.lock()
                    && state.is_current(OperationFamily::Resolution, generation)
                {
                    state.active_resolution = None;
                }
                WalletClientError::SendFailed { diagnostic }
            }
        }
    }
}

/// Reports whether provider time is strictly beyond the signed validity window
/// and its indexer synchronization margin.
///
/// Saturation is conservative: if the boundary cannot fit in `u64`, the send
/// remains pending instead of being unlocked early for a replacement signature.
const fn resolution_window_expired(
    provider_time: u64,
    valid_until: u64,
    margin_seconds: u64,
) -> bool {
    provider_time > valid_until.saturating_add(margin_seconds)
}

#[cfg(kani)]
mod verification {
    use super::resolution_window_expired;

    /// Proves that expiration is impossible before both the signed deadline and
    /// the complete resolver margin have elapsed.
    #[kani::proof]
    fn resolution_never_expires_before_its_boundary() {
        let provider_time = kani::any::<u64>();
        let valid_until = kani::any::<u64>();
        let margin_seconds = kani::any::<u64>();
        let boundary = valid_until.saturating_add(margin_seconds);

        if resolution_window_expired(provider_time, valid_until, margin_seconds) {
            assert!(provider_time > boundary);
            assert!(provider_time > valid_until);
        }
    }

    /// Proves that an unrepresentable deadline remains pending for every
    /// possible provider timestamp instead of wrapping and expiring early.
    #[kani::proof]
    fn overflowing_resolution_boundary_never_expires() {
        let provider_time = kani::any::<u64>();
        let valid_until = kani::any::<u64>();
        let margin_seconds = kani::any::<u64>();

        kani::assume(valid_until.checked_add(margin_seconds).is_none());
        kani::cover!();
        assert!(!resolution_window_expired(
            provider_time,
            valid_until,
            margin_seconds
        ));
    }

    /// Proves the strict boundary behavior: equality remains pending and the
    /// first representable second after the boundary expires the send.
    #[kani::proof]
    fn resolution_expires_immediately_after_a_representable_boundary() {
        let valid_until = kani::any::<u64>();
        let margin_seconds = kani::any::<u64>();
        let Some(boundary) = valid_until.checked_add(margin_seconds) else {
            return;
        };

        assert!(!resolution_window_expired(
            boundary,
            valid_until,
            margin_seconds
        ));
        if let Some(after_boundary) = boundary.checked_add(1) {
            assert!(resolution_window_expired(
                after_boundary,
                valid_until,
                margin_seconds
            ));
        }
    }
}
