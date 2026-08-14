//! Wallet client orchestration and callback interfaces for the host application.
//!
//! This module constructs provider calls, coordinates refresh and send work,
//! and publishes immutable snapshots. It releases the state lock before every
//! awaited host callback.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::channel::oneshot;
use futures::future::join;
use serde_json::Value;
use ton::ton_core::types::TonAddress;
use url::Url;

use crate::diagnostic::bounded_diagnostic;
use crate::provider::{
    ActivityPage, ActivityPageCursor, ActivityRecord, activity_record_order, parse_account,
    parse_activity, response_error,
};
use crate::send::{FreshSendAccount, SendDirective, SendWorkflow};
use crate::signer::{derive_source, prepare_transfer};
use crate::{
    AccountSnapshot, AccountStatus, DomainError, ErrorCategory, ErrorCode, HttpCall, HttpCallId,
    HttpHeader, HttpHostError, HttpHostErrorKind, HttpMethod, HttpResponse, JournalCompareExchange,
    JournalCompareExchangeResult, JournalHostError, JournalKey, JournalRecord,
    ProtectedSecretHostError, ProtectedSecretRead, ProtectedSecretRef, ProtectedSecretStore,
    ResourcePhase, ResourceState, RetryAdvice, SendPhase, SendRequest, SendResult,
    WalletClientConfig, WalletClientError, WalletOperationOutcome, WalletSnapshot, WalletUpdate,
};

const PAGE_SIZE: u32 = 10;
const MAX_RESPONSE_BODY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESPONSE_HEADER_BYTES: u64 = 64 * 1024;

/// Executes bounded HTTP work for the engine.
///
/// The host must enforce each response limit while it reads the response.
/// It must reject redirects and return the observed URL in `final_url`.
#[uniffi::export(foreign)]
#[async_trait]
pub trait WalletHttpHost: Send + Sync {
    /// Executes one complete HTTP call and returns a bounded response.
    ///
    /// If `credential` is present, resolve it locally. Add it only when the
    /// request origin exactly equals `credential_origin`.
    async fn execute_http(&self, call: HttpCall) -> Result<HttpResponse, HttpHostError>;

    /// Requests cancellation of the call with `call_id`.
    ///
    /// This callback must be idempotent. It can run before `execute_http`
    /// registers the call, so the host must remember an early cancellation.
    async fn cancel_http(&self, call_id: HttpCallId);
}

/// Supplies time, protected storage, and durable journal storage.
///
/// Callback implementations must not call the same client operation
/// recursively. The engine does not hold its wallet-state lock during calls.
#[uniffi::export(foreign)]
#[async_trait]
pub trait WalletPlatformHost: Send + Sync {
    /// Returns the current Unix timestamp in seconds.
    async fn now(&self) -> u64;

    /// Reads protected secret bytes after the required user authorization.
    ///
    /// The host must not log the bytes. Return a classified host error when
    /// authorization fails or the user cancels the prompt.
    async fn read_protected_secret(
        &self,
        request: ProtectedSecretRead,
    ) -> Result<Vec<u8>, ProtectedSecretHostError>;

    /// Stores secret bytes under the supplied reference.
    ///
    /// The host must apply the `require_user_presence` policy to later reads.
    async fn store_protected_secret(
        &self,
        request: ProtectedSecretStore,
    ) -> Result<(), ProtectedSecretHostError>;

    /// Deletes the protected secret for `secret_ref`.
    async fn delete_protected_secret(
        &self,
        secret_ref: ProtectedSecretRef,
    ) -> Result<(), ProtectedSecretHostError>;

    /// Loads the current opaque journal record for `key`.
    async fn load_journal(
        &self,
        key: JournalKey,
    ) -> Result<Option<JournalRecord>, JournalHostError>;

    /// Atomically replaces a journal record when its version matches.
    ///
    /// The host must compare and replace in one durable transaction. It must
    /// return the current record when the expected version does not match.
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
    // The public snapshot uses decimal strings for FFI portability. Keep the
    // authoritative activity values numeric so merge and pagination logic do
    // not need to parse those strings again.
    activity: Vec<ActivityRecord>,
    activity_cursor: Option<ActivityPageCursor>,
    activity_has_more: bool,
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

