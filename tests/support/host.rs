use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use ton::block_tlb::Msg;
use ton::ton_core::cell::TonCell;
use ton::ton_core::traits::tlb::TLB;
use wallet_engine::{
    HttpHeader, HttpHostError, HttpHostErrorKind, HttpRequest, HttpRequestId, HttpResponse,
    JournalCompareExchange, JournalCompareExchangeResult, JournalHostError, JournalKey,
    JournalRecord, ProtectedSecretHostError, ProtectedSecretHostErrorKind, ProtectedSecretRead,
    ProtectedSecretRef, ProtectedSecretStore, SecretAccessReason, WalletHttpHost,
    WalletPlatformHost,
};

use super::scenario::{SubmissionOutcome, WalletFixture};
use super::test_wallet;

const CHECKPOINT_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) struct ScenarioHttpHost {
    state: Mutex<HttpState>,
    changed: Condvar,
}

struct HttpState {
    wallet: WalletFixture,
    account_status: u16,
    activity_status: u16,
    account_retry_after_seconds: Option<u64>,
    activity_malformed: bool,
    account_redirected: bool,
    activity_pages: Vec<usize>,
    activity_page_index: usize,
    submission_gate: Option<SubmissionGate>,
    request_gate: Option<RequestGate>,
    submitted_message: Option<SubmittedMessage>,
    cancelled: HashSet<HttpRequestId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestKind {
    Account,
    Activity,
}

struct RequestGate {
    name: String,
    kind: RequestKind,
    request_id: Option<HttpRequestId>,
    reached: bool,
    released: bool,
}

#[derive(Clone)]
pub(super) struct SubmittedMessage {
    pub(super) contains_state_init: bool,
}

struct SubmissionGate {
    name: String,
    reached: bool,
    outcome: Option<SubmissionOutcome>,
}

impl ScenarioHttpHost {
    pub(super) fn new(wallet: WalletFixture, paused_submission: Option<String>) -> Self {
        Self {
            state: Mutex::new(HttpState {
                wallet,
                account_status: 200,
                activity_status: 200,
                account_retry_after_seconds: None,
                activity_malformed: false,
                account_redirected: false,
                activity_pages: Vec::new(),
                activity_page_index: 0,
                submission_gate: paused_submission.map(|name| SubmissionGate {
                    name,
                    reached: false,
                    outcome: None,
                }),
                request_gate: None,
                submitted_message: None,
                cancelled: HashSet::new(),
            }),
            changed: Condvar::new(),
        }
    }

    pub(super) fn set_wallet(&self, wallet: WalletFixture) {
        lock(&self.state).wallet = wallet;
    }

    pub(super) fn set_provider_behavior(&self, fixture: &super::scenario::ProviderFixture) {
        let mut state = lock(&self.state);
        state.account_status = fixture.account_status;
        state.activity_status = fixture.activity_status;
        state.account_retry_after_seconds = fixture.account_retry_after_seconds;
        state.activity_malformed = fixture.activity_malformed;
        state.account_redirected = fixture.account_redirected;
    }

    pub(super) fn set_activity_pages(&self, pages: Vec<usize>) {
        let mut state = lock(&self.state);
        state.activity_pages = pages;
        state.activity_page_index = 0;
    }

    pub(super) fn pause_submission(&self, name: String) {
        lock(&self.state).submission_gate = Some(SubmissionGate {
            name,
            reached: false,
            outcome: None,
        });
    }

    pub(super) fn pause_next_request(&self, name: String, kind: RequestKind) {
        lock(&self.state).request_gate = Some(RequestGate {
            name,
            kind,
            request_id: None,
            reached: false,
            released: false,
        });
    }

    pub(super) fn wait_for_request(&self, name: &str) -> Result<(), String> {
        let mut state = lock(&self.state);
        let deadline = Instant::now() + CHECKPOINT_TIMEOUT;
        loop {
            let gate = state
                .request_gate
                .as_ref()
                .ok_or_else(|| format!("request checkpoint `{name}` does not exist"))?;
            if gate.name != name {
                return Err(format!(
                    "expected request checkpoint `{}`, got `{name}`",
                    gate.name
                ));
            }
            if gate.reached {
                return Ok(());
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "request checkpoint `{name}` was not reached within {CHECKPOINT_TIMEOUT:?}"
                ));
            }
            state = match self.changed.wait_timeout(state, remaining) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    pub(super) fn release_request(&self, name: &str) -> Result<(), String> {
        self.wait_for_request(name)?;
        let mut state = lock(&self.state);
        let gate = state
            .request_gate
            .as_mut()
            .ok_or_else(|| format!("request checkpoint `{name}` disappeared"))?;
        gate.released = true;
        self.changed.notify_all();
        Ok(())
    }

