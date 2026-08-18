//! Wallet transfer orchestration.
use super::client::WalletClient;
use super::expiration::resolve_send_expiration;
use super::http::build_toncenter_v2_request;
use super::send_http::{
    SendBocResponse, build_send_boc_request, build_seqno_request, is_explicit_send_rejection,
    parse_send_response, parse_seqno,
};
use super::send_state::SensitiveBytes;
use super::state::{OperationFamily, ensure_running};

use crate::wallet::send::{FreshSendAccount, SendDirective, SendWorkflow};
use crate::wallet::transfer::{derive_source, prepare_transfer};
use crate::{AccountStatus, SendPhase, SendRequest, SendResult, WalletClientError};

use super::provider::parse_account;
use super::resolution::ResolutionRequests;
use crate::wallet::send::{SendResolution, pending_send_record};

#[uniffi::export]
impl WalletClient {
    /// Signs, records, and submits one wallet transfer.
    ///
    /// A preceding [`Self::preview_send`] is an informational UI step, not a
    /// prerequisite. This method independently reloads account state and seqno,
    /// calculates a new validity window, and builds the real message only after
    /// protected-secret authorization.
    ///
    /// A transport error after submission produces `SubmissionUnknown`. A later
    /// [`Self::resolve_pending`] call, refresh, or send attempt can reconcile the
    /// persisted message against provider evidence. By default, the unresolved
    /// send blocks a new signature. `SendRequest::force` overrides that block
    /// after the application obtains explicit user confirmation.
    ///
    /// Workflow failures return a typed send error. The same bounded diagnostic
    /// is published in `snapshot().send.error_message`.
    pub async fn send(&self, request: SendRequest) -> Result<SendResult, WalletClientError> {
        // Reserve one send generation and every HTTP ID under the state lock.
        // This makes concurrent sends single-flight and lets late callbacks be ignored safely.
        let (
            generation,
            config,
            expected_source,
            account_request,
            seqno_request,
            resolution_request_ids,
            submit_request_id,
            journal_key,
            mut workflow,
        ) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            let local_secret_ref = state
                .config
                .local_secret_ref
                .clone()
                .ok_or(WalletClientError::LocalSigningUnavailable)?;

            if state.active_send.is_some() || state.active_resolution.is_some() {
                return Err(WalletClientError::SendAlreadyInProgress);
            }

            state.send_generation = state
                .send_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.send_generation;
            let config = state.config.clone();
            let expected_source = config.address.clone();

            let account_request = build_toncenter_v2_request(
                &config,
                state.allocate_request_id()?,
                "getAddressInformation",
                &[("address", config.address.as_str())],
            )?;
            let seqno_request = build_seqno_request(&config, state.allocate_request_id()?)?;
            let resolution_request_ids = [
                state.allocate_request_id()?,
                state.allocate_request_id()?,
                state.allocate_request_id()?,
                state.allocate_request_id()?,
            ];
            let submit_request_id = state.allocate_request_id()?;
            let mut workflow = SendWorkflow::new(
                config.record_id.clone(),
                expected_source.clone(),
                request.clone(),
                local_secret_ref,
            );
            let directive = workflow
                .begin()
                .map_err(|_| WalletClientError::InvalidSendRequest)?;
            let SendDirective::LoadJournal(journal_key) = directive else {
                return Err(WalletClientError::StateUnavailable);
            };
            state.active_send = Some((generation, Vec::new()));
            state.send_commit_started = false;
            state.snapshot.send = workflow.snapshot();
            state.send_workflow = Some(workflow.clone());
            state.next_revision()?;
            (
                generation,
                config,
                expected_source,
                account_request,
                seqno_request,
                resolution_request_ids,
                submit_request_id,
                journal_key,
                workflow,
            )
        };

        // Read the durable wallet-wide send slot before fetching or signing.
        // An unresolved earlier submission must block creation of a different signed message.
        let journal_record = self
            .platform_host
            .load_journal(journal_key)
            .await
            .map_err(|error| self.send_failed_error(generation, error.to_string()))?;

        self.ensure_current_send(generation)?;

        let pending = journal_record
            .as_ref()
            .map(|record| pending_send_record(record, &config.record_id, &expected_source))
            .transpose()
            .map_err(|error| self.send_workflow_error(generation, &error))?
            .flatten();

        // Fetch current account status before authorization. A stale cached status can produce
        // an invalid seqno or incorrectly include StateInit in the external message.
        let account = self
            .execute_tracked_send_request(generation, &account_request)
            .await?
            .and_then(|body| parse_account(&body))
            .map_err(|error| self.send_failed_error(generation, error.developer_message))?;

        let provider_time = account.sync_utime;