    /// Publishes the internal numeric activity model through portable DTOs.
    fn sync_activity_snapshot(&mut self) {
        self.snapshot.activity = self.activity.iter().map(ActivityRecord::snapshot).collect();
        self.snapshot.activity_cursor = self
            .activity_cursor
            .as_ref()
            .map(ActivityPageCursor::snapshot);
        self.snapshot.activity_has_more = self.activity_has_more;
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
/// Coordinates state and operations for one wallet record.
///
/// The client owns no transport or platform resources. Call [`Self::shutdown`]
/// before the host releases callback objects or application services.
pub struct WalletClient {
    http_host: Arc<dyn WalletHttpHost>,
    platform_host: Arc<dyn WalletPlatformHost>,
    state: Mutex<State>,
}

#[uniffi::export]
impl WalletClient {
    #[uniffi::constructor]
    /// Creates a client after validation of identifiers, URLs, and credential origin.
    ///
    /// The initial snapshot has revision zero and idle resource states.
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
                activity: Vec::new(),
                activity_cursor: None,
                activity_has_more: false,
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

    /// Returns a clone of the current immutable snapshot.
    ///
    /// A returned snapshot never changes. Read a newer snapshot to observe a
    /// higher revision.
    pub fn snapshot(&self) -> Result<WalletSnapshot, WalletClientError> {
        Ok(self.lock()?.snapshot.clone())
    }

    /// Waits until the snapshot revision is greater than `after_revision`.
    ///
    /// This method returns immediately when a newer revision already exists.
    /// Shutdown releases all waiters and returns [`WalletClientError::Shutdown`].
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

    /// Refreshes account and first-page activity data concurrently.
    ///
    /// Each resource publishes independently. One request can succeed while
    /// the other fails, which produces [`WalletOperationOutcome::PartiallyCompleted`].
    /// A newer refresh supersedes the older refresh and cancels its host calls.
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

            let calls = build_refresh_calls(&config, account_id, activity_id)?;

            state.refresh_generation = state
                .refresh_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.refresh_generation;

            let mut previous_calls = state
                .active_refresh
                .replace((generation, vec![account_id, activity_id]))
                .map(|active| active.1)
                .unwrap_or_default();
            if let Some((_, page_call)) = state.active_pagination.take() {
                previous_calls.push(page_call);
            }

            state.snapshot.account_resource = ResourceState::loading();
            state.snapshot.activity_resource = ResourceState::loading();
            state.snapshot.activity_pagination_resource = ResourceState::idle();
            state.next_revision()?;

            (generation, calls, previous_calls)
        };

        for call_id in previous_calls {
            self.http_host.cancel_http(call_id).await;
        }

        let (account, activity) = join(
            self.http_host.execute_http(calls.0.clone()),
            self.http_host.execute_http(calls.1.clone()),
        )
        .await;

        let account = evaluate_for_call(&calls.0, account, parse_account);
        self.publish_refresh_component(generation, RefreshValue::Account(account))?;

        let activity =
            evaluate_for_call(&calls.1, activity, |body| parse_activity(body, PAGE_SIZE));
        self.publish_refresh_component(generation, RefreshValue::Activity(activity))?;

        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Refresh, generation) {
            return Ok(update(WalletOperationOutcome::Superseded, 0, &state));
        }

        state.active_refresh = None;

        let failed = [
            &state.snapshot.account_resource,
            &state.snapshot.activity_resource,
        ]
        .into_iter()
        .filter(|resource| resource.phase == ResourcePhase::Failed)
        .count();

        let outcome = match failed {
            0 => WalletOperationOutcome::Completed,
            2 => WalletOperationOutcome::Failed,
            _ => WalletOperationOutcome::PartiallyCompleted,
        };

