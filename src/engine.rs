use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::future::join3;
use serde_json::Value;
use url::Url;

use crate::provider::{
    ActivityPage, activity_item_order, decimal_cmp, parse_account, parse_activity, parse_rate,
    response_error, sanitize_diagnostic,
};
use crate::send::{FreshSendAccountV3, SendDirectiveV3, SendWorkflowV3};
use crate::signer::{derive_source, prepare_transfer, same_address};
use crate::*;

const PAGE_SIZE: u32 = 10;
const MAX_RESPONSE_BODY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESPONSE_HEADER_BYTES: u64 = 64 * 1024;

#[uniffi::export(foreign)]
#[async_trait]
pub trait WalletHttpHostV3: Send + Sync {
    async fn execute_http(&self, call: HttpCall) -> Result<HttpResponse, HttpHostError>;
    async fn cancel_http(&self, call_id: HttpCallIdV3);
}

#[uniffi::export(foreign)]
#[async_trait]
pub trait WalletPlatformHostV3: Send + Sync {
    async fn now(&self) -> u64;
    async fn read_protected_secret(
        &self,
        request: ProtectedSecretReadV3,
    ) -> Result<Vec<u8>, ProtectedSecretHostErrorV3>;
    async fn store_protected_secret(
        &self,
        request: ProtectedSecretStoreV3,
    ) -> Result<(), ProtectedSecretHostErrorV3>;
    async fn delete_protected_secret(
        &self,
        secret_ref: ProtectedSecretRefV3,
    ) -> Result<(), ProtectedSecretHostErrorV3>;
    async fn load_journal(
        &self,
        key: JournalKeyV3,
    ) -> Result<Option<JournalRecordV3>, JournalHostErrorV3>;
    async fn compare_exchange_journal(
        &self,
        mutation: JournalCompareExchangeV3,
    ) -> Result<JournalCompareExchangeResultV3, JournalHostErrorV3>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OperationFamily {
    Refresh,
    Pagination,
    Send,
}

struct State {
    config: WalletClientConfigV3,
    snapshot: WalletSnapshotV3,
    next_id: u64,
    refresh_generation: u64,
    pagination_generation: u64,
    send_generation: u64,
    active_refresh: Option<(u64, Vec<HttpCallIdV3>)>,
    active_pagination: Option<(u64, HttpCallIdV3)>,
    active_send: Option<(u64, Vec<HttpCallIdV3>)>,
    send_commit_started: bool,
    send_workflow: Option<SendWorkflowV3>,
    waiters: Vec<(u64, oneshot::Sender<()>)>,
    shutdown: bool,
}

impl State {
    fn allocate_id(&mut self) -> Result<u64, WalletClientErrorV3> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(WalletClientErrorV3::IdentifierExhausted)?;
        Ok(id)
    }

    fn next_revision(&mut self) -> Result<(), WalletClientErrorV3> {
        self.snapshot.revision = self
            .snapshot
            .revision
            .checked_add(1)
            .ok_or(WalletClientErrorV3::IdentifierExhausted)?;
        let revision = self.snapshot.revision;
        let mut pending = Vec::new();
        for (after, waiter) in self.waiters.drain(..) {
            if revision > after {
                let _ = waiter.send(());
            } else {
                pending.push((after, waiter));
            }
        }
        self.waiters = pending;
        Ok(())
    }

    fn is_current(&self, family: OperationFamily, generation: u64) -> bool {
        match family {
            OperationFamily::Refresh => self
                .active_refresh
                .as_ref()
                .is_some_and(|active| active.0 == generation),
            OperationFamily::Pagination => self
                .active_pagination
                .is_some_and(|active| active.0 == generation),
            OperationFamily::Send => self
                .active_send
                .as_ref()
                .is_some_and(|active| active.0 == generation),
        }
    }
}

#[derive(uniffi::Object)]
pub struct WalletClientV3 {
    http_host: Arc<dyn WalletHttpHostV3>,
    platform_host: Arc<dyn WalletPlatformHostV3>,
    state: Mutex<State>,
}

#[uniffi::export]
impl WalletClientV3 {
    #[uniffi::constructor]
    pub fn new(
        config: WalletClientConfigV3,
        http_host: Arc<dyn WalletHttpHostV3>,
        platform_host: Arc<dyn WalletPlatformHostV3>,
    ) -> Result<Arc<Self>, WalletClientErrorV3> {
        validate_config(&config)?;
        let snapshot = WalletSnapshotV3::empty(&config);
        Ok(Arc::new(Self {
            http_host,
            platform_host,
            state: Mutex::new(State {
                config,
                snapshot,
                next_id: 1,
                refresh_generation: 0,
                pagination_generation: 0,
                send_generation: 0,
                active_refresh: None,
                active_pagination: None,
                active_send: None,
                send_commit_started: false,
                send_workflow: None,
                waiters: Vec::new(),
                shutdown: false,
            }),
        }))
    }

    pub fn snapshot(&self) -> Result<WalletSnapshotV3, WalletClientErrorV3> {
        Ok(self.lock()?.snapshot.clone())
    }