    pub(super) fn request_was_cancelled(&self, name: &str) -> Result<bool, String> {
        let state = lock(&self.state);
        let gate = state
            .request_gate
            .as_ref()
            .ok_or_else(|| format!("request checkpoint `{name}` does not exist"))?;
        if gate.name != name {
            return Err(format!(
                "expected request checkpoint `{}`, got `{name}`",
                gate.name
            ));
        }
        let request_id = gate
            .request_id
            .ok_or_else(|| format!("request checkpoint `{name}` was not reached"))?;
        Ok(state.cancelled.contains(&request_id))
    }

    fn wait_at_request_gate(
        &self,
        kind: RequestKind,
        request_id: HttpRequestId,
    ) -> Result<(), HttpHostError> {
        let mut state = lock(&self.state);
        let should_capture = matches!(
            state.request_gate.as_ref(),
            Some(gate) if gate.kind == kind && gate.request_id.is_none()
        );
        if should_capture {
            let gate = state.request_gate.as_mut().expect("request gate exists");
            gate.request_id = Some(request_id);
            gate.reached = true;
            self.changed.notify_all();
        }

        loop {
            let Some(gate) = state.request_gate.as_ref() else {
                return Ok(());
            };
            if gate.request_id != Some(request_id) {
                return Ok(());
            }
            // A real transport can complete after cancellation. The gate
            // deliberately ignores the cancellation tombstone so scenarios
            // can release a successful late response and verify generation guards.
            if gate.released {
                state.request_gate = None;
                return Ok(());
            }
            state = wait(&self.changed, state);
        }
    }

    pub(super) fn submitted_message(&self) -> Option<SubmittedMessage> {
        lock(&self.state).submitted_message.clone()
    }

    pub(super) fn resume_submission(
        &self,
        name: &str,
        outcome: SubmissionOutcome,
    ) -> Result<(), String> {
        let mut state = lock(&self.state);
        let deadline = Instant::now() + CHECKPOINT_TIMEOUT;
        loop {
            let gate = state
                .submission_gate
                .as_ref()
                .ok_or_else(|| format!("submission checkpoint `{name}` does not exist"))?;
            if gate.name != name {
                return Err(format!(
                    "expected submission checkpoint `{}`, got `{name}`",
                    gate.name
                ));
            }
            if gate.reached {
                break;
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "submission checkpoint `{name}` was not reached within {CHECKPOINT_TIMEOUT:?}"
                ));
            }
            state = match self.changed.wait_timeout(state, remaining) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }

        state
            .submission_gate
            .as_mut()
            .ok_or_else(|| format!("submission checkpoint `{name}` disappeared"))?
            .outcome = Some(outcome);
        self.changed.notify_all();
        Ok(())
    }

    fn account_response(&self, request: &HttpRequest) -> HttpResponse {
        let state = lock(&self.state);
        let mut response = response_with_status(
            request,
            state.account_status,
            if state.account_status == 429 {
                json!({ "ok": false, "error": "account rate limited" })
            } else {
                json!({
                    "ok": true,
                    "result": {
                        "balance": state.wallet.balance_nanograms,
                        "state": state.wallet.status,
                        "sync_utime": state.wallet.sync_utime,
                    }
                })
            },
        );
        if let Some(seconds) = state.account_retry_after_seconds {
            response.headers.push(HttpHeader {
                name: "Retry-After".to_owned(),
                value: seconds.to_string(),
            });
        }
        if state.account_redirected {
            response.final_url = format!("{}/redirected", request.url);
        }
        response
    }

    fn activity_response(&self, request: &HttpRequest) -> HttpResponse {
        let mut state = lock(&self.state);
        let page_index = state.activity_page_index;
        state.activity_page_index = state.activity_page_index.saturating_add(1);
        let count = state.activity_pages.get(page_index).copied().unwrap_or(0);
        if state.activity_malformed {
            return HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: b"not-json".to_vec(),
                final_url: request.url.clone(),
            };
        }
        response_with_status(
            request,
            state.activity_status,
            if state.activity_status == 200 {
                json!({ "ok": true, "result": activity_transactions(page_index, count) })
            } else {
                json!({ "ok": false, "error": "scripted activity failure" })
            },
        )
    }

    fn seqno_response(&self, request: &HttpRequest) -> HttpResponse {
        let state = lock(&self.state);
        response(
            request,
            json!({
                "jsonrpc": "2.0",
                "id": request.id.value.to_string(),
                "result": {
                    "stack": [["num", format!("0x{:x}", state.wallet.seqno)]]
                }
            }),
        )
    }

    fn submit_response(
        &self,
        request: &HttpRequest,
        body: &Value,
    ) -> Result<HttpResponse, HttpHostError> {
        let encoded_boc = body
            .pointer("/params/boc")
            .and_then(Value::as_str)
            .ok_or_else(|| host_error(HttpHostErrorKind::Other, "sendBoc has no BOC"))?;
        let boc = STANDARD
            .decode(encoded_boc)
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
        let cell = TonCell::from_boc(boc)
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
        let message = Msg::<TonCell>::from_cell(&cell)
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;

        let mut state = lock(&self.state);
        state.submitted_message = Some(SubmittedMessage {
            contains_state_init: message.state_init().is_some(),
        });

        let outcome = if state.submission_gate.is_some() {
            {
                let gate = state.submission_gate.as_mut().ok_or_else(|| {
                    host_error(
                        HttpHostErrorKind::Other,
                        "submission checkpoint disappeared",
                    )
                })?;
                gate.reached = true;
            }
            self.changed.notify_all();

            loop {
                if state.cancelled.contains(&request.id) {
                    return Err(host_error(
                        HttpHostErrorKind::Cancelled,
                        "request cancelled",
                    ));
                }
                if let Some(outcome) = state
                    .submission_gate
                    .as_ref()
                    .and_then(|gate| gate.outcome.clone())
                {
                    break outcome;
                }

                state = wait(&self.changed, state);
            }
        } else {
            SubmissionOutcome::Accepted
        };

        match outcome {
            SubmissionOutcome::Accepted => Ok(response(
                request,
                json!({
                    "jsonrpc": "2.0",
                    "id": request.id.value.to_string(),
                    "result": { "@type": "ok" }
                }),
            )),
            SubmissionOutcome::Rejected(diagnostic) => Ok(response(
                request,
                json!({
                    "jsonrpc": "2.0",
                    "id": request.id.value.to_string(),
                    "error": { "message": diagnostic }
                }),
            )),
            SubmissionOutcome::Timeout => Err(host_error(
                HttpHostErrorKind::Timeout,
                "submission timed out",
            )),
            SubmissionOutcome::MalformedSuccess => Ok(response(
                request,
                json!({
                    "jsonrpc": "2.0",
                    "id": request.id.value.to_string(),
                    "result": null
                }),
            )),
            SubmissionOutcome::HttpFailure { status, diagnostic } => Ok(response_with_status(
                request,
                status,
                json!({ "error": diagnostic }),
            )),
        }
    }
}