        Ok(update(outcome, 0, &state))
    }

    /// Cancels the active refresh and requests cancellation of its HTTP calls.
    ///
    /// This method has no effect when no refresh is active.
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
                state.next_revision()?;
            }

            calls
        };

        for call_id in calls {
            self.http_host.cancel_http(call_id).await;
        }

        Ok(())
    }

    /// Loads the next older activity page and merges unique items by item ID.
    ///
    /// The method returns `Skipped` during refresh, during another page load,
    /// or when no advancing cursor exists. A page must move to an older logical
    /// time. Otherwise pagination stops and adds no items.
    pub async fn load_more_activity(&self) -> Result<WalletUpdate, WalletClientError> {
        let (generation, call) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;

            if state.active_refresh.is_some()
                || state.active_pagination.is_some()
                || !state.activity_has_more
            {
                return Ok(update(WalletOperationOutcome::Skipped, 0, &state));
            }

            let Some(cursor) = state.activity_cursor.clone() else {
                return Ok(update(WalletOperationOutcome::Skipped, 0, &state));
            };

            let id = HttpCallId {
                value: state.allocate_id()?,
            };
            let call = build_activity_page_call(&state.config, &cursor, id)?;

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
                let added = apply_activity_page(&mut state, page);
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

    /// Cancels the active activity page load.
    ///
    /// This method has no effect when no page load is active.
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
            let expected_source = config.parsed_address()?;

            let account_call = build_toncenter_call(
                &config,
                HttpCallId {
                    value: state.allocate_id()?,
                },
                "getAddressInformation",
                &[("address", config.address.as_str())],
            )?;
            let seqno_call = build_seqno_call(
                &config,
                HttpCallId {
                    value: state.allocate_id()?,
                },
            )?;
            let submit_call_id = HttpCallId {
                value: state.allocate_id()?,
            };
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
                account_call,
                seqno_call,
                submit_call_id,
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
            .execute_tracked_send_call(generation, &account_call)
            .await?;
        let account = evaluate_for_call(&account_call, account_response, parse_account)
            .map_err(|error| self.send_failed_error(generation, error.developer_message))?;
        // Active wallets require a fresh seqno for replay protection. A wallet that is not yet
        // deployed starts at seqno zero; the workflow rejects unsupported account states later.
        let seqno = if account.status == AccountStatus::Active {
            let seqno_response = self
                .execute_tracked_send_call(generation, &seqno_call)
                .await?;

            evaluate_for_call(&seqno_call, seqno_response, parse_seqno)
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

        // Use host wall time only to create a bounded validity window. Rust still owns the exact
        // signed payload so every platform produces the same V5R1 message.
        let now = self.platform_host.now().await;
        self.ensure_current_send(generation)?;

        let Some(valid_until) = now
            .checked_add(300)
            .and_then(|timestamp| u32::try_from(timestamp).ok())
        else {
            self.fail_send(generation, "transfer expiry overflow".to_owned())?;
            return Err(WalletClientError::IdentifierExhausted);
        };

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
        let submit_call = build_send_boc_call(&config, submit_call_id, &prepared.signed_boc)
            .map_err(|_| self.send_failed_error(generation, "failed to construct submission"))?;
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
            .execute_tracked_send_call(generation, &submit_call)
            .await?;

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

    /// Stops new work, cancels active host calls, and releases snapshot waiters.
    ///
    /// The operation is idempotent. It returns `SendCancellationTooLate` while
    /// a send is past its durable commit boundary. Call it again after that
    /// send reaches a terminal phase.
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
                    state.activity = page.items;
                    state.activity_cursor = page.cursor;
                    state.activity_has_more = page.has_more;
                    state.sync_activity_snapshot();
                    state.snapshot.activity_resource = ResourceState::ready();
                }
                Err(error) => state.snapshot.activity_resource = ResourceState::failed(error),
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
            state.snapshot.send.error_message = Some(bounded_diagnostic(message));
            state.next_revision()?;
        }

        Ok(())
    }

    fn send_failed_error(&self, generation: u64, message: impl Into<String>) -> WalletClientError {
        let diagnostic = bounded_diagnostic(message.into());

        match self.fail_send(generation, diagnostic.clone()) {
            Ok(()) => WalletClientError::SendFailed { diagnostic },
            Err(error) => error,
        }
    }

    fn mark_send_unknown(&self, generation: u64, message: String) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.is_current(OperationFamily::Send, generation) {
            state.active_send = None;
            state.send_commit_started = false;
            state.snapshot.send.phase = SendPhase::SubmissionUnknown;
            state.snapshot.send.error_message = Some(bounded_diagnostic(message));
            state.next_revision()?;
        }

        Ok(())
    }

    fn submission_unknown_error(
        &self,
        generation: u64,
        message: impl Into<String>,
    ) -> WalletClientError {
        let diagnostic = bounded_diagnostic(message.into());

        match self.mark_send_unknown(generation, diagnostic.clone()) {
            Ok(()) => WalletClientError::SubmissionUnknown { diagnostic },
            Err(error) => error,
        }
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

    async fn execute_tracked_send_call(
        &self,
        generation: u64,
        call: &HttpCall,
    ) -> Result<Result<HttpResponse, HttpHostError>, WalletClientError> {
        self.start_send_http_call(generation, call.id)?;

        // Do not hold the state lock while the foreign host performs I/O.
        let result = self.http_host.execute_http(call.clone()).await;

        self.finish_send_http_call(generation, call.id)?;

        Ok(result)
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
        Err(HttpHostError::Failed { kind, diagnostic }) => {
            return Err(host_error(kind, &diagnostic));
        }
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
        developer_message: bounded_diagnostic(message),
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

fn apply_activity_page(state: &mut State, page: ActivityPage) -> u64 {
    // Toncenter pages move from newer to older logical times. Comparing the
    // retained BigUint values makes this check exact for every valid LT size.
    let advanced = match (state.activity_cursor.as_ref(), page.cursor.as_ref()) {
        (Some(previous), Some(next)) => next.logical_time < previous.logical_time,
        _ => false,
    };
    if !advanced {
        state.activity_has_more = false;
        state.sync_activity_snapshot();
        return 0;
    }

    let previous_len = state.activity.len();
    let mut by_id: HashMap<_, _> = state
        .activity
        .drain(..)
        .map(|item| (item.id.clone(), item))
        .collect();

    for item in page.items {
        by_id.insert(item.id.clone(), item);
    }

    state.activity = by_id.into_values().collect();
    state.activity.sort_by(activity_record_order);
    state.activity_cursor = page.cursor;
    state.activity_has_more = page.has_more;
    state.sync_activity_snapshot();

    u64::try_from(state.activity.len().saturating_sub(previous_len)).unwrap_or(u64::MAX)
}

fn build_refresh_calls(
    config: &WalletClientConfig,
    account_id: HttpCallId,
    activity_id: HttpCallId,
) -> Result<(HttpCall, HttpCall), WalletClientError> {
    Ok((
        build_toncenter_call(
            config,
            account_id,
            "getAddressInformation",
            &[("address", config.address.as_str())],
        )?,
        build_toncenter_call(
            config,
            activity_id,
            "getTransactions",
            &[("address", config.address.as_str()), ("limit", "10")],
        )?,
    ))
}

fn build_activity_page_call(
    config: &WalletClientConfig,
    cursor: &ActivityPageCursor,
    id: HttpCallId,
) -> Result<HttpCall, WalletClientError> {
    if cursor.hash.is_empty() {
        return Err(WalletClientError::InvalidConfig);
    }

    let logical_time = cursor.logical_time.to_string();

    build_toncenter_call(
        config,
        id,
        "getTransactions",
        &[
            ("address", config.address.as_str()),
            ("limit", "10"),
            ("lt", logical_time.as_str()),
            ("hash", cursor.hash.as_str()),
        ],
    )
}

fn build_seqno_call(
    config: &WalletClientConfig,
    id: HttpCallId,
) -> Result<HttpCall, WalletClientError> {
    build_json_rpc_call(
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

fn build_send_boc_call(
    config: &WalletClientConfig,
    id: HttpCallId,
    boc: &[u8],
) -> Result<HttpCall, WalletClientError> {
    use base64::Engine as _;

    let encoded = base64::engine::general_purpose::STANDARD.encode(boc);

    build_json_rpc_call(config, id, "sendBoc", serde_json::json!({ "boc": encoded }))
}

fn build_json_rpc_call(
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

    let mut call = build_toncenter_call(config, id, "jsonRPC", &[])?;

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

    bounded_diagnostic(message)
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
        developer_message: bounded_diagnostic(message.into()),
        provider_status: None,
        retry_after_ms: None,
        host_kind: None,
    }
}

fn build_toncenter_call(
    config: &WalletClientConfig,
    id: HttpCallId,
    path: &str,
    query: &[(&str, &str)],
) -> Result<HttpCall, WalletClientError> {
    let mut call = build_public_call(id, &config.providers.toncenter_base_url, path, query)?;

    call.credential
        .clone_from(&config.providers.toncenter_credential);
    call.credential_origin
        .clone_from(&config.providers.toncenter_credential_origin);

    Ok(call)
}

fn build_public_call(
    id: HttpCallId,
    base: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<HttpCall, WalletClientError> {
    Ok(HttpCall {
        id,
        method: HttpMethod::Get,
        url: build_provider_url(base, path, query)?,
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

fn build_provider_url(
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
    if config.record_id.trim().is_empty() {
        return Err(WalletClientError::InvalidConfig);
    }

    config.parsed_address()?;
    validate_https_url(&config.providers.toncenter_base_url)?;

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

impl WalletClientConfig {
    fn parsed_address(&self) -> Result<TonAddress, WalletClientError> {
        TonAddress::from_str(&self.address).map_err(|_| WalletClientError::InvalidConfig)
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