        let journal_record = if let Some(pending) = pending {
            if request.force && pending.can_force_retry() {
                journal_record
            } else {
                // Resolve the old signed message before authorizing a new one. Only
                // durable terminal evidence unlocks the wallet-wide send slot;
                // absence or a temporary provider failure must never become an
                // implicit permission to sign a potentially duplicate payment.
                let requests =
                    ResolutionRequests::new(&config, &pending, resolution_request_ids)
                        .map_err(|error| self.send_failed_error(generation, error.to_string()))?;
                let resolved = self
                    .resolve_pending_for_send(
                        generation,
                        &config,
                        pending.clone(),
                        provider_time,
                        &requests,
                    )
                    .await?;
                let snapshot = pending.snapshot(&resolved.resolution);
                self.publish_prior_send_resolution(generation, snapshot.clone())?;
                if matches!(resolved.resolution, SendResolution::StillPending(_)) {
                    return Err(self.block_send_for_pending(generation, snapshot)?);
                }
                Some(resolved.journal)
            }
        } else {
            journal_record
        };

        let directive = workflow
            .journal_loaded(journal_record)
            .map_err(|error| self.send_workflow_error(generation, &error))?;
        let SendDirective::FetchFreshAccount = directive else {
            return Err(self.send_failed_error(generation, "invalid send journal transition"));
        };
        self.publish_send_workflow(generation, &workflow)?;

        // Reject an impossible value before reading the mnemonic or creating a signed BOC.
        // Fees are intentionally not estimated here, so equality can still fail on-chain.
        let available = &account.balance_nanograms;
        let Ok(requested) = request.intent.exact_value_total() else {
            let error = WalletClientError::InvalidSendRequest;
            self.fail_send(generation, error.to_string())?;
            return Err(error);
        };
        if let Some(nanograms) = &requested
            && nanograms > available
        {
            let error = WalletClientError::InsufficientBalance {
                available_nanograms: available.clone(),
                requested_nanograms: nanograms.clone(),
            };
            self.fail_send(generation, error.to_string())?;
            return Err(error);
        }

        // Active wallets require a fresh seqno for replay protection. A wallet that is not yet
        // deployed starts at seqno zero; the workflow rejects unsupported account states later.
        let seqno = if account.status == AccountStatus::Active {
            self.execute_tracked_send_request(generation, &seqno_request)
                .await?
                .and_then(|body| parse_seqno(&body))
                .map_err(|error| self.send_failed_error(generation, error.developer_message))?
        } else {
            0
        };

        let fresh = FreshSendAccount {
            status: account.status,
            seqno,
        };

        // Use synchronized provider time for the real signature. A UI preview
        // can be several blocks old by the time the user confirms it.
        let valid_until = resolve_send_expiration(
            &request.intent.expiration,
            provider_time,
            config.send_validity_seconds,
        )
        .map_err(|error| self.send_failed_error(generation, error.to_string()))?;

        let directive = workflow
            .fresh_account_loaded(fresh.clone())
            .map_err(|error| self.send_workflow_error(generation, &error))?;
        let SendDirective::ReadProtectedSecret(secret_request) = directive else {
            return Err(self.send_failed_error(generation, "invalid secret-read transition"));
        };

        self.publish_send_workflow(generation, &workflow)?;

        // Ask the platform host for the mnemonic only after all public preconditions pass.
        // The RAII wrapper clears the Rust-owned byte buffer on every return path.
        let secret = SensitiveBytes::new(
            self.platform_host
                .read_protected_secret(secret_request)
                .await
                .map_err(|error| self.send_failed_error(generation, error.to_string()))?,
        );
        self.ensure_current_send(generation)?;

        // Derive the source again from the unlocked mnemonic. This prevents signing a transfer
        // for wallet A with a secret that belongs to wallet B.
        let source = derive_source(secret.as_slice(), config.network).map_err(|_| {
            let diagnostic = "protected mnemonic is invalid".to_owned();
            match self.fail_send(generation, diagnostic) {
                Ok(()) => WalletClientError::InvalidProtectedSecret,
                Err(error) => error,
            }
        })?;

        if &source != expected_source.as_address() {
            return Err(self.send_failed_error(
                generation,
                "protected mnemonic does not belong to this wallet",
            ));
        }

        let SendDirective::PrepareTransfer { .. } = workflow
            .authorization_succeeded()
            .map_err(|error| self.send_failed_error(generation, error.to_string()))?
        else {
            return Err(self.send_failed_error(generation, "invalid send authorization transition"));
        };

        self.publish_send_workflow(generation, &workflow)?;

        let prepared = prepare_transfer(
            secret.as_slice(),
            &config.record_id,
            &expected_source,
            config.network,
            &request,
            &fresh,
            valid_until,
        )
        .map_err(|error| {
            self.send_failed_error(generation, format!("failed to prepare transfer: {error}"))
        })?;
        let signed_boc = prepared.signed_boc.clone();

