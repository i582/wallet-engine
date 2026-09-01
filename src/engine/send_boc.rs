//! Durable submission of an already signed external message BOC.

use crate::transport::build_toncenter_v2_request;
use crate::wallet::send::{
    FreshSendAccount, SendDirective, SendResolution, SendWorkflow, pending_send_record,
};
use crate::wallet::transfer::prepare_signed_boc;
use crate::{AccountStatus, SendBocRequest, SendResult, WalletClientError};

use super::WalletClient;
use super::expiration::resolve_send_expiration;
use super::provider::parse_account;
use super::resolution::ResolutionRequests;
use super::send_http::{build_seqno_request, parse_seqno};
use super::state::ensure_running;

#[uniffi::export]
impl WalletClient {
    /// Durably records and submits an already signed external-message BOC.
    ///
    /// The request must carry the exact `seqno` and expiration covered by the
    /// BOC. The engine validates them against fresh provider state, stores the
    /// exact BOC in the wallet-wide journal before provider handoff, and exposes
    /// the normal pending-resolution and cancellation behavior.
    pub async fn send_boc(&self, request: SendBocRequest) -> Result<SendResult, WalletClientError> {
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
            let mut workflow = SendWorkflow::new_prepared_external(
                config.record_id.clone(),
                expected_source.clone(),
                request.operation_id.clone(),
                request.force,
                request.valid_until,
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
            if request.force && pending.can_force_retry() {
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

        let seqno = if account.status == AccountStatus::Active {
            self.execute_tracked_send_request(generation, &seqno_request)
                .await?
                .and_then(|body| parse_seqno(&body))
                .map_err(|error| self.send_failed_error(generation, error.developer_message))?
        } else {
            0
        };
        if seqno != request.seqno {
            return Err(self.send_failed_error(
                generation,
                format!(
                    "prepared BOC seqno {} does not match current wallet seqno {seqno}",
                    request.seqno
                ),
            ));
        }

        let _ = resolve_send_expiration(
            &crate::SendExpiration::Exact {
                unix_timestamp: request.valid_until,
            },
            provider_time,
            config.send_validity_seconds,
        )
        .map_err(|error| self.send_failed_error(generation, error.to_string()))?;

        workflow
            .prepared_account_loaded(FreshSendAccount {
                status: account.status,
                seqno,
            })
            .map_err(|error| self.send_workflow_error(generation, &error))?;
        self.publish_send_workflow(generation, &workflow)?;

        let prepared =
            prepare_signed_boc(&config.record_id, &expected_source, &request).map_err(|error| {
                self.send_failed_error(generation, format!("invalid prepared BOC: {error}"))
            })?;

        self.submit_prepared_send(generation, &config, submit_request_id, workflow, prepared)
            .await
    }
}
