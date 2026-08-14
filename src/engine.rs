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
use crate::send::{FreshSendAccount, SendDirective, SendWorkflow};
use crate::signer::{derive_source, prepare_transfer, same_address};
use crate::{
    AccountSnapshot, AccountStatus, ActivityCursor, DomainError, ErrorCategory, ErrorCode,
    HttpCall, HttpCallId, HttpHeader, HttpHostError, HttpHostErrorKind, HttpMethod, HttpResponse,
    JournalCompareExchange, JournalCompareExchangeResult, JournalHostError, JournalKey,
    JournalRecord, ProtectedSecretHostError, ProtectedSecretRead, ProtectedSecretRef,
    ProtectedSecretStore, ResourcePhase, ResourceState, RetryAdvice, SendPhase, SendRequest,
    SendResult, WalletClientConfig, WalletClientError, WalletOperationOutcome, WalletSnapshot,
    WalletUpdate,
};

const PAGE_SIZE: u32 = 10;
const MAX_RESPONSE_BODY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESPONSE_HEADER_BYTES: u64 = 64 * 1024;

#[uniffi::export(foreign)]
#[async_trait]
pub trait WalletHttpHost: Send + Sync {
    async fn execute_http(&self, call: HttpCall) -> Result<HttpResponse, HttpHostError>;
    async fn cancel_http(&self, call_id: HttpCallId);
}

#[uniffi::export(foreign)]
#[async_trait]
pub trait WalletPlatformHost: Send + Sync {
    async fn now(&self) -> u64;
    async fn read_protected_secret(
        &self,
        request: ProtectedSecretRead,
    ) -> Result<Vec<u8>, ProtectedSecretHostError>;
    async fn store_protected_secret(
        &self,
        request: ProtectedSecretStore,
    ) -> Result<(), ProtectedSecretHostError>;
    async fn delete_protected_secret(
        &self,
        secret_ref: ProtectedSecretRef,
    ) -> Result<(), ProtectedSecretHostError>;
    async fn load_journal(
        &self,
        key: JournalKey,
    ) -> Result<Option<JournalRecord>, JournalHostError>;
    async fn compare_exchange_journal(
        &self,
        mutation: JournalCompareExchange,
    ) -> Result<JournalCompareExchangeResult, JournalHostError>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OperationFamily {
    Refresh,
    Pagination,
    Send,
}

struct State {
    config: WalletClientConfig,
    snapshot: WalletSnapshot,
    next_id: u64,
    refresh_generation: u64,
    pagination_generation: u64,
    send_generation: u64,
    active_refresh: Option<(u64, Vec<HttpCallId>)>,
    active_pagination: Option<(u64, HttpCallId)>,
    active_send: Option<(u64, Vec<HttpCallId>)>,
    send_commit_started: bool,
    send_workflow: Option<SendWorkflow>,
    waiters: Vec<(u64, oneshot::Sender<()>)>,
    shutdown: bool,
}

impl State {
    fn allocate_id(&mut self) -> Result<u64, WalletClientError> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(WalletClientError::IdentifierExhausted)?;
        Ok(id)
    }