    pub async fn wait_for_change(
        &self,
        after_revision: u64,
    ) -> Result<WalletSnapshotV3, WalletClientErrorV3> {
        let receiver = {
            let mut state = self.lock()?;
            if state.shutdown {
                return Err(WalletClientErrorV3::Shutdown);
            }
            if state.snapshot.revision > after_revision {
                return Ok(state.snapshot.clone());
            }
            let (sender, receiver) = oneshot::channel();
            state.waiters.push((after_revision, sender));
            receiver
        };
        receiver.await.map_err(|_| WalletClientErrorV3::Shutdown)?;
        let state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientErrorV3::Shutdown);
        }
        Ok(state.snapshot.clone())
    }

    pub async fn refresh(&self) -> Result<WalletUpdateV3, WalletClientErrorV3> {
        let (generation, calls, previous_calls) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            let config = state.config.clone();
            let account_id = HttpCallIdV3 {
                value: state.allocate_id()?,
            };
            let activity_id = HttpCallIdV3 {
                value: state.allocate_id()?,
            };
            let rate_id = HttpCallIdV3 {
                value: state.allocate_id()?,
            };
            let calls = refresh_calls(&config, account_id, activity_id, rate_id)?;
            state.refresh_generation = state
                .refresh_generation
                .checked_add(1)
                .ok_or(WalletClientErrorV3::IdentifierExhausted)?;
            let generation = state.refresh_generation;
            let mut previous_calls = state
                .active_refresh
                .replace((generation, vec![account_id, activity_id, rate_id]))
                .map(|active| active.1)
                .unwrap_or_default();
            if let Some((_, page_call)) = state.active_pagination.take() {
                previous_calls.push(page_call);
            }
            state.snapshot.account_resource = ResourceStateV3::loading();
            state.snapshot.activity_resource = ResourceStateV3::loading();
            state.snapshot.activity_pagination_resource = ResourceStateV3::idle();
            state.snapshot.rate_resource = ResourceStateV3::loading();
            state.next_revision()?;
            (generation, calls, previous_calls)
        };
        for call_id in previous_calls {
            self.http_host.cancel_http(call_id).await;
        }

        let (account, activity, rate) = join3(
            self.http_host.execute_http(calls.0.clone()),
            self.http_host.execute_http(calls.1.clone()),
            self.http_host.execute_http(calls.2.clone()),
        )
        .await;

        let account = evaluate_for_call(&calls.0, account, parse_account);
        self.publish_refresh_component(generation, RefreshValue::Account(account))?;
        let activity =
            evaluate_for_call(&calls.1, activity, |body| parse_activity(body, PAGE_SIZE));
        self.publish_refresh_component(generation, RefreshValue::Activity(activity))?;
        let rate = evaluate_for_call(&calls.2, rate, parse_rate);
        self.publish_refresh_component(generation, RefreshValue::Rate(rate))?;

        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Refresh, generation) {
            return Ok(update(WalletOperationOutcomeV3::Superseded, 0, &state));
        }
        state.active_refresh = None;
        let failed = [
            &state.snapshot.account_resource,
            &state.snapshot.activity_resource,
            &state.snapshot.rate_resource,
        ]
        .into_iter()
        .filter(|resource| resource.phase == ResourcePhaseV3::Failed)
        .count();
        let outcome = match failed {
            0 => WalletOperationOutcomeV3::Completed,
            3 => WalletOperationOutcomeV3::Failed,
            _ => WalletOperationOutcomeV3::PartiallyCompleted,
        };
        Ok(update(outcome, 0, &state))
    }

    pub async fn cancel_refresh(&self) -> Result<(), WalletClientErrorV3> {
        let calls = {
            let mut state = self.lock()?;
            let calls = state
                .active_refresh
                .take()
                .map(|active| active.1)
                .unwrap_or_default();
            if !calls.is_empty() {
                mark_loading_cancelled(&mut state.snapshot.account_resource);
                mark_loading_cancelled(&mut state.snapshot.activity_resource);
                mark_loading_cancelled(&mut state.snapshot.rate_resource);
                state.next_revision()?;
            }
            calls
        };
        for call_id in calls {
            self.http_host.cancel_http(call_id).await;
        }
        Ok(())
    }

    pub async fn load_more_activity(&self) -> Result<WalletUpdateV3, WalletClientErrorV3> {
        let (generation, call) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            if state.active_refresh.is_some()
                || state.active_pagination.is_some()
                || !state.snapshot.activity_has_more
            {
                return Ok(update(WalletOperationOutcomeV3::Skipped, 0, &state));
            }
            let Some(cursor) = state.snapshot.activity_cursor.clone() else {
                return Ok(update(WalletOperationOutcomeV3::Skipped, 0, &state));
            };
            let id = HttpCallIdV3 {
                value: state.allocate_id()?,
            };
            let call = activity_page_call(&state.config, &cursor, id)?;
            state.pagination_generation = state
                .pagination_generation
                .checked_add(1)
                .ok_or(WalletClientErrorV3::IdentifierExhausted)?;
            let generation = state.pagination_generation;
            state.active_pagination = Some((generation, id));
            state.snapshot.activity_pagination_resource = ResourceStateV3::loading();
            state.next_revision()?;
            (generation, call)
        };

        let result = evaluate_for_call(
            &call,
            self.http_host.execute_http(call.clone()).await,
            |body| parse_activity(body, PAGE_SIZE),
        );
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Pagination, generation) {
            return Ok(update(WalletOperationOutcomeV3::Superseded, 0, &state));
        }
        state.active_pagination = None;
        let (outcome, added) = match result {
            Ok(page) => {
                let added = apply_activity_page(&mut state.snapshot, page);
                state.snapshot.activity_pagination_resource = ResourceStateV3::ready();
                (WalletOperationOutcomeV3::Completed, added)
            }
            Err(error) if error.code == ErrorCodeV3::HostCancelled => {
                state.snapshot.activity_pagination_resource = ResourceStateV3::idle();
                (WalletOperationOutcomeV3::Cancelled, 0)
            }
            Err(error) => {
                state.snapshot.activity_pagination_resource = ResourceStateV3::failed(error);
                (WalletOperationOutcomeV3::Failed, 0)
            }
        };
        state.next_revision()?;
        Ok(update(outcome, added, &state))
    }

    pub async fn cancel_load_more_activity(&self) -> Result<(), WalletClientErrorV3> {
        let call = {
            let mut state = self.lock()?;
            let call = state.active_pagination.take().map(|active| active.1);
            if call.is_some() {
                state.snapshot.activity_pagination_resource = ResourceStateV3::idle();
                state.next_revision()?;
            }
            call
        };
        if let Some(call_id) = call {
            self.http_host.cancel_http(call_id).await;
        }
        Ok(())
    }

    /// Fetches fresh wallet state, authorizes protected-secret access, signs a
    /// V5R1 transfer inside Rust, durably records the exact BOC, and submits
    /// that BOC. Streaming confirmation is deliberately outside V3.
    pub async fn send(&self, request: SendRequestV3) -> Result<SendResultV3, WalletClientErrorV3> {
        validate_send(&request)?;
        let (
            generation,
            config,
            account_call,
            seqno_call,
            submit_call_id,
            journal_key,
            mut workflow,
        ) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            if state.active_send.is_some() {
                return Err(WalletClientErrorV3::StateUnavailable);
            }
            state.send_generation = state
                .send_generation
                .checked_add(1)
                .ok_or(WalletClientErrorV3::IdentifierExhausted)?;
            let generation = state.send_generation;
            let config = state.config.clone();
            let account_call = toncenter_call(
                &config,
                HttpCallIdV3 {
                    value: state.allocate_id()?,
                },
                "getAddressInformation",
                &[("address", config.address.as_str())],
            )?;
            let seqno_call = seqno_call(
                &config,
                HttpCallIdV3 {
                    value: state.allocate_id()?,
                },
            )?;
            let submit_call_id = HttpCallIdV3 {
                value: state.allocate_id()?,
            };
            let mut workflow = SendWorkflowV3::new(
                config.wallet_id.clone(),
                config.address.clone(),
                request.clone(),
            );
            let directive = workflow
                .begin()
                .map_err(|_| WalletClientErrorV3::InvalidConfig)?;
            let SendDirectiveV3::LoadJournal(journal_key) = directive else {
                return Err(WalletClientErrorV3::StateUnavailable);
            };
            state.active_send = Some((generation, Vec::new()));
            state.send_commit_started = false;
            state.snapshot.send = workflow.snapshot();
            state.send_workflow = Some(workflow.clone());
            state.next_revision()?;
            (
                generation,
                config,
                account_call,
                seqno_call,
                submit_call_id,
                journal_key,
                workflow,
            )
        };

        let journal_record =
            self.platform_host
                .load_journal(journal_key)
                .await
                .map_err(|error| {
                    let _ = self.fail_send(generation, error.to_string());
                    WalletClientErrorV3::StateUnavailable
                })?;
        self.ensure_current_send(generation)?;
        let directive = workflow.journal_loaded(journal_record).map_err(|error| {
            let _ = self.fail_send(generation, error.to_string());
            WalletClientErrorV3::StateUnavailable
        })?;
        let SendDirectiveV3::FetchFreshAccount = directive else {
            self.fail_send(generation, "invalid send journal transition".to_owned())?;
            return Err(WalletClientErrorV3::StateUnavailable);
        };
        self.publish_send_workflow(generation, &workflow)?;

        self.start_send_http_call(generation, account_call.id)?;
        let account_response = self.http_host.execute_http(account_call.clone()).await;
        self.finish_send_http_call(generation, account_call.id)?;
        let account =
            evaluate_for_call(&account_call, account_response, parse_account).map_err(|error| {
                let _ = self.fail_send(generation, error.developer_message);
                WalletClientErrorV3::StateUnavailable
            })?;
        let seqno = if account.status == AccountStatusV3::Active {
            self.start_send_http_call(generation, seqno_call.id)?;
            let seqno_response = self.http_host.execute_http(seqno_call.clone()).await;
            self.finish_send_http_call(generation, seqno_call.id)?;
            evaluate_for_call(&seqno_call, seqno_response, parse_seqno).map_err(|error| {
                let _ = self.fail_send(generation, error.developer_message);
                WalletClientErrorV3::StateUnavailable
            })?
        } else {
            0
        };
        let fresh = FreshSendAccountV3 {
            status: account.status,
            seqno,
            observed_at: account.sync_utime.unwrap_or_default(),
        };
        let directive = workflow
            .fresh_account_loaded(fresh.clone())
            .map_err(|error| {
                let _ = self.fail_send(generation, error.to_string());
                WalletClientErrorV3::StateUnavailable
            })?;
        let SendDirectiveV3::ReadProtectedSecret(secret_request) = directive else {
            self.fail_send(generation, "invalid secret-read transition".to_owned())?;
            return Err(WalletClientErrorV3::StateUnavailable);
        };
        self.publish_send_workflow(generation, &workflow)?;
        let secret = SensitiveBytesV3::new(
            self.platform_host
                .read_protected_secret(secret_request)
                .await
                .map_err(|error| {
                    let _ = self.fail_send(generation, error.to_string());
                    WalletClientErrorV3::StateUnavailable
                })?,
        );
        self.ensure_current_send(generation)?;
        let source = derive_source(secret.as_slice(), config.network).map_err(|_| {
            let _ = self.fail_send(generation, "protected mnemonic is invalid".to_owned());
            WalletClientErrorV3::InvalidConfig
        })?;
        let source_matches = match same_address(&source, &config.address) {
            Ok(matches) => matches,
            Err(error) => {
                self.fail_send(generation, error)?;
                return Err(WalletClientErrorV3::InvalidConfig);
            }
        };
        if !source_matches {
            self.fail_send(
                generation,
                "protected mnemonic does not belong to this wallet".to_owned(),
            )?;
            return Err(WalletClientErrorV3::InvalidConfig);
        }
        let SendDirectiveV3::PrepareTransfer { .. } =
            workflow.authorization_succeeded().map_err(|error| {
                let _ = self.fail_send(generation, error.to_string());
                WalletClientErrorV3::StateUnavailable
            })?
        else {
            self.fail_send(
                generation,
                "invalid send authorization transition".to_owned(),
            )?;
            return Err(WalletClientErrorV3::StateUnavailable);
        };
        self.publish_send_workflow(generation, &workflow)?;
        let now = self.platform_host.now().await;
        self.ensure_current_send(generation)?;
        let Some(valid_until) = now.checked_add(300) else {
            self.fail_send(generation, "transfer expiry overflow".to_owned())?;
            return Err(WalletClientErrorV3::IdentifierExhausted);
        };
        let prepared = prepare_transfer(
            secret.as_slice(),
            &config.wallet_id,
            &source,
            config.network,
            &request,
            &fresh,
            valid_until,
        )
        .map_err(|_| {
            let _ = self.fail_send(generation, "failed to prepare transfer".to_owned());
            WalletClientErrorV3::StateUnavailable
        })?;
        let summary = prepared.public_summary();
        let submit_call =
            send_boc_call(&config, submit_call_id, &prepared.signed_boc).map_err(|_| {
                let _ = self.fail_send(generation, "failed to construct submission".to_owned());
                WalletClientErrorV3::StateUnavailable
            })?;
        let directive = workflow.transfer_prepared(prepared).map_err(|error| {
            let _ = self.fail_send(generation, error.to_string());
            WalletClientErrorV3::StateUnavailable
        })?;
        let SendDirectiveV3::PersistJournal(mutation) = directive else {
            self.fail_send(generation, "invalid send persistence transition".to_owned())?;
            return Err(WalletClientErrorV3::StateUnavailable);
        };
        self.publish_send_workflow(generation, &workflow)?;
        self.begin_send_commit(generation)?;
        let journal = match self.platform_host.compare_exchange_journal(mutation).await {
            Ok(journal) => journal,
            Err(error) => {
                self.mark_send_unknown(generation, error.to_string())?;
                return Err(WalletClientErrorV3::StateUnavailable);
            }
        };
        self.ensure_current_send(generation)?;
        let journal_applied = journal.applied;
        let directive = workflow.journal_persisted(journal).map_err(|error| {
            if journal_applied {
                let _ = self.mark_send_unknown(generation, error.to_string());
            } else {
                let _ = self.fail_send(generation, error.to_string());
            }
            WalletClientErrorV3::StateUnavailable
        })?;
        let SendDirectiveV3::Submit {
            signed_boc: _,
            message_hash,
        } = directive
        else {
            self.mark_send_unknown(generation, "invalid send submission transition".to_owned())?;
            return Err(WalletClientErrorV3::StateUnavailable);
        };
        workflow.submission_started().map_err(|error| {
            let _ = self.mark_send_unknown(generation, error.to_string());
            WalletClientErrorV3::StateUnavailable
        })?;
        self.publish_send_workflow(generation, &workflow)?;
        self.start_send_http_call(generation, submit_call.id)?;
        let submit_result = self.http_host.execute_http(submit_call.clone()).await;
        self.finish_send_http_call(generation, submit_call.id)?;
        let final_directive = match submit_result {
            Ok(response) => {
                match evaluate_for_call(&submit_call, Ok(response), parse_send_response) {
                    Ok(SendBocResponse::Accepted) => workflow.submission_succeeded(None),
                    Ok(SendBocResponse::Rejected(message)) => workflow.submission_rejected(message),
                    Err(error) if is_explicit_send_rejection(&error) => {
                        workflow.submission_rejected(error.developer_message)
                    }
                    Err(error) => workflow.submission_unknown(error.developer_message),
                }
            }
            Err(error) => workflow.submission_unknown(sanitize_diagnostic(&error.to_string())),
        }
        .map_err(|error| {
            let _ = self.mark_send_unknown(generation, error.to_string());
            WalletClientErrorV3::StateUnavailable
        })?;
        let SendDirectiveV3::PersistJournal(mutation) = final_directive else {
            self.mark_send_unknown(
                generation,
                "invalid terminal persistence transition".to_owned(),
            )?;
            return Err(WalletClientErrorV3::StateUnavailable);
        };
        self.publish_send_workflow(generation, &workflow)?;
        self.ensure_current_send(generation)?;
        let journal = match self.platform_host.compare_exchange_journal(mutation).await {
            Ok(journal) => journal,
            Err(error) => {
                self.mark_send_unknown(generation, error.to_string())?;
                return Err(WalletClientErrorV3::StateUnavailable);
            }
        };
        self.ensure_current_send(generation)?;
        workflow.journal_persisted(journal).map_err(|error| {
            let _ = self.mark_send_unknown(generation, error.to_string());
            WalletClientErrorV3::StateUnavailable
        })?;
        let phase = workflow.snapshot().phase;
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
        Ok(SendResultV3 {
            operation_id: request.operation_id,
            message_hash,
            phase,
        })
    }

    pub async fn cancel_send(&self) -> Result<(), WalletClientErrorV3> {
        let calls = {
            let mut state = self.lock()?;
            if state.active_send.is_some() && state.send_commit_started {
                return Err(WalletClientErrorV3::SendCancellationTooLate);
            }
            let active = state.active_send.take();
            state.send_commit_started = false;
            if active.is_some() {
                if let Some(mut workflow) = state.send_workflow.take() {
                    let _ = workflow.cancel();
                    state.snapshot.send = workflow.snapshot();
                    state.send_workflow = Some(workflow);
                }
                state.snapshot.send.phase = SendPhaseV3::Cancelled;
                state.next_revision()?;
            }
            active.map(|active| active.1).unwrap_or_default()
        };
        for call in calls {
            self.http_host.cancel_http(call).await;
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), WalletClientErrorV3> {
        let (calls, waiters) = {
            let mut state = self.lock()?;
            if state.shutdown {
                return Ok(());
            }
            if state.active_send.is_some() && state.send_commit_started {
                return Err(WalletClientErrorV3::SendCancellationTooLate);
            }
            state.shutdown = true;
            let mut calls = state
                .active_refresh
                .take()
                .map(|active| active.1)
                .unwrap_or_default();
            if let Some((_, call)) = state.active_pagination.take() {
                calls.push(call);
            }
            if let Some((_, send_calls)) = state.active_send.take() {
                calls.extend(send_calls);
            }
            state.send_commit_started = false;
            (calls, std::mem::take(&mut state.waiters))
        };
        for call in calls {
            self.http_host.cancel_http(call).await;
        }
        drop(waiters);
        Ok(())
    }
}