fn activity_transactions(page: usize, count: usize) -> Vec<Value> {
    let address = test_wallet().testnet_v5_address();
    (0..count)
        .map(|index| {
            let ordinal = page.saturating_mul(10).saturating_add(index);
            let lt = 10_000_u64.saturating_sub(ordinal as u64);
            json!({
                "utime": 1_800_000_000_u64.saturating_sub(ordinal as u64),
                "transaction_id": {
                    "lt": lt.to_string(),
                    "hash": STANDARD.encode([ordinal as u8; 32]),
                },
                "in_msg": { "source": address, "value": "1" },
                "out_msgs": [],
            })
        })
        .collect()
}

#[async_trait]
impl WalletHttpHost for ScenarioHttpHost {
    async fn execute_http(&self, request: HttpRequest) -> Result<HttpResponse, HttpHostError> {
        if request.url.contains("getAddressInformation") {
            let response = self.account_response(&request);
            self.wait_at_request_gate(RequestKind::Account, request.id)?;
            return Ok(response);
        }
        if request.url.contains("getTransactions") {
            let response = self.activity_response(&request);
            self.wait_at_request_gate(RequestKind::Activity, request.id)?;
            return Ok(response);
        }

        let body: Value = serde_json::from_slice(&request.body)
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
        match body.get("method").and_then(Value::as_str) {
            Some("runGetMethod") => Ok(self.seqno_response(&request)),
            Some("sendBoc") => self.submit_response(&request, &body),
            method => Err(host_error(
                HttpHostErrorKind::Other,
                format!("unexpected JSON-RPC method: {method:?}"),
            )),
        }
    }

    async fn cancel_http(&self, request_id: HttpRequestId) {
        lock(&self.state).cancelled.insert(request_id);
        self.changed.notify_all();
    }
}

#[derive(Default)]
pub(super) struct MemoryPlatformHost {
    secrets: Mutex<HashMap<String, Vec<u8>>>,
    secret_reads: Mutex<u64>,
    secret_user_presence: Mutex<HashMap<String, bool>>,
    secret_read_reasons: Mutex<Vec<(String, SecretAccessReason)>>,
    journal: Mutex<HashMap<(String, String), JournalRecord>>,
    conflict_next_journal_write: Mutex<bool>,
}

impl MemoryPlatformHost {
    pub(super) fn store_test_secret(&self, secret_ref: &ProtectedSecretRef, bytes: &[u8]) {
        lock(&self.secrets).insert(secret_ref.value.clone(), bytes.to_vec());
    }

    pub(super) fn conflict_next_journal_write(&self) {
        *lock(&self.conflict_next_journal_write) = true;
    }

