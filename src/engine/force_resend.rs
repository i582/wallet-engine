//! Explicit same-seqno replacement of one unresolved durable transfer.

use crate::domain::bounded_diagnostic;
use crate::wallet::send::{
    FreshSendAccount, SendResolution, SendStage, pending_send_record, send_snapshot_from_journal,
};
use crate::wallet::transfer::{derive_source, prepare_transfer};
use crate::{
    AccountStatus, JournalKey, ProtectedSecretRead, SecretAccessReason, SendPhase, SendResult,
    WalletClientError,
};

use super::WalletClient;
use super::http::{build_toncenter_v2_request, evaluate_response};
use super::provider::parse_account;
use super::resolution::ResolutionRequests;
use super::send_http::{
    SendBocResponse, build_send_boc_request, is_explicit_send_rejection, parse_send_response,
};
use super::send_state::SensitiveBytes;
use super::state::{OperationFamily, ensure_running};

#[uniffi::export]
impl WalletClient {
    /// Explicitly signs and submits one replacement with the unresolved
    /// journal attempt's exact seqno and transfer intent.
    ///
    /// V5R1 replay protection makes the two messages mutually exclusive. The
    /// old BOC remains in the same CAS record so later resolution can identify
    /// which hash won.
    pub async fn force_resend(&self) -> Result<SendResult, WalletClientError> {
        let (
            generation,
            config,
            source,
            local_secret_ref,
            journal_key,
            account_request,
            resolution_ids,
            submit_request_id,
            initial_snapshot,
        ) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            if state.active_send.is_some() || state.active_resolution.is_some() {
                return Err(WalletClientError::SendAlreadyInProgress);
            }
            let local_secret_ref = state
                .config
                .local_secret_ref
                .clone()
                .ok_or(WalletClientError::LocalSigningUnavailable)?;
            state.send_generation = state
                .send_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.send_generation;
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
            let submit_request_id = state.allocate_request_id()?;
            let initial_snapshot = state.snapshot.send.clone();
            state.active_send = Some((generation, Vec::new()));
            state.send_commit_started = false;
            state.send_workflow = None;
            (
                generation,
                config.clone(),
                source,
                local_secret_ref,
                JournalKey {
                    record_id: config.record_id,
                    slot: crate::wallet::send::SEND_SLOT.to_owned(),
                },
                account_request,
                resolution_ids,
                submit_request_id,
                initial_snapshot,
            )
        };