    fn next_revision(&mut self) -> Result<(), WalletClientError> {
        self.snapshot.revision = self
            .snapshot
            .revision
            .checked_add(1)
            .ok_or(WalletClientError::IdentifierExhausted)?;
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
pub struct WalletClient {
    http_host: Arc<dyn WalletHttpHost>,
    platform_host: Arc<dyn WalletPlatformHost>,
    state: Mutex<State>,
}

#[uniffi::export]
impl WalletClient {
    #[uniffi::constructor]
    pub fn new(
        config: WalletClientConfig,
        http_host: Arc<dyn WalletHttpHost>,
        platform_host: Arc<dyn WalletPlatformHost>,
    ) -> Result<Arc<Self>, WalletClientError> {
        validate_config(&config)?;
        let snapshot = WalletSnapshot::empty(&config);
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

    pub fn snapshot(&self) -> Result<WalletSnapshot, WalletClientError> {
        Ok(self.lock()?.snapshot.clone())
    }

    pub async fn wait_for_change(
        &self,
        after_revision: u64,
    ) -> Result<WalletSnapshot, WalletClientError> {
        let receiver = {
            let mut state = self.lock()?;
            if state.shutdown {
                return Err(WalletClientError::Shutdown);
            }
            if state.snapshot.revision > after_revision {
                return Ok(state.snapshot.clone());
            }
            let (sender, receiver) = oneshot::channel();
            state.waiters.push((after_revision, sender));
            receiver
        };
        receiver.await.map_err(|_| WalletClientError::Shutdown)?;
        let state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientError::Shutdown);
        }
        Ok(state.snapshot.clone())
    }

    pub async fn refresh(&self) -> Result<WalletUpdate, WalletClientError> {
        let (generation, calls, previous_calls) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            let config = state.config.clone();
            let account_id = HttpCallId {
                value: state.allocate_id()?,
            };
            let activity_id = HttpCallId {
                value: state.allocate_id()?,
            };
            let rate_id = HttpCallId {
                value: state.allocate_id()?,
            };
            let calls = refresh_calls(&config, account_id, activity_id, rate_id)?;
            state.refresh_generation = state
                .refresh_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.refresh_generation;
            let mut previous_calls = state
                .active_refresh
                .replace((generation, vec![account_id, activity_id, rate_id]))
                .map(|active| active.1)
                .unwrap_or_default();
            if let Some((_, page_call)) = state.active_pagination.take() {
                previous_calls.push(page_call);
            }
            state.snapshot.account_resource = ResourceState::loading();
            state.snapshot.activity_resource = ResourceState::loading();
            state.snapshot.activity_pagination_resource = ResourceState::idle();
            state.snapshot.rate_resource = ResourceState::loading();
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
            return Ok(update(WalletOperationOutcome::Superseded, 0, &state));
        }
        state.active_refresh = None;
        let failed = [
            &state.snapshot.account_resource,
            &state.snapshot.activity_resource,
            &state.snapshot.rate_resource,
        ]
        .into_iter()
        .filter(|resource| resource.phase == ResourcePhase::Failed)
        .count();
        let outcome = match failed {
            0 => WalletOperationOutcome::Completed,
            3 => WalletOperationOutcome::Failed,
            _ => WalletOperationOutcome::PartiallyCompleted,
        };
        Ok(update(outcome, 0, &state))
    }

    pub async fn cancel_refresh(&self) -> Result<(), WalletClientError> {
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

    pub async fn load_more_activity(&self) -> Result<WalletUpdate, WalletClientError> {
        let (generation, call) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            if state.active_refresh.is_some()
                || state.active_pagination.is_some()
                || !state.snapshot.activity_has_more
            {
                return Ok(update(WalletOperationOutcome::Skipped, 0, &state));
            }
            let Some(cursor) = state.snapshot.activity_cursor.clone() else {
                return Ok(update(WalletOperationOutcome::Skipped, 0, &state));
            };
            let id = HttpCallId {
                value: state.allocate_id()?,
            };
            let call = activity_page_call(&state.config, &cursor, id)?;
            state.pagination_generation = state
                .pagination_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.pagination_generation;
            state.active_pagination = Some((generation, id));
            state.snapshot.activity_pagination_resource = ResourceState::loading();
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
            return Ok(update(WalletOperationOutcome::Superseded, 0, &state));
        }
        state.active_pagination = None;
        let (outcome, added) = match result {
            Ok(page) => {
                let added = apply_activity_page(&mut state.snapshot, page);
                state.snapshot.activity_pagination_resource = ResourceState::ready();
                (WalletOperationOutcome::Completed, added)
            }
            Err(error) if error.code == ErrorCode::HostCancelled => {
                state.snapshot.activity_pagination_resource = ResourceState::idle();
                (WalletOperationOutcome::Cancelled, 0)
            }
            Err(error) => {
                state.snapshot.activity_pagination_resource = ResourceState::failed(error);
                (WalletOperationOutcome::Failed, 0)
            }
        };
        state.next_revision()?;
        Ok(update(outcome, added, &state))
    }