impl WalletClientV3 {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>, WalletClientErrorV3> {
        self.state
            .lock()
            .map_err(|_| WalletClientErrorV3::StateUnavailable)
    }

    fn publish_refresh_component(
        &self,
        generation: u64,
        value: RefreshValue,
    ) -> Result<(), WalletClientErrorV3> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Refresh, generation) {
            return Ok(());
        }
        match value {
            RefreshValue::Account(result) => match result {
                Ok(account) => {
                    state.snapshot.account = Some(account);
                    state.snapshot.account_resource = ResourceStateV3::ready();
                }
                Err(error) => state.snapshot.account_resource = ResourceStateV3::failed(error),
            },
            RefreshValue::Activity(result) => match result {
                Ok(page) => {
                    state.snapshot.activity = page.items;
                    state.snapshot.activity_cursor = page.cursor;
                    state.snapshot.activity_has_more = page.has_more;
                    state.snapshot.activity_resource = ResourceStateV3::ready();
                }
                Err(error) => state.snapshot.activity_resource = ResourceStateV3::failed(error),
            },
            RefreshValue::Rate(result) => match result {
                Ok(rate) => {
                    state.snapshot.usd_per_gram = Some(rate);
                    state.snapshot.rate_resource = ResourceStateV3::ready();
                }
                Err(error) => state.snapshot.rate_resource = ResourceStateV3::failed(error),
            },
        }
        state.next_revision()?;
        Ok(())
    }

    fn fail_send(&self, generation: u64, message: String) -> Result<(), WalletClientErrorV3> {
        let mut state = self.lock()?;
        if state.is_current(OperationFamily::Send, generation) {
            state.active_send = None;
            state.send_commit_started = false;
            state.snapshot.send.phase = SendPhaseV3::Failed;
            state.snapshot.send.error_message = Some(sanitize_diagnostic(&message));
            state.next_revision()?;
        }
        Ok(())
    }

    fn mark_send_unknown(
        &self,
        generation: u64,
        message: String,
    ) -> Result<(), WalletClientErrorV3> {
        let mut state = self.lock()?;
        if state.is_current(OperationFamily::Send, generation) {
            state.active_send = None;
            state.send_commit_started = false;
            state.snapshot.send.phase = SendPhaseV3::SubmissionUnknown;
            state.snapshot.send.error_message = Some(sanitize_diagnostic(&message));
            state.next_revision()?;
        }
        Ok(())
    }

    fn begin_send_commit(&self, generation: u64) -> Result<(), WalletClientErrorV3> {
        let mut state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientErrorV3::Shutdown);
        }
        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientErrorV3::StateUnavailable);
        }
        state.send_commit_started = true;
        Ok(())
    }

    fn ensure_current_send(&self, generation: u64) -> Result<(), WalletClientErrorV3> {
        let state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientErrorV3::Shutdown);
        }
        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientErrorV3::StateUnavailable);
        }
        Ok(())
    }

    fn publish_send_workflow(
        &self,
        generation: u64,
        workflow: &SendWorkflowV3,
    ) -> Result<(), WalletClientErrorV3> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientErrorV3::StateUnavailable);
        }
        state.snapshot.send = workflow.snapshot();
        state.send_workflow = Some(workflow.clone());
        state.next_revision()?;
        Ok(())
    }

    fn start_send_http_call(
        &self,
        generation: u64,
        call_id: HttpCallIdV3,
    ) -> Result<(), WalletClientErrorV3> {
        let mut state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientErrorV3::Shutdown);
        }
        let Some((active_generation, calls)) = state.active_send.as_mut() else {
            return Err(WalletClientErrorV3::StateUnavailable);
        };
        if *active_generation != generation {
            return Err(WalletClientErrorV3::StateUnavailable);
        }
        calls.push(call_id);
        Ok(())
    }

    fn finish_send_http_call(
        &self,
        generation: u64,
        call_id: HttpCallIdV3,
    ) -> Result<(), WalletClientErrorV3> {
        let mut state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientErrorV3::Shutdown);
        }
        let Some((active_generation, calls)) = state.active_send.as_mut() else {
            return Err(WalletClientErrorV3::StateUnavailable);
        };
        if *active_generation != generation {
            return Err(WalletClientErrorV3::StateUnavailable);
        }
        calls.retain(|active| *active != call_id);
        Ok(())
    }
}

