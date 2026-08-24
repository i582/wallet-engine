//! Durable owner-signed internal messages for TON Connect relayers.

use crate::transport::build_toncenter_v2_request;
use crate::wallet::send::{FreshSendAccount, SendDirective, SendResolution, SendWorkflow};
use crate::wallet::transfer::{derive_source, prepare_internal_signed_transfer};
use crate::{
    AccountStatus, SendPhase, SendRequest, SignMessageRequest, SignMessageResult, WalletClientError,
};

use super::WalletClient;
use super::expiration::resolve_send_expiration;
use super::provider::parse_account;
use super::resolution::ResolutionRequests;
use super::send_http::{build_seqno_request, parse_seqno};
use super::send_state::SensitiveBytes;
use super::state::{OperationFamily, ensure_running};
use crate::wallet::send::pending_send_record;

#[uniffi::export]
impl WalletClient {
    /// Signs and durably records a Wallet V5 `internal_signed` request.
    ///
    /// This method does not submit the message. The returned BOC is a complete
    /// relaxed internal message that the caller can give to a relayer until
    /// `valid_until`. The shared send journal prevents another signature from
    /// silently reusing the same wallet sequence number.
    pub async fn sign_message(
        &self,
        request: SignMessageRequest,
    ) -> Result<SignMessageResult, WalletClientError> {
        let send_request = SendRequest {
            operation_id: request.operation_id.clone(),
            force: request.force,
            intent: request.intent,
        };
        let (
            generation,
            config,
            expected_source,
            account_request,
            seqno_request,
            resolution_request_ids,
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
            let mut workflow = SendWorkflow::new_internal(
                config.record_id.clone(),
                expected_source.clone(),
                send_request.clone(),
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
                journal_key,
                workflow,
            )
        };

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
        let account = self
            .execute_tracked_send_request(generation, &account_request)
            .await?
            .and_then(|body| parse_account(&body))
            .map_err(|error| self.send_failed_error(generation, error.developer_message))?;
        let provider_time = account.sync_utime;

        let journal_record = if let Some(pending) = pending {
            if send_request.force && pending.can_force_retry() {
                journal_record
            } else {
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

        if send_request.intent.exact_value_total().is_err() {
            let error = WalletClientError::InvalidSendRequest;
            self.fail_send(generation, error.to_string())?;
            return Err(error);
        }

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
        let valid_until = resolve_send_expiration(
            &send_request.intent.expiration,
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

        let secret = SensitiveBytes::new(
            self.platform_host
                .read_protected_secret(secret_request)
                .await
                .map_err(|error| self.send_failed_error(generation, error.to_string()))?,
        );
        self.ensure_current_send(generation)?;
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
            return Err(self.send_failed_error(generation, "invalid authorization transition"));
        };
        self.publish_send_workflow(generation, &workflow)?;

        let prepared = prepare_internal_signed_transfer(
            secret.as_slice(),
            &config.record_id,
            &expected_source,
            config.network,
            &send_request,
            &fresh,
            valid_until,
        )
        .map_err(|error| {
            self.send_failed_error(
                generation,
                format!("failed to prepare internal signed message: {error}"),
            )
        })?;
        let directive = workflow
            .transfer_prepared(prepared)
            .map_err(|error| self.send_failed_error(generation, error.to_string()))?;
        let SendDirective::PersistJournal(mutation) = directive else {
            return Err(self.send_failed_error(generation, "invalid persistence transition"));
        };
        self.publish_send_workflow(generation, &workflow)?;

        self.begin_send_commit(generation)?;
        let journal = self
            .platform_host
            .compare_exchange_journal(mutation)
            .await
            .map_err(|error| self.submission_unknown_error(generation, error.to_string()))?;
        self.ensure_current_send(generation)?;
        let journal_applied = journal.applied;
        let directive = workflow.journal_persisted(&journal).map_err(|error| {
            if journal_applied {
                self.submission_unknown_error(generation, error.to_string())
            } else {
                self.send_workflow_error(generation, &error)
            }
        })?;
        let SendDirective::HandOff {
            internal_boc,
            valid_until,
        } = directive
        else {
            return Err(self.submission_unknown_error(
                generation,
                "invalid internal-message handoff transition",
            ));
        };
        let phase = workflow.snapshot().phase;
        debug_assert_eq!(phase, SendPhase::HandedOff);

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

        Ok(SignMessageResult {
            operation_id: request.operation_id,
            internal_boc,
            valid_until,
            phase,
        })
    }
}
