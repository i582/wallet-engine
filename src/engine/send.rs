//! V5R1 transfer orchestration.
use super::client::WalletClient;
use super::http::{build_toncenter_request, evaluate_response};
use super::send_http::{
    SendBocResponse, build_send_boc_request, build_seqno_request, is_explicit_send_rejection,
    parse_send_response, parse_seqno,
};
use super::send_state::SensitiveBytes;
use super::state::{OperationFamily, ensure_running};
use super::validation::validate_send;

use crate::domain::bounded_diagnostic;
use crate::wallet::send::{FreshSendAccount, SendDirective, SendWorkflow};
use crate::wallet::transfer::{derive_source, prepare_transfer};
use crate::{AccountStatus, SendPhase, SendRequest, SendResult, WalletClientError};

use super::provider::parse_account;

#[uniffi::export]
impl WalletClient {
    /// Signs, records, and submits one V5R1 transfer.
    ///
    /// The engine first loads fresh account state. It then authorizes mnemonic
    /// access and signs inside Rust. Before submission, it stores the exact BOC
    /// with compare-and-swap.
    ///
    /// A transport error after submission produces `SubmissionUnknown`. Do not
    /// create a replacement transfer for the same funds. This crate does not
    /// reconcile the result or stream chain confirmation.
    ///
    /// Workflow failures return [`WalletClientError::SendFailed`] with the same
    /// bounded diagnostic published in `snapshot().send.error_message`.
    pub async fn send(&self, request: SendRequest) -> Result<SendResult, WalletClientError> {
        // Reject malformed input before reserving IDs or changing observable state.
        validate_send(&request)?;

        // Reserve one send generation and every HTTP ID under the state lock.
        // This makes concurrent sends single-flight and lets late callbacks be ignored safely.
        let (
            generation,
            config,
            expected_source,
            account_request,
            seqno_request,
            submit_request_id,
            journal_key,
            mut workflow,
        ) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            if state.active_send.is_some() {
                return Err(WalletClientError::StateUnavailable);
            }
            state.send_generation = state
                .send_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.send_generation;
            let config = state.config.clone();
            let expected_source = config.parsed_address()?;

            let account_request = build_toncenter_request(
                &config,
                state.allocate_request_id()?,
                "getAddressInformation",
                &[("address", config.address.as_str())],
            )?;
            let seqno_request = build_seqno_request(&config, state.allocate_request_id()?)?;
            let submit_request_id = state.allocate_request_id()?;
            let mut workflow = SendWorkflow::new(
                config.record_id.clone(),
                config.address.clone(),
                request.clone(),
            );
            let directive = workflow
                .begin()
                .map_err(|_| WalletClientError::InvalidConfig)?;
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
        let directive = workflow
            .journal_loaded(journal_record)
            .map_err(|error| self.send_failed_error(generation, error.to_string()))?;
        let SendDirective::FetchFreshAccount = directive else {
            return Err(self.send_failed_error(generation, "invalid send journal transition"));
        };
        self.publish_send_workflow(generation, &workflow)?;

        // Fetch current account status before authorization. A stale cached status can produce
        // an invalid seqno or incorrectly include StateInit in the external message.
        let account_response = self
            .execute_tracked_send_request(generation, &account_request)
            .await?;
        let account = evaluate_response(&account_request, account_response, parse_account)
            .map_err(|error| self.send_failed_error(generation, error.developer_message))?;
        // Active wallets require a fresh seqno for replay protection. A wallet that is not yet
        // deployed starts at seqno zero; the workflow rejects unsupported account states later.
        let seqno = if account.status == AccountStatus::Active {
            let seqno_response = self
                .execute_tracked_send_request(generation, &seqno_request)
                .await?;

            evaluate_response(&seqno_request, seqno_response, parse_seqno)
                .map_err(|error| self.send_failed_error(generation, error.developer_message))?
        } else {
            0
        };
        let fresh = FreshSendAccount {
            status: account.status,
            seqno,
            observed_at: account.sync_utime.unwrap_or_default(),
        };

        let directive = workflow
            .fresh_account_loaded(fresh.clone())
            .map_err(|error| self.send_failed_error(generation, error.to_string()))?;
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
        let source = derive_source(secret.as_slice(), config.network)
            .map_err(|_| self.send_failed_error(generation, "protected mnemonic is invalid"))?;

        if source != expected_source {
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

        // Base the expiration on the provider's synchronized chain view. Device clocks can be
        // wrong, while this timestamp belongs to the same fresh state used for status and seqno.
        let provider_time = account.sync_utime.ok_or_else(|| {
            self.send_failed_error(
                generation,
                "fresh account state did not include provider synchronization time",
            )
        })?;
        let provider_time = u32::try_from(provider_time).map_err(|_| {
            self.send_failed_error(
                generation,
                "provider synchronization time does not fit the wallet timestamp field",
            )
        })?;
        let valid_until = provider_time
            .checked_add(config.send_validity_seconds)
            .ok_or_else(|| {
                self.send_failed_error(generation, "transfer expiration timestamp overflow")
            })?;

        let prepared = prepare_transfer(
            secret.as_slice(),
            &config.record_id,
            &config.address,
            config.network,
            &request,
            &fresh,
            valid_until,
        )
        .map_err(|error| {
            self.send_failed_error(generation, format!("failed to prepare transfer: {error}"))
        })?;

        let summary = prepared.public_summary();
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
        let directive = workflow.journal_persisted(journal).map_err(|error| {
            if journal_applied {
                self.submission_unknown_error(generation, error.to_string())
            } else {
                self.send_failed_error(generation, error.to_string())
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

        let final_directive = match submit_result {
            Ok(response) => {
                match evaluate_response(&submit_request, Ok(response), parse_send_response) {
                    Ok(SendBocResponse::Accepted) => workflow.submission_succeeded(None),
                    Ok(SendBocResponse::Rejected(message)) => workflow.submission_rejected(message),
                    Err(error) if is_explicit_send_rejection(&error) => {
                        workflow.submission_rejected(error.developer_message)
                    }
                    Err(error) => workflow.submission_unknown(error.developer_message),
                }
            }
            Err(error) => workflow.submission_unknown(bounded_diagnostic(error.to_string())),
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
        workflow
            .journal_persisted(journal)
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
        let _ = summary;
        Ok(SendResult {
            operation_id: request.operation_id,
            message_hash,
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