enum RefreshValue {
    Account(Result<AccountSnapshotV3, DomainErrorV3>),
    Activity(Result<ActivityPage, DomainErrorV3>),
    Rate(Result<f64, DomainErrorV3>),
}

fn ensure_running(state: &State) -> Result<(), WalletClientErrorV3> {
    if state.shutdown {
        Err(WalletClientErrorV3::Shutdown)
    } else {
        Ok(())
    }
}

struct SensitiveBytesV3(Vec<u8>);

impl SensitiveBytesV3 {
    fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for SensitiveBytesV3 {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

fn update(outcome: WalletOperationOutcomeV3, added: u64, state: &State) -> WalletUpdateV3 {
    WalletUpdateV3 {
        outcome,
        activity_items_added: added,
        snapshot: state.snapshot.clone(),
    }
}

fn evaluate<T>(
    result: Result<HttpResponse, HttpHostError>,
    parse: impl FnOnce(&[u8]) -> Result<T, DomainErrorV3>,
) -> Result<T, DomainErrorV3> {
    let response = match result {
        Ok(response) => response,
        Err(HttpHostError::Failed { kind, message }) => return Err(host_error(kind, &message)),
    };
    if let Some(error) = response_error(response.status, &response.headers, &response.body) {
        return Err(error);
    }
    parse(&response.body)
}

fn evaluate_for_call<T>(
    call: &HttpCall,
    result: Result<HttpResponse, HttpHostError>,
    parse: impl FnOnce(&[u8]) -> Result<T, DomainErrorV3>,
) -> Result<T, DomainErrorV3> {
    if let Ok(response) = &result
        && response.final_url != call.url
    {
        return Err(host_error(
            HttpHostErrorKind::PolicyViolation,
            "HTTP redirect or mismatched final URL",
        ));
    }
    if let Ok(response) = &result {
        if response.body.len() as u64 > call.max_response_body_bytes {
            return Err(host_error(
                HttpHostErrorKind::ResponseTooLarge,
                "HTTP response exceeded the requested limit",
            ));
        }
        let header_bytes = response.headers.iter().fold(0_u64, |size, header| {
            size.saturating_add((header.name.len() + header.value.len()) as u64)
        });
        if header_bytes > call.max_response_header_bytes {
            return Err(host_error(
                HttpHostErrorKind::ResponseTooLarge,
                "HTTP response headers exceeded the requested limit",
            ));
        }
    }
    evaluate(result, parse)
}

fn host_error(kind: HttpHostErrorKind, message: &str) -> DomainErrorV3 {
    let cancelled = kind == HttpHostErrorKind::Cancelled;
    let policy = kind == HttpHostErrorKind::PolicyViolation;
    let too_large = kind == HttpHostErrorKind::ResponseTooLarge;
    DomainErrorV3 {
        code: if cancelled {
            ErrorCodeV3::HostCancelled
        } else if policy {
            ErrorCodeV3::HostPolicyViolation
        } else if too_large {
            ErrorCodeV3::ResponseTooLarge
        } else {
            ErrorCodeV3::TransportFailed
        },
        category: if cancelled {
            ErrorCategoryV3::Cancellation
        } else if policy || too_large {
            ErrorCategoryV3::HostPolicy
        } else {
            ErrorCategoryV3::Transport
        },
        retry: if cancelled || policy || too_large {
            RetryAdviceV3::None
        } else {
            RetryAdviceV3::Safe
        },
        developer_message: sanitize_diagnostic(message),
        provider_status: None,
        retry_after_ms: None,
        host_kind: Some(kind),
    }
}

fn mark_loading_cancelled(resource: &mut ResourceStateV3) {
    if resource.phase == ResourcePhaseV3::Loading {
        *resource = ResourceStateV3::idle();
    }
}

fn apply_activity_page(snapshot: &mut WalletSnapshotV3, page: ActivityPage) -> u64 {
    let advanced = match (snapshot.activity_cursor.as_ref(), page.cursor.as_ref()) {
        (Some(previous), Some(next)) => {
            decimal_cmp(&next.logical_time, &previous.logical_time).is_lt()
        }
        _ => false,
    };
    if !advanced {
        snapshot.activity_has_more = false;
        return 0;
    }
    let previous_len = snapshot.activity.len();
    let mut by_id: HashMap<_, _> = snapshot
        .activity
        .drain(..)
        .map(|item| (item.id.clone(), item))
        .collect();
    for item in page.items {
        by_id.insert(item.id.clone(), item);
    }
    snapshot.activity = by_id.into_values().collect();
    snapshot.activity.sort_by(activity_item_order);
    snapshot.activity_cursor = page.cursor;
    snapshot.activity_has_more = page.has_more;
    u64::try_from(snapshot.activity.len().saturating_sub(previous_len)).unwrap_or(u64::MAX)
}

fn refresh_calls(
    config: &WalletClientConfigV3,
    account_id: HttpCallIdV3,
    activity_id: HttpCallIdV3,
    rate_id: HttpCallIdV3,
) -> Result<(HttpCall, HttpCall, HttpCall), WalletClientErrorV3> {
    Ok((
        toncenter_call(
            config,
            account_id,
            "getAddressInformation",
            &[("address", config.address.as_str())],
        )?,
        toncenter_call(
            config,
            activity_id,
            "getTransactions",
            &[("address", config.address.as_str()), ("limit", "10")],
        )?,
        public_call(
            rate_id,
            &config.providers.tonapi_base_url,
            "rates",
            &[("tokens", "ton"), ("currencies", "usd")],
        )?,
    ))
}

fn activity_page_call(
    config: &WalletClientConfigV3,
    cursor: &ActivityCursorV3,
    id: HttpCallIdV3,
) -> Result<HttpCall, WalletClientErrorV3> {
    if cursor.logical_time.is_empty()
        || !cursor
            .logical_time
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        || cursor.hash.is_empty()
    {
        return Err(WalletClientErrorV3::InvalidConfig);
    }
    toncenter_call(
        config,
        id,
        "getTransactions",
        &[
            ("address", config.address.as_str()),
            ("limit", "10"),
            ("lt", cursor.logical_time.as_str()),
            ("hash", cursor.hash.as_str()),
        ],
    )
}

fn seqno_call(
    config: &WalletClientConfigV3,
    id: HttpCallIdV3,
) -> Result<HttpCall, WalletClientErrorV3> {
    json_rpc_call(
        config,
        id,
        "runGetMethod",
        serde_json::json!({
            "address": config.address,
            "method": "seqno",
            "stack": []
        }),
    )
}

fn send_boc_call(
    config: &WalletClientConfigV3,
    id: HttpCallIdV3,
    boc: &[u8],
) -> Result<HttpCall, WalletClientErrorV3> {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(boc);
    json_rpc_call(config, id, "sendBoc", serde_json::json!({ "boc": encoded }))
}

fn json_rpc_call(
    config: &WalletClientConfigV3,
    id: HttpCallIdV3,
    method: &str,
    params: Value,
) -> Result<HttpCall, WalletClientErrorV3> {
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.value.to_string(),
        "method": method,
        "params": params,
    }))
    .map_err(|_| WalletClientErrorV3::StateUnavailable)?;
    let mut call = toncenter_call(config, id, "jsonRPC", &[])?;
    call.method = HttpMethodV3::Post;
    call.headers.push(HttpHeaderV3 {
        name: "Content-Type".to_owned(),
        value: "application/json".to_owned(),
    });
    call.body = body;
    Ok(call)
}