    pub async fn cancel_load_more_activity(&self) -> Result<(), WalletClientError> {
        let call = {
            let mut state = self.lock()?;
            let call = state.active_pagination.take().map(|active| active.1);
            if call.is_some() {
                state.snapshot.activity_pagination_resource = ResourceState::idle();
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
    /// that BOC. Streaming confirmation is deliberately outside .
    pub async fn send(&self, request: SendRequest) -> Result<SendResult, WalletClientError> {
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
                return Err(WalletClientError::StateUnavailable);
            }
            state.send_generation = state
                .send_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.send_generation;
            let config = state.config.clone();
            let account_call = toncenter_call(
                &config,
                HttpCallId {
                    value: state.allocate_id()?,
                },
                "getAddressInformation",
                &[("address", config.address.as_str())],
            )?;
            let seqno_call = seqno_call(
                &config,
                HttpCallId {
                    value: state.allocate_id()?,
                },
            )?;
            let submit_call_id = HttpCallId {
                value: state.allocate_id()?,
            };
            let mut workflow = SendWorkflow::new(
                config.wallet_id.clone(),
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
                    WalletClientError::StateUnavailable
                })?;
        self.ensure_current_send(generation)?;
        let directive = workflow.journal_loaded(journal_record).map_err(|error| {
            let _ = self.fail_send(generation, error.to_string());
            WalletClientError::StateUnavailable
        })?;
        let SendDirective::FetchFreshAccount = directive else {
            self.fail_send(generation, "invalid send journal transition".to_owned())?;
            return Err(WalletClientError::StateUnavailable);
        };
        self.publish_send_workflow(generation, &workflow)?;

        self.start_send_http_call(generation, account_call.id)?;
        let account_response = self.http_host.execute_http(account_call.clone()).await;
        self.finish_send_http_call(generation, account_call.id)?;
        let account =
            evaluate_for_call(&account_call, account_response, parse_account).map_err(|error| {
                let _ = self.fail_send(generation, error.developer_message);
                WalletClientError::StateUnavailable
            })?;
        let seqno = if account.status == AccountStatus::Active {
            self.start_send_http_call(generation, seqno_call.id)?;
            let seqno_response = self.http_host.execute_http(seqno_call.clone()).await;
            self.finish_send_http_call(generation, seqno_call.id)?;
            evaluate_for_call(&seqno_call, seqno_response, parse_seqno).map_err(|error| {
                let _ = self.fail_send(generation, error.developer_message);
                WalletClientError::StateUnavailable
            })?
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
            .map_err(|error| {
                let _ = self.fail_send(generation, error.to_string());
                WalletClientError::StateUnavailable
            })?;
        let SendDirective::ReadProtectedSecret(secret_request) = directive else {
            self.fail_send(generation, "invalid secret-read transition".to_owned())?;
            return Err(WalletClientError::StateUnavailable);
        };
        self.publish_send_workflow(generation, &workflow)?;
        let secret = SensitiveBytes::new(
            self.platform_host
                .read_protected_secret(secret_request)
                .await
                .map_err(|error| {
                    let _ = self.fail_send(generation, error.to_string());
                    WalletClientError::StateUnavailable
                })?,
        );
        self.ensure_current_send(generation)?;
        let source = derive_source(secret.as_slice(), config.network).map_err(|_| {
            let _ = self.fail_send(generation, "protected mnemonic is invalid".to_owned());
            WalletClientError::InvalidConfig
        })?;
        let source_matches = match same_address(&source, &config.address) {
            Ok(matches) => matches,
            Err(error) => {
                self.fail_send(generation, error)?;
                return Err(WalletClientError::InvalidConfig);
            }
        };
        if !source_matches {
            self.fail_send(
                generation,
                "protected mnemonic does not belong to this wallet".to_owned(),
            )?;
            return Err(WalletClientError::InvalidConfig);
        }
        let SendDirective::PrepareTransfer { .. } =
            workflow.authorization_succeeded().map_err(|error| {
                let _ = self.fail_send(generation, error.to_string());
                WalletClientError::StateUnavailable
            })?
        else {
            self.fail_send(
                generation,
                "invalid send authorization transition".to_owned(),
            )?;
            return Err(WalletClientError::StateUnavailable);
        };
        self.publish_send_workflow(generation, &workflow)?;
        let now = self.platform_host.now().await;
        self.ensure_current_send(generation)?;
        let Some(valid_until) = now.checked_add(300) else {
            self.fail_send(generation, "transfer expiry overflow".to_owned())?;
            return Err(WalletClientError::IdentifierExhausted);
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
            WalletClientError::StateUnavailable
        })?;
        let summary = prepared.public_summary();
        let submit_call =
            send_boc_call(&config, submit_call_id, &prepared.signed_boc).map_err(|_| {
                let _ = self.fail_send(generation, "failed to construct submission".to_owned());
                WalletClientError::StateUnavailable
            })?;
        let directive = workflow.transfer_prepared(prepared).map_err(|error| {
            let _ = self.fail_send(generation, error.to_string());
            WalletClientError::StateUnavailable
        })?;
        let SendDirective::PersistJournal(mutation) = directive else {
            self.fail_send(generation, "invalid send persistence transition".to_owned())?;
            return Err(WalletClientError::StateUnavailable);
        };
        self.publish_send_workflow(generation, &workflow)?;
        self.begin_send_commit(generation)?;
        let journal = match self.platform_host.compare_exchange_journal(mutation).await {
            Ok(journal) => journal,
            Err(error) => {
                self.mark_send_unknown(generation, error.to_string())?;
                return Err(WalletClientError::StateUnavailable);
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
            WalletClientError::StateUnavailable
        })?;
        let SendDirective::Submit {
            signed_boc: _,
            message_hash,
        } = directive
        else {
            self.mark_send_unknown(generation, "invalid send submission transition".to_owned())?;
            return Err(WalletClientError::StateUnavailable);
        };
        workflow.submission_started().map_err(|error| {
            let _ = self.mark_send_unknown(generation, error.to_string());
            WalletClientError::StateUnavailable
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
            WalletClientError::StateUnavailable
        })?;
        let SendDirective::PersistJournal(mutation) = final_directive else {
            self.mark_send_unknown(
                generation,
                "invalid terminal persistence transition".to_owned(),
            )?;
            return Err(WalletClientError::StateUnavailable);
        };
        self.publish_send_workflow(generation, &workflow)?;
        self.ensure_current_send(generation)?;
        let journal = match self.platform_host.compare_exchange_journal(mutation).await {
            Ok(journal) => journal,
            Err(error) => {
                self.mark_send_unknown(generation, error.to_string())?;
                return Err(WalletClientError::StateUnavailable);
            }
        };
        self.ensure_current_send(generation)?;
        workflow.journal_persisted(journal).map_err(|error| {
            let _ = self.mark_send_unknown(generation, error.to_string());
            WalletClientError::StateUnavailable
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
        Ok(SendResult {
            operation_id: request.operation_id,
            message_hash,
            phase,
        })
    }

    pub async fn cancel_send(&self) -> Result<(), WalletClientError> {
        let calls = {
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
        for call in calls {
            self.http_host.cancel_http(call).await;
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), WalletClientError> {
        let (calls, waiters) = {
            let mut state = self.lock()?;
            if state.shutdown {
                return Ok(());
            }
            if state.active_send.is_some() && state.send_commit_started {
                return Err(WalletClientError::SendCancellationTooLate);
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

impl WalletClient {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>, WalletClientError> {
        self.state
            .lock()
            .map_err(|_| WalletClientError::StateUnavailable)
    }

    fn publish_refresh_component(
        &self,
        generation: u64,
        value: RefreshValue,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Refresh, generation) {
            return Ok(());
        }
        match value {
            RefreshValue::Account(result) => match result {
                Ok(account) => {
                    state.snapshot.account = Some(account);
                    state.snapshot.account_resource = ResourceState::ready();
                }
                Err(error) => state.snapshot.account_resource = ResourceState::failed(error),
            },
            RefreshValue::Activity(result) => match result {
                Ok(page) => {
                    state.snapshot.activity = page.items;
                    state.snapshot.activity_cursor = page.cursor;
                    state.snapshot.activity_has_more = page.has_more;
                    state.snapshot.activity_resource = ResourceState::ready();
                }
                Err(error) => state.snapshot.activity_resource = ResourceState::failed(error),
            },
            RefreshValue::Rate(result) => match result {
                Ok(rate) => {
                    state.snapshot.usd_per_gram = Some(rate);
                    state.snapshot.rate_resource = ResourceState::ready();
                }
                Err(error) => state.snapshot.rate_resource = ResourceState::failed(error),
            },
        }
        state.next_revision()?;
        Ok(())
    }

    fn fail_send(&self, generation: u64, message: String) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.is_current(OperationFamily::Send, generation) {
            state.active_send = None;
            state.send_commit_started = false;
            state.snapshot.send.phase = SendPhase::Failed;
            state.snapshot.send.error_message = Some(sanitize_diagnostic(&message));
            state.next_revision()?;
        }
        Ok(())
    }

    fn mark_send_unknown(&self, generation: u64, message: String) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.is_current(OperationFamily::Send, generation) {
            state.active_send = None;
            state.send_commit_started = false;
            state.snapshot.send.phase = SendPhase::SubmissionUnknown;
            state.snapshot.send.error_message = Some(sanitize_diagnostic(&message));
            state.next_revision()?;
        }
        Ok(())
    }

    fn begin_send_commit(&self, generation: u64) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientError::Shutdown);
        }
        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.send_commit_started = true;
        Ok(())
    }

    fn ensure_current_send(&self, generation: u64) -> Result<(), WalletClientError> {
        let state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientError::Shutdown);
        }
        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        Ok(())
    }

    fn publish_send_workflow(
        &self,
        generation: u64,
        workflow: &SendWorkflow,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.snapshot.send = workflow.snapshot();
        state.send_workflow = Some(workflow.clone());
        state.next_revision()?;
        Ok(())
    }

    fn start_send_http_call(
        &self,
        generation: u64,
        call_id: HttpCallId,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientError::Shutdown);
        }
        let Some((active_generation, calls)) = state.active_send.as_mut() else {
            return Err(WalletClientError::StateUnavailable);
        };
        if *active_generation != generation {
            return Err(WalletClientError::StateUnavailable);
        }
        calls.push(call_id);
        Ok(())
    }

    fn finish_send_http_call(
        &self,
        generation: u64,
        call_id: HttpCallId,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientError::Shutdown);
        }
        let Some((active_generation, calls)) = state.active_send.as_mut() else {
            return Err(WalletClientError::StateUnavailable);
        };
        if *active_generation != generation {
            return Err(WalletClientError::StateUnavailable);
        }
        calls.retain(|active| *active != call_id);
        Ok(())
    }
}