        let submit_request =
            build_send_boc_request(&config, submit_request_id, &prepared.signed_boc).map_err(
                |_| self.send_failed_error(generation, "failed to construct submission"),
            )?;

        let directive = workflow
            .transfer_prepared(prepared)
            .map_err(|error| self.send_failed_error(generation, error.to_string()))?;

        let SendDirective::PersistJournal(mutation) = directive else {
            return Err(self.send_failed_error(generation, "invalid send persistence transition"));
        };

        self.publish_send_workflow(generation, &workflow)?;

        // Start the irreversible boundary before awaiting durable CAS. After this point cancel
        // returns TooLate because the exact BOC can survive a crash or already be in flight.
        self.begin_send_commit(generation)?;
        let journal = match self.platform_host.compare_exchange_journal(mutation).await {
            Ok(journal) => journal,
            Err(error) => {
                return Err(self.submission_unknown_error(generation, error.to_string()));
            }
        };
        self.ensure_current_send(generation)?;

        let journal_applied = journal.applied;
        let directive = workflow.journal_persisted(&journal).map_err(|error| {
            if journal_applied {
                self.submission_unknown_error(generation, error.to_string())
            } else {
                self.send_workflow_error(generation, &error)
            }
        })?;

        let SendDirective::Submit {
            signed_boc: _,
            message_hash,
        } = directive
        else {
            return Err(
                self.submission_unknown_error(generation, "invalid send submission transition")
            );
        };
        workflow
            .submission_started()
            .map_err(|error| self.submission_unknown_error(generation, error.to_string()))?;
        self.publish_send_workflow(generation, &workflow)?;

        // Submit exactly the BOC stored above. Transport or malformed-response failures are
        // classified as SubmissionUnknown because the provider might have accepted the POST.
        let submit_result = self
            .execute_tracked_send_request(generation, &submit_request)
            .await?;

        let final_directive = match submit_result.and_then(|body| parse_send_response(&body)) {
            Ok(SendBocResponse::Accepted) => workflow.submission_succeeded(None),
            Ok(SendBocResponse::Rejected(message)) => workflow.submission_rejected(message),
            Err(error) if is_explicit_send_rejection(&error) => {
                workflow.submission_rejected(error.developer_message)
            }
            Err(error) => workflow.submission_unknown(error.developer_message),
        }
        .map_err(|error| self.submission_unknown_error(generation, error.to_string()))?;
        let SendDirective::PersistJournal(mutation) = final_directive else {
            return Err(self
                .submission_unknown_error(generation, "invalid terminal persistence transition"));
        };
        self.publish_send_workflow(generation, &workflow)?;
        self.ensure_current_send(generation)?;

        // Persist the terminal outcome before publishing completion. On failure, keep the public
        // state unknown so a restart cannot silently replace a possibly submitted transfer.
        let journal = match self.platform_host.compare_exchange_journal(mutation).await {
            Ok(journal) => journal,
            Err(error) => {
                return Err(self.submission_unknown_error(generation, error.to_string()));
            }
        };
        self.ensure_current_send(generation)?;
        let _ = workflow
            .journal_persisted(&journal)
            .map_err(|error| self.submission_unknown_error(generation, error.to_string()))?;
        let phase = workflow.snapshot().phase;

        // Publish one terminal snapshot and release the single-flight send slot.
        {
            let mut state = self.lock()?;
            if state.is_current(OperationFamily::Send, generation) {
                state.snapshot.send = workflow.snapshot();
                state.send_workflow = Some(workflow);
                state.active_send = None;
                state.send_commit_started = false;
                state.next_revision()?;
            }
        }
        Ok(SendResult {
            operation_id: request.operation_id,
            message_hash,
            signed_boc,
            phase,
        })
    }

    /// Cancels the active send before its durable commit boundary.
    ///
    /// After journal persistence starts, this method returns
    /// [`WalletClientError::SendCancellationTooLate`]. The caller must let the
    /// send finish because the signed BOC can already be durable or submitted.
    pub async fn cancel_send(&self) -> Result<(), WalletClientError> {
        let request_ids = {
            let mut state = self.lock()?;
            if state.active_send.is_some() && state.send_commit_started {
                return Err(WalletClientError::SendCancellationTooLate);
            }

            let active = state.active_send.take();
            state.send_commit_started = false;

            if active.is_some() {
                if let Some(mut workflow) = state.send_workflow.take() {
                    let _ = workflow.cancel();
                    state.snapshot.send = workflow.snapshot();
                    state.send_workflow = Some(workflow);
                }
                state.snapshot.send.phase = SendPhase::Cancelled;
                state.next_revision()?;
            }

            active.map(|active| active.1).unwrap_or_default()
        };

        for request_id in request_ids {
            self.http_host.cancel_http(request_id).await;
        }

        Ok(())
    }
}