fn parse_seqno(body: &[u8]) -> Result<u32, DomainErrorV3> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| invalid_json(error.to_string()))?;
    if let Some(error) = value.get("error") {
        return Err(invalid_json(error.to_string()));
    }
    let first = value
        .pointer("/result/stack/0")
        .ok_or_else(|| invalid_json("missing seqno stack"))?;
    let encoded = first
        .as_array()
        .filter(|items| items.len() == 2 && items[0].as_str() == Some("num"))
        .and_then(|items| items[1].as_str())
        .or_else(|| {
            (first.get("type").and_then(Value::as_str) == Some("num"))
                .then(|| first.get("value").and_then(Value::as_str))
                .flatten()
        })
        .ok_or_else(|| invalid_json("invalid seqno value"))?;
    if let Some(hex) = encoded.strip_prefix("0x") {
        u32::from_str_radix(hex, 16).map_err(|error| invalid_json(error.to_string()))
    } else {
        encoded
            .parse::<u32>()
            .map_err(|error| invalid_json(error.to_string()))
    }
}

enum SendBocResponse {
    Accepted,
    Rejected(String),
}

fn parse_send_response(body: &[u8]) -> Result<SendBocResponse, DomainErrorV3> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| invalid_json(error.to_string()))?;
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        return Ok(SendBocResponse::Rejected(json_error_message(error)));
    }
    if value.get("ok") == Some(&Value::Bool(false)) {
        return Ok(SendBocResponse::Rejected(json_error_message(&value)));
    }
    if value.pointer("/result/@type").and_then(Value::as_str) == Some("ok") {
        return Ok(SendBocResponse::Accepted);
    }
    Err(invalid_json("invalid sendBoc success response"))
}