        let result = async {
            let journal = self
                .platform_host
                .load_journal(journal_key)
                .await
                .map_err(|error| WalletClientError::SendFailed {
                    diagnostic: bounded_diagnostic(error.to_string()),
                })?
                .ok_or(WalletClientError::ForceResendUnavailable)?;
            self.ensure_current_send(generation)?;
            let pending = pending_send_record(&journal, &config.record_id, &source)
                .map_err(|error| WalletClientError::SendFailed {
                    diagnostic: bounded_diagnostic(error.to_string()),
                })?
                .ok_or(WalletClientError::ForceResendUnavailable)?;
            let request = pending
                .force_resend_request()
                .map_err(|_| WalletClientError::ForceResendUnavailable)?;

            let account_response = self
                .execute_tracked_send_request(generation, &account_request)
                .await?;
            let account = evaluate_response(&account_request, account_response, parse_account)
                .map_err(|error| WalletClientError::SendFailed {
                    diagnostic: error.developer_message,
                })?;
            let provider_time =
                account
                    .sync_utime
                    .ok_or_else(|| WalletClientError::SendFailed {
                        diagnostic:
                            "fresh account state did not include provider synchronization time"
                                .to_owned(),
                    })?;

            // Recheck immediately before authorization. Force resend is not
            // offered after either terminal hash/seqno/expiry proof appears.
            let requests = ResolutionRequests::new(&config, &pending, resolution_ids)?;
            let resolved = self
                .resolve_pending_for_send(
                    generation,
                    &config,
                    pending.clone(),
                    provider_time,
                    &requests,
                )
                .await?;
            if !matches!(resolved.resolution, SendResolution::StillPending(_)) {
                let snapshot =
                    pending.snapshot(&resolved.resolution, config.resolution_poll_interval_ms);
                self.publish_prior_send_resolution(generation, snapshot)?;
                return Err(WalletClientError::ForceResendUnavailable);
            }

            let provider_time =
                u32::try_from(provider_time).map_err(|_| WalletClientError::SendFailed {
                    diagnostic: "provider time does not fit the wallet timestamp field".to_owned(),
                })?;
            let provider_valid_until = provider_time
                .checked_add(config.send_validity_seconds)
                .ok_or_else(|| WalletClientError::SendFailed {
                    diagnostic: "force-resend expiration timestamp overflow".to_owned(),
                })?;
            // Ed25519 signing is deterministic. If provider time has not moved,
            // the same intent, seqno, and deadline would reproduce the exact
            // old BOC and hash instead of creating a separately observable
            // replacement. Advance the deadline by at least one second.
            let valid_until =
                provider_valid_until.max(pending.valid_until.checked_add(1).ok_or_else(|| {
                    WalletClientError::SendFailed {
                        diagnostic: "force-resend expiration timestamp overflow".to_owned(),
                    }
                })?);
            let fresh = FreshSendAccount {
                status: account.status,
                seqno: pending.seqno,
                observed_at: u64::from(provider_time),
            };
            match fresh.status {
                AccountStatus::Active => {}
                AccountStatus::Nonexistent | AccountStatus::Uninitialized if fresh.seqno == 0 => {}
                status => return Err(WalletClientError::SendAccountUnavailable { status }),
            }

            self.publish_force_phase(generation, &request.operation_id, SendPhase::Authorizing)?;
            let secret = SensitiveBytes::new(
                self.platform_host
                    .read_protected_secret(ProtectedSecretRead {
                        secret_ref: local_secret_ref,
                        reason: SecretAccessReason::SignTransfer,
                        prompt: "Authenticate to replace the pending GRAM transfer".to_owned(),
                    })
                    .await
                    .map_err(|error| WalletClientError::SendFailed {
                        diagnostic: bounded_diagnostic(error.to_string()),
                    })?,
            );
            self.ensure_current_send(generation)?;
            let derived = derive_source(secret.as_slice(), config.network)
                .map_err(|_| WalletClientError::InvalidProtectedSecret)?;
            if derived != source {
                return Err(WalletClientError::InvalidProtectedSecret);
            }

            self.publish_force_phase(generation, &request.operation_id, SendPhase::Preparing)?;
            let prepared = prepare_transfer(
                secret.as_slice(),
                &config.record_id,
                &source,
                config.network,
                &request,
                &fresh,
                valid_until,
            )
            .map_err(|error| WalletClientError::SendFailed {
                diagnostic: bounded_diagnostic(format!("failed to prepare force resend: {error}")),
            })?;
            let message_hash = prepared.message_hash.clone();
            let submit_request =
                build_send_boc_request(&config, submit_request_id, prepared.signed_boc.as_bytes())?;
            let mutation = pending.force_resend_mutation(&prepared).map_err(|error| {
                WalletClientError::SendFailed {
                    diagnostic: bounded_diagnostic(error.to_string()),
                }
            })?;
            let prepared_journal = mutation.replacement.clone();

            self.publish_force_phase(generation, &request.operation_id, SendPhase::Persisting)?;
            self.begin_send_commit(generation)?;
            let persisted = self
                .platform_host
                .compare_exchange_journal(mutation)
                .await
                .map_err(|error| WalletClientError::SubmissionUnknown {
                    diagnostic: bounded_diagnostic(error.to_string()),
                })?;
            self.ensure_current_send(generation)?;
            if !persisted.applied {
                if let Some(current_journal) = persisted.current {
                    let current_pending =
                        pending_send_record(&current_journal, &config.record_id, &source).map_err(
                            |error| WalletClientError::SendFailed {
                                diagnostic: bounded_diagnostic(error.to_string()),
                            },
                        )?;
                    if current_pending.as_ref().is_some_and(|current| {
                        current.journal_version == pending.journal_version
                            && current.message_hash == pending.message_hash
                            && current.superseded_message_hash.is_none()
                    }) {
                        // The host reported a clean CAS miss without changing
                        // the record. Nothing durable happened in this force
                        // attempt, so the original pending state remains valid.
                        self.finish_force_send(generation, initial_snapshot.clone())?;
                        return Err(WalletClientError::SendAlreadyInProgress);
                    }
                    let snapshot = send_snapshot_from_journal(
                        &current_journal,
                        &config.record_id,
                        &source,
                        config.resolution_poll_interval_ms,
                    )
                    .map_err(|error| WalletClientError::SendFailed {
                        diagnostic: bounded_diagnostic(error.to_string()),
                    })?;
                    self.finish_force_send(generation, snapshot)?;
                    return Err(WalletClientError::ForceResendUnavailable);
                }
                return Err(WalletClientError::StateUnavailable);
            }
            let prepared_pending =
                pending_send_record(&prepared_journal, &config.record_id, &source)
                    .map_err(|error| WalletClientError::SendFailed {
                        diagnostic: bounded_diagnostic(error.to_string()),
                    })?
                    .ok_or(WalletClientError::StateUnavailable)?;

            self.publish_force_phase(generation, &request.operation_id, SendPhase::Submitting)?;
            let submit_result = self
                .execute_tracked_send_request(generation, &submit_request)
                .await?;
            let (stage, diagnostic) = match submit_result {
                Ok(response) => {
                    match evaluate_response(&submit_request, Ok(response), parse_send_response) {
                        Ok(SendBocResponse::Accepted) => (SendStage::Submitted, None),
                        Ok(SendBocResponse::Rejected(message)) => (
                            SendStage::SubmissionUnknown,
                            Some(format!(
                                "replacement rejected; original remains live: {message}"
                            )),
                        ),
                        Err(error) if is_explicit_send_rejection(&error) => (
                            SendStage::SubmissionUnknown,
                            Some(format!(
                                "replacement rejected; original remains live: {}",
                                error.developer_message
                            )),
                        ),
                        Err(error) => (SendStage::SubmissionUnknown, Some(error.developer_message)),
                    }
                }
                Err(error) => (
                    SendStage::SubmissionUnknown,
                    Some(bounded_diagnostic(error.to_string())),
                ),
            };
            let mutation = prepared_pending
                .force_submission_mutation(stage, diagnostic)
                .map_err(|error| WalletClientError::SubmissionUnknown {
                    diagnostic: bounded_diagnostic(error.to_string()),
                })?;
            let classified_journal = mutation.replacement.clone();
            let classified = self
                .platform_host
                .compare_exchange_journal(mutation)
                .await
                .map_err(|error| WalletClientError::SubmissionUnknown {
                    diagnostic: bounded_diagnostic(error.to_string()),
                })?;
            self.ensure_current_send(generation)?;
            let current_journal = if classified.applied {
                classified_journal
            } else {
                classified
                    .current
                    .ok_or(WalletClientError::StateUnavailable)?
            };
            let current = pending_send_record(&current_journal, &config.record_id, &source)
                .map_err(|error| WalletClientError::SubmissionUnknown {
                    diagnostic: bounded_diagnostic(error.to_string()),
                })?;
            let Some(current) = current else {
                // Another client can resolve either hash between our submit
                // and classification CAS. Publish that durable winner instead
                // of overwriting it with an ambiguous local phase.
                let snapshot = send_snapshot_from_journal(
                    &current_journal,
                    &config.record_id,
                    &source,
                    config.resolution_poll_interval_ms,
                )
                .map_err(|error| WalletClientError::SubmissionUnknown {
                    diagnostic: bounded_diagnostic(error.to_string()),
                })?;
                let phase = snapshot.phase;
                self.finish_force_send(generation, snapshot)?;
                return Ok(SendResult {
                    operation_id: request.operation_id,
                    message_hash,
                    phase,
                });
            };
            let snapshot = current.snapshot(
                &SendResolution::StillPending(crate::PendingReason::AwaitingWindow),
                config.resolution_poll_interval_ms,
            );
            self.finish_force_send(generation, snapshot)?;

            let phase = self
                .resolve_pending_active()
                .await
                .map_or_else(|_| stage.public_phase(), |snapshot| snapshot.phase);
            Ok(SendResult {
                operation_id: request.operation_id,
                message_hash,
                phase,
            })
        }
        .await;