enum RefreshValue {
    Account(Result<AccountSnapshot, DomainError>),
    Activity(Result<ActivityPage, DomainError>),
    Rate(Result<f64, DomainError>),
}

const fn ensure_running(state: &State) -> Result<(), WalletClientError> {
    if state.shutdown {
        Err(WalletClientError::Shutdown)
    } else {
        Ok(())
    }
}

struct SensitiveBytes(Vec<u8>);

impl SensitiveBytes {
    const fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for SensitiveBytes {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

fn update(outcome: WalletOperationOutcome, added: u64, state: &State) -> WalletUpdate {
    WalletUpdate {
        outcome,
        activity_items_added: added,
        snapshot: state.snapshot.clone(),
    }
}

fn evaluate<T>(
    result: Result<HttpResponse, HttpHostError>,
    parse: impl FnOnce(&[u8]) -> Result<T, DomainError>,
) -> Result<T, DomainError> {
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
    parse: impl FnOnce(&[u8]) -> Result<T, DomainError>,
) -> Result<T, DomainError> {
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

fn host_error(kind: HttpHostErrorKind, message: &str) -> DomainError {
    let cancelled = kind == HttpHostErrorKind::Cancelled;
    let policy = kind == HttpHostErrorKind::PolicyViolation;
    let too_large = kind == HttpHostErrorKind::ResponseTooLarge;
    DomainError {
        code: if cancelled {
            ErrorCode::HostCancelled
        } else if policy {
            ErrorCode::HostPolicyViolation
        } else if too_large {
            ErrorCode::ResponseTooLarge
        } else {
            ErrorCode::TransportFailed
        },
        category: if cancelled {
            ErrorCategory::Cancellation
        } else if policy || too_large {
            ErrorCategory::HostPolicy
        } else {
            ErrorCategory::Transport
        },
        retry: if cancelled || policy || too_large {
            RetryAdvice::None
        } else {
            RetryAdvice::Safe
        },
        developer_message: sanitize_diagnostic(message),
        provider_status: None,
        retry_after_ms: None,
        host_kind: Some(kind),
    }
}

fn mark_loading_cancelled(resource: &mut ResourceState) {
    if resource.phase == ResourcePhase::Loading {
        *resource = ResourceState::idle();
    }
}

fn apply_activity_page(snapshot: &mut WalletSnapshot, page: ActivityPage) -> u64 {
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
    config: &WalletClientConfig,
    account_id: HttpCallId,
    activity_id: HttpCallId,
    rate_id: HttpCallId,
) -> Result<(HttpCall, HttpCall, HttpCall), WalletClientError> {
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
    config: &WalletClientConfig,
    cursor: &ActivityCursor,
    id: HttpCallId,
) -> Result<HttpCall, WalletClientError> {
    if cursor.logical_time.is_empty()
        || !cursor
            .logical_time
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        || cursor.hash.is_empty()
    {
        return Err(WalletClientError::InvalidConfig);
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

fn seqno_call(config: &WalletClientConfig, id: HttpCallId) -> Result<HttpCall, WalletClientError> {
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
    config: &WalletClientConfig,
    id: HttpCallId,
    boc: &[u8],
) -> Result<HttpCall, WalletClientError> {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(boc);
    json_rpc_call(config, id, "sendBoc", serde_json::json!({ "boc": encoded }))
}

fn json_rpc_call(
    config: &WalletClientConfig,
    id: HttpCallId,
    method: &str,
    params: Value,
) -> Result<HttpCall, WalletClientError> {
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.value.to_string(),
        "method": method,
        "params": params,
    }))
    .map_err(|_| WalletClientError::StateUnavailable)?;
    let mut call = toncenter_call(config, id, "jsonRPC", &[])?;
    call.method = HttpMethod::Post;
    call.headers.push(HttpHeader {
        name: "Content-Type".to_owned(),
        value: "application/json".to_owned(),
    });
    call.body = body;
    Ok(call)
}

fn parse_seqno(body: &[u8]) -> Result<u32, DomainError> {
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

fn parse_send_response(body: &[u8]) -> Result<SendBocResponse, DomainError> {
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
        .map_or_else(|| value.to_string(), str::to_owned);
    sanitize_diagnostic(&message)
}

fn is_explicit_send_rejection(error: &DomainError) -> bool {
    error
        .provider_status
        .is_some_and(|status| matches!(status, 400 | 401 | 403 | 404 | 405 | 413 | 422 | 429))
}

fn invalid_json(message: impl Into<String>) -> DomainError {
    DomainError {
        code: ErrorCode::InvalidProviderResponse,
        category: ErrorCategory::ProviderProtocol,
        retry: RetryAdvice::None,
        developer_message: sanitize_diagnostic(&message.into()),
        provider_status: None,
        retry_after_ms: None,
        host_kind: None,
    }
}

fn toncenter_call(
    config: &WalletClientConfig,
    id: HttpCallId,
    path: &str,
    query: &[(&str, &str)],
) -> Result<HttpCall, WalletClientError> {
    let mut call = public_call(id, &config.providers.toncenter_base_url, path, query)?;
    call.credential
        .clone_from(&config.providers.toncenter_credential);
    call.credential_origin
        .clone_from(&config.providers.toncenter_credential_origin);
    Ok(call)
}

fn public_call(
    id: HttpCallId,
    base: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<HttpCall, WalletClientError> {
    Ok(HttpCall {
        id,
        method: HttpMethod::Get,
        url: provider_url(base, path, query)?,
        headers: vec![HttpHeader {
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
) -> Result<String, WalletClientError> {
    let mut url = Url::parse(base).map_err(|_| WalletClientError::InvalidConfig)?;
    url.path_segments_mut()
        .map_err(|_| WalletClientError::InvalidConfig)?
        .pop_if_empty()
        .push(path);
    url.query_pairs_mut().extend_pairs(query.iter().copied());
    Ok(url.into())
}

fn validate_config(config: &WalletClientConfig) -> Result<(), WalletClientError> {
    if config.wallet_id.trim().is_empty() || config.address.trim().is_empty() {
        return Err(WalletClientError::InvalidConfig);
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
                return Err(WalletClientError::InvalidConfig);
            }
            Ok(())
        }
        (None, None) => Ok(()),
        _ => Err(WalletClientError::InvalidConfig),
    }
}

fn effective_origin(value: &str) -> Result<String, WalletClientError> {
    let url = Url::parse(value).map_err(|_| WalletClientError::InvalidConfig)?;
    let host = url.host_str().ok_or(WalletClientError::InvalidConfig)?;
    let port = url
        .port_or_known_default()
        .ok_or(WalletClientError::InvalidConfig)?;
    Ok(format!("{}://{host}:{port}", url.scheme()))
}

fn validate_https_url(value: &str) -> Result<(), WalletClientError> {
    let url = Url::parse(value).map_err(|_| WalletClientError::InvalidConfig)?;
    if url.scheme() != "https" || url.host_str().is_none() || url.fragment().is_some() {
        return Err(WalletClientError::InvalidConfig);
    }
    Ok(())
}

fn validate_https_origin(value: &str) -> Result<(), WalletClientError> {
    let url = Url::parse(value).map_err(|_| WalletClientError::InvalidConfig)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || url.port_or_known_default() != Some(443)
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(WalletClientError::InvalidConfig);
    }
    Ok(())
}

fn validate_send(request: &SendRequest) -> Result<(), WalletClientError> {
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
        return Err(WalletClientError::InvalidConfig);
    }
    Ok(())
}