fn json_error_message(value: &Value) -> String {
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))
        .or_else(|| value.get("description").and_then(Value::as_str))
        .or_else(|| value.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    sanitize_diagnostic(&message)
}

fn is_explicit_send_rejection(error: &DomainErrorV3) -> bool {
    error
        .provider_status
        .is_some_and(|status| matches!(status, 400 | 401 | 403 | 404 | 405 | 413 | 422 | 429))
}

fn invalid_json(message: impl Into<String>) -> DomainErrorV3 {
    DomainErrorV3 {
        code: ErrorCodeV3::InvalidProviderResponse,
        category: ErrorCategoryV3::ProviderProtocol,
        retry: RetryAdviceV3::None,
        developer_message: sanitize_diagnostic(&message.into()),
        provider_status: None,
        retry_after_ms: None,
        host_kind: None,
    }
}

fn toncenter_call(
    config: &WalletClientConfigV3,
    id: HttpCallIdV3,
    path: &str,
    query: &[(&str, &str)],
) -> Result<HttpCall, WalletClientErrorV3> {
    let mut call = public_call(id, &config.providers.toncenter_base_url, path, query)?;
    call.credential = config.providers.toncenter_credential.clone();
    call.credential_origin = config.providers.toncenter_credential_origin.clone();
    Ok(call)
}