        if result.is_err() {
            self.release_force_send(generation, initial_snapshot)?;
        }
        result
    }
}

impl WalletClient {
    fn publish_force_phase(
        &self,
        generation: u64,
        operation_id: &str,
        phase: SendPhase,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.snapshot.send.operation_id = Some(operation_id.to_owned());
        state.snapshot.send.phase = phase;
        state.snapshot.send.error_message = None;
        state.next_revision()?;
        Ok(())
    }

    fn finish_force_send(
        &self,
        generation: u64,
        snapshot: crate::SendSnapshot,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.snapshot.send = snapshot;
        state.active_send = None;
        state.send_commit_started = false;
        state.next_revision()?;
        Ok(())
    }

    fn release_force_send(
        &self,
        generation: u64,
        initial_snapshot: crate::SendSnapshot,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.is_current(OperationFamily::Send, generation) {
            if state.send_commit_started {
                // A journal CAS failure is ambiguous: the replacement record
                // may already be durable even if the host lost its response.
                // Keep the wallet locked and disable another force attempt.
                state.snapshot.send.phase = SendPhase::SubmissionUnknown;
                if let Some(resolution) = state.snapshot.send.resolution.as_mut() {
                    resolution.can_force_retry = false;
                }
            } else if state.snapshot.send.operation_id != initial_snapshot.operation_id {
                // Authorization/preparation failures happen before any durable
                // mutation. Restore the previous pending attempt instead of
                // leaving a transient force-resend phase in the public state.
                state.snapshot.send = initial_snapshot;
            }
            state.active_send = None;
            state.send_commit_started = false;
            state.next_revision()?;
        }
        Ok(())
    }
}