    pub(super) fn secret_read_count(&self) -> u64 {
        *lock(&self.secret_reads)
    }

    pub(super) fn journal_is_empty(&self) -> bool {
        lock(&self.journal).is_empty()
    }

    pub(super) fn secret_exists(&self, secret_ref: &ProtectedSecretRef) -> bool {
        lock(&self.secrets).contains_key(&secret_ref.value)
    }

    pub(super) fn stored_secret_count(&self) -> usize {
        lock(&self.secrets).len()
    }

    pub(super) fn replace_secret(
        &self,
        target: &ProtectedSecretRef,
        source: &ProtectedSecretRef,
    ) -> Result<(), String> {
        let mut secrets = lock(&self.secrets);
        let bytes = secrets
            .get(&source.value)
            .cloned()
            .ok_or_else(|| format!("source secret `{}` does not exist", source.value))?;
        secrets.insert(target.value.clone(), bytes);
        Ok(())
    }

    pub(super) fn secret_requires_user_presence(
        &self,
        secret_ref: &ProtectedSecretRef,
    ) -> Option<bool> {
        lock(&self.secret_user_presence)
            .get(&secret_ref.value)
            .copied()
    }

    pub(super) fn secret_was_read_for(
        &self,
        secret_ref: &ProtectedSecretRef,
        reason: SecretAccessReason,
    ) -> bool {
        lock(&self.secret_read_reasons)
            .iter()
            .any(|entry| entry == &(secret_ref.value.clone(), reason))
    }
}

#[async_trait]
impl WalletPlatformHost for MemoryPlatformHost {
    async fn read_protected_secret(
        &self,
        request: ProtectedSecretRead,
    ) -> Result<Vec<u8>, ProtectedSecretHostError> {
        *lock(&self.secret_reads) += 1;
        lock(&self.secret_read_reasons).push((request.secret_ref.value.clone(), request.reason));
        lock(&self.secrets)
            .get(&request.secret_ref.value)
            .cloned()
            .ok_or_else(|| secret_error("protected secret does not exist"))
    }

    async fn store_protected_secret(
        &self,
        request: ProtectedSecretStore,
    ) -> Result<(), ProtectedSecretHostError> {
        let secret_ref = request.secret_ref.value;
        lock(&self.secret_user_presence).insert(secret_ref.clone(), request.require_user_presence);
        lock(&self.secrets).insert(secret_ref, request.bytes);
        Ok(())
    }

    async fn delete_protected_secret(
        &self,
        secret_ref: ProtectedSecretRef,
    ) -> Result<(), ProtectedSecretHostError> {
        lock(&self.secrets).remove(&secret_ref.value);
        lock(&self.secret_user_presence).remove(&secret_ref.value);
        Ok(())
    }

    async fn load_journal(
        &self,
        key: JournalKey,
    ) -> Result<Option<JournalRecord>, JournalHostError> {
        Ok(lock(&self.journal).get(&(key.record_id, key.slot)).cloned())
    }

    async fn compare_exchange_journal(
        &self,
        mutation: JournalCompareExchange,
    ) -> Result<JournalCompareExchangeResult, JournalHostError> {
        let key = (mutation.key.record_id, mutation.key.slot);
        let mut journal = lock(&self.journal);
        let current = journal.get(&key).cloned();
        if std::mem::take(&mut *lock(&self.conflict_next_journal_write)) {
            return Ok(JournalCompareExchangeResult {
                applied: false,
                current,
            });
        }
        let current_version = current.as_ref().map(|record| record.version);

        if current_version != mutation.expected_version {
            return Ok(JournalCompareExchangeResult {
                applied: false,
                current,
            });
        }

        journal.insert(key, mutation.replacement.clone());
        Ok(JournalCompareExchangeResult {
            applied: true,
            current: Some(mutation.replacement),
        })
    }
}

fn response(request: &HttpRequest, body: Value) -> HttpResponse {
    response_with_status(request, 200, body)
}

fn response_with_status(request: &HttpRequest, status: u16, body: Value) -> HttpResponse {
    let body = match serde_json::to_vec(&body) {
        Ok(body) => body,
        Err(error) => panic!("test response serialization failed: {error}"),
    };

    HttpResponse {
        status,
        headers: Vec::new(),
        body,
        final_url: request.url.clone(),
    }
}

fn host_error(kind: HttpHostErrorKind, diagnostic: impl Into<String>) -> HttpHostError {
    HttpHostError::Failed {
        kind,
        diagnostic: diagnostic.into(),
    }
}

fn secret_error(diagnostic: impl Into<String>) -> ProtectedSecretHostError {
    ProtectedSecretHostError::Failed {
        kind: ProtectedSecretHostErrorKind::NotFound,
        diagnostic: diagnostic.into(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn wait<'a, T>(condvar: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    match condvar.wait(guard) {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