fn public_call(
    id: HttpCallIdV3,
    base: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<HttpCall, WalletClientErrorV3> {
    Ok(HttpCall {
        id,
        method: HttpMethodV3::Get,
        url: provider_url(base, path, query)?,
        headers: vec![HttpHeaderV3 {
            name: "Accept".to_owned(),
            value: "application/json".to_owned(),
        }],
        body: Vec::new(),
        credential: None,
        credential_origin: None,
        max_response_header_bytes: MAX_RESPONSE_HEADER_BYTES,
        max_response_body_bytes: MAX_RESPONSE_BODY_BYTES,
    })
}

fn provider_url(
    base: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<String, WalletClientErrorV3> {
    let mut url = Url::parse(base).map_err(|_| WalletClientErrorV3::InvalidConfig)?;
    url.path_segments_mut()
        .map_err(|_| WalletClientErrorV3::InvalidConfig)?
        .pop_if_empty()
        .push(path);
    url.query_pairs_mut().extend_pairs(query.iter().copied());
    Ok(url.into())
}

fn validate_config(config: &WalletClientConfigV3) -> Result<(), WalletClientErrorV3> {
    if config.wallet_id.trim().is_empty() || config.address.trim().is_empty() {
        return Err(WalletClientErrorV3::InvalidConfig);
    }
    validate_https_url(&config.providers.toncenter_base_url)?;
    validate_https_url(&config.providers.tonapi_base_url)?;
    match (
        &config.providers.toncenter_credential,
        &config.providers.toncenter_credential_origin,
    ) {
        (Some(_), Some(origin)) => {
            validate_https_origin(origin)?;
            if effective_origin(&config.providers.toncenter_base_url)? != *origin {
                return Err(WalletClientErrorV3::InvalidConfig);
            }
            Ok(())
        }
        (None, None) => Ok(()),
        _ => Err(WalletClientErrorV3::InvalidConfig),
    }
}

fn effective_origin(value: &str) -> Result<String, WalletClientErrorV3> {
    let url = Url::parse(value).map_err(|_| WalletClientErrorV3::InvalidConfig)?;
    let host = url.host_str().ok_or(WalletClientErrorV3::InvalidConfig)?;
    let port = url
        .port_or_known_default()
        .ok_or(WalletClientErrorV3::InvalidConfig)?;
    Ok(format!("{}://{host}:{port}", url.scheme()))
}

fn validate_https_url(value: &str) -> Result<(), WalletClientErrorV3> {
    let url = Url::parse(value).map_err(|_| WalletClientErrorV3::InvalidConfig)?;
    if url.scheme() != "https" || url.host_str().is_none() || url.fragment().is_some() {
        return Err(WalletClientErrorV3::InvalidConfig);
    }
    Ok(())
}

fn validate_https_origin(value: &str) -> Result<(), WalletClientErrorV3> {
    let url = Url::parse(value).map_err(|_| WalletClientErrorV3::InvalidConfig)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || url.port_or_known_default() != Some(443)
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(WalletClientErrorV3::InvalidConfig);
    }
    Ok(())
}

fn validate_send(request: &SendRequestV3) -> Result<(), WalletClientErrorV3> {
    if request.operation_id.trim().is_empty()
        || request.destination.trim().is_empty()
        || request.secret_ref.value.trim().is_empty()
        || request.amount_nanograms.is_empty()
        || !request
            .amount_nanograms
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        || request.amount_nanograms.bytes().all(|byte| byte == b'0')
    {
        return Err(WalletClientErrorV3::InvalidConfig);
    }
    Ok(())
}
