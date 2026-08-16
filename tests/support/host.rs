use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures::future::poll_fn;
use serde_json::{Value, json};
use ton::block_tlb::Msg;
use ton::tep::snake_data::SnakeData;
use ton::ton_core::cell::TonCell;
use ton::ton_core::traits::tlb::TLB;
use ton::ton_core::types::TonAddress;
use ton::ton_wallet::WalletV5ExtMsgBody;
use wallet_engine::{
    HttpHeader, HttpHostError, HttpHostErrorKind, HttpRequest, HttpRequestId, HttpResponse,
    JournalCompareExchange, JournalCompareExchangeResult, JournalHostError, JournalHostErrorKind,
    JournalKey, JournalRecord, ProtectedSecretHostError, ProtectedSecretHostErrorKind,
    ProtectedSecretRead, ProtectedSecretRef, ProtectedSecretStore, SecretAccessReason,
    WalletHttpHost, WalletPlatformHost,
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
    account_host_error: Option<HttpHostErrorKind>,
    activity_malformed: bool,
    activity_host_error: Option<HttpHostErrorKind>,
    account_redirected: bool,
    seqno_status: u16,
    seqno_host_error: Option<HttpHostErrorKind>,
    emulation_status: u16,
    emulation_host_error: Option<HttpHostErrorKind>,
    emulation_rejected: bool,
    message_is_executed: bool,
    message_is_pending: bool,
    pending_status: u16,
    activity_pages: Vec<usize>,
    activity_page_index: usize,
    next_activity_status: Option<u16>,
    next_activity_host_error: Option<HttpHostErrorKind>,
    submission_gate: Option<SubmissionGate>,
    request_gates: Vec<RequestGate>,
    submitted_message: Option<SubmittedMessage>,
    cancelled: HashSet<HttpRequestId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestKind {
    Account,
    Activity,
    Seqno,
    Emulation,
    ExecutedMessage,
    PendingMessage,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlatformCallKind {
    JournalLoad,
    SecretRead,
}

struct RequestGate {
    name: String,
    kind: RequestKind,
    request_id: Option<HttpRequestId>,
    reached: bool,
    released: bool,
    waker: Option<Waker>,
}

#[derive(Clone)]
pub(super) struct SubmittedMessage {
    pub(super) contains_state_init: bool,
    pub(super) send_modes: Vec<u8>,
    pub(super) comment: Option<String>,
    pub(super) message_hash: String,
}

pub(super) fn decode_submitted_comment(
    body: &WalletV5ExtMsgBody,
) -> Result<Option<String>, HttpHostError> {
    let Some(message) = body.msgs.first() else {
        return Ok(None);
    };
    let message = Msg::<TonCell>::from_cell(message)
        .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
    let mut parser = message.body.parser();
    if parser
        .data_bits_left()
        .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?
        < 32
    {
        return Ok(None);
    }
    let opcode = parser
        .read_num::<u32>(32)
        .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
    if opcode != 0 {
        return Ok(None);
    }
    let comment = SnakeData::read(&mut parser)
        .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
    String::from_utf8(comment.as_slice().to_vec())
        .map(Some)
        .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))
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
                account_host_error: None,
                activity_malformed: false,
                activity_host_error: None,
                account_redirected: false,
                seqno_status: 200,
                seqno_host_error: None,
                emulation_status: 200,
                emulation_host_error: None,
                emulation_rejected: false,
                message_is_executed: false,
                message_is_pending: false,
                pending_status: 200,
                activity_pages: Vec::new(),
                activity_page_index: 0,
                next_activity_status: None,
                next_activity_host_error: None,
                submission_gate: paused_submission.map(|name| SubmissionGate {
                    name,
                    reached: false,
                    outcome: None,
                }),
                request_gates: Vec::new(),
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
        state.account_host_error = fixture.account_host_error;
        state.activity_malformed = fixture.activity_malformed;
        state.activity_host_error = fixture.activity_host_error;
        state.account_redirected = fixture.account_redirected;
        state.seqno_status = fixture.seqno_status;
        state.seqno_host_error = fixture.seqno_host_error;
        state.emulation_status = fixture.emulation_status;
        state.emulation_host_error = fixture.emulation_host_error;
        state.emulation_rejected = fixture.emulation_rejected;
        state.message_is_executed = fixture.message_is_executed;
        state.message_is_pending = fixture.message_is_pending;
        state.pending_status = fixture.pending_status;
    }

    pub(super) fn set_activity_pages(&self, pages: Vec<usize>) {
        let mut state = lock(&self.state);
        state.activity_pages = pages;
        state.activity_page_index = 0;
    }

    pub(super) fn fail_next_activity_response(&self, status: u16) {
        lock(&self.state).next_activity_status = Some(status);
    }

    pub(super) fn cancel_next_activity_response(&self) {
        lock(&self.state).next_activity_host_error = Some(HttpHostErrorKind::Cancelled);
    }

    pub(super) fn pause_submission(&self, name: String) {
        lock(&self.state).submission_gate = Some(SubmissionGate {
            name,
            reached: false,
            outcome: None,
        });
    }

    pub(super) fn pause_next_request(&self, name: String, kind: RequestKind) {
        lock(&self.state).request_gates.push(RequestGate {
            name,
            kind,
            request_id: None,
            reached: false,
            released: false,
            waker: None,
        });
    }

    pub(super) fn wait_for_request(&self, name: &str) -> Result<(), String> {
        let mut state = lock(&self.state);
        let deadline = Instant::now() + CHECKPOINT_TIMEOUT;
        loop {
            let gate = state
                .request_gates
                .iter()
                .find(|gate| gate.name == name)
                .ok_or_else(|| format!("request checkpoint `{name}` does not exist"))?;
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
        let waker = {
            let mut state = lock(&self.state);
            let gate = state
                .request_gates
                .iter_mut()
                .find(|gate| gate.name == name)
                .ok_or_else(|| format!("request checkpoint `{name}` disappeared"))?;
            gate.released = true;
            self.changed.notify_all();
            gate.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        Ok(())
    }

    pub(super) fn request_was_cancelled(&self, name: &str) -> Result<bool, String> {
        let state = lock(&self.state);
        let gate = state
            .request_gates
            .iter()
            .find(|gate| gate.name == name)
            .ok_or_else(|| format!("request checkpoint `{name}` does not exist"))?;
        let request_id = gate
            .request_id
            .ok_or_else(|| format!("request checkpoint `{name}` was not reached"))?;
        Ok(state.cancelled.contains(&request_id))
    }

    async fn wait_at_request_gate(
        &self,
        kind: RequestKind,
        request_id: HttpRequestId,
    ) -> Result<(), HttpHostError> {
        {
            let mut state = lock(&self.state);
            if let Some(gate) = state
                .request_gates
                .iter_mut()
                .find(|gate| gate.kind == kind && gate.request_id.is_none())
            {
                gate.request_id = Some(request_id);
                gate.reached = true;
                self.changed.notify_all();
            }
        }

        poll_fn(|context| {
            let mut state = lock(&self.state);
            let Some(gate) = state
                .request_gates
                .iter_mut()
                .find(|gate| gate.request_id == Some(request_id))
            else {
                return Poll::Ready(());
            };
            // A real transport can complete after cancellation. The gate
            // deliberately ignores the cancellation tombstone so scenarios
            // can release a successful late response and verify generation guards.
            if gate.released {
                return Poll::Ready(());
            }

            gate.waker = Some(context.waker().clone());
            Poll::Pending
        })
        .await;

        Ok(())
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
        let mut account = json!({
            "balance": state.wallet.balance_nanograms,
            "state": state.wallet.status,
        });
        if let Some(sync_utime) = state.wallet.sync_utime {
            account["sync_utime"] = json!(sync_utime);
        }
        let mut response = response_with_status(
            request,
            state.account_status,
            if state.account_status == 429 {
                json!({ "ok": false, "error": "account rate limited" })
            } else {
                json!({
                    "ok": true,
                    "result": account,
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
        let status = state
            .next_activity_status
            .take()
            .unwrap_or(state.activity_status);
        response_with_status(
            request,
            status,
            if status == 200 {
                json!({ "ok": true, "result": activity_transactions(page_index, count) })
            } else {
                json!({ "ok": false, "error": "scripted activity failure" })
            },
        )
    }

    fn seqno_response(&self, request: &HttpRequest) -> HttpResponse {
        let state = lock(&self.state);
        response_with_status(
            request,
            state.seqno_status,
            if state.seqno_status == 200 {
                json!({
                    "jsonrpc": "2.0",
                    "id": request.id.value.to_string(),
                    "result": {
                        "stack": [["num", format!("0x{:x}", state.wallet.seqno)]]
                    }
                })
            } else {
                json!({ "error": "scripted seqno failure" })
            },
        )
    }

    async fn emulation_response(
        &self,
        request: &HttpRequest,
    ) -> Result<HttpResponse, HttpHostError> {
        let body: Value = serde_json::from_slice(&request.body)
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
        if body.get("ignore_chksig") != Some(&Value::Bool(true)) {
            return Err(host_error(
                HttpHostErrorKind::Other,
                "emulation must explicitly ignore only signature verification",
            ));
        }
        if body.get("with_actions") != Some(&Value::Bool(true)) {
            return Err(host_error(
                HttpHostErrorKind::Other,
                "emulation must request high-level actions",
            ));
        }
        let encoded_boc = body
            .get("boc")
            .and_then(Value::as_str)
            .ok_or_else(|| host_error(HttpHostErrorKind::Other, "emulation has no BOC"))?;
        let boc = STANDARD
            .decode(encoded_boc)
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
        let cell = TonCell::from_boc(boc)
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
        let message = Msg::<TonCell>::from_cell(&cell)
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
        let account = TonAddress::from_msg_address(message.dst())
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?
            .to_string();

        self.wait_at_request_gate(RequestKind::Emulation, request.id)
            .await?;
        let state = lock(&self.state);
        if let Some(kind) = state.emulation_host_error {
            return Err(host_error(kind, "scripted emulation transport failure"));
        }
        if state.emulation_status != 200 {
            return Ok(response_with_status(
                request,
                state.emulation_status,
                json!({ "error": "scripted emulation failure" }),
            ));
        }
        let rejected = state.emulation_rejected;
        drop(state);
        Ok(response(
            request,
            json!({
                "mc_block_seqno": 42,
                "trace": { "tx_hash": "root", "children": [] },
                "transactions": {
                    "root": {
                        "account": account,
                        "total_fees": "1000000",
                        "description": {
                            "type": "ord",
                            "aborted": rejected,
                            "compute_ph": {
                                "success": !rejected,
                                "exit_code": if rejected { 33 } else { 0 }
                            },
                            "action": {
                                "success": !rejected,
                                "result_code": if rejected { 34 } else { 0 }
                            }
                        }
                    }
                },
                "actions": [{
                    "action_id": STANDARD.encode([1_u8; 32]),
                    "type": "ton_transfer",
                    "success": !rejected,
                    "accounts": [account],
                    "transactions": [STANDARD.encode([2_u8; 32])],
                    "details": {
                        "source": account,
                        "destination": account,
                        "value": "1000000000"
                    }
                }],
                "rand_seed": "",
                "is_incomplete": false
            }),
        ))
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
        let message_hash = message
            .cell_hash_normalized()
            .map(|hash| STANDARD.encode(hash.as_slice()))
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
        let body = WalletV5ExtMsgBody::from_cell(&message.body)
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
        let comment = decode_submitted_comment(&body)?;

        let mut state = lock(&self.state);
        state.submitted_message = Some(SubmittedMessage {
            contains_state_init: message.state_init().is_some(),
            send_modes: body.msgs_modes,
            comment,
            message_hash,
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
            self.wait_at_request_gate(RequestKind::Account, request.id)
                .await?;
            let host_error_kind = lock(&self.state).account_host_error;
            if let Some(kind) = host_error_kind {
                return Err(host_error(kind, "scripted account transport failure"));
            }
            return Ok(response);
        }
        if request.url.contains("getTransactions") {
            let response = self.activity_response(&request);
            self.wait_at_request_gate(RequestKind::Activity, request.id)
                .await?;
            let host_error_kind = lock(&self.state).next_activity_host_error.take();
            if let Some(kind) = host_error_kind {
                return Err(host_error(kind, "scripted activity transport failure"));
            }
            let host_error_kind = lock(&self.state).activity_host_error;
            if let Some(kind) = host_error_kind {
                return Err(host_error(kind, "scripted activity transport failure"));
            }
            return Ok(response);
        }
        if request.url.contains("/api/emulate/v1/emulateTrace") {
            return self.emulation_response(&request).await;
        }
        if request.url.contains("/api/v3/transactionsByMessage") {
            let executed = lock(&self.state).message_is_executed;
            let transactions = if executed {
                vec![json!({
                    "hash": STANDARD.encode([9_u8; 32]),
                    "lt": "9001"
                })]
            } else {
                Vec::new()
            };
            let response = response(
                &request,
                json!({ "transactions": transactions, "address_book": {} }),
            );
            self.wait_at_request_gate(RequestKind::ExecutedMessage, request.id)
                .await?;
            return Ok(response);
        }
        if request.url.contains("/api/v3/pendingTransactions") {
            let response = {
                let state = lock(&self.state);
                if state.pending_status != 200 {
                    response_with_status(
                        &request,
                        state.pending_status,
                        json!({ "error": "scripted pending endpoint failure" }),
                    )
                } else {
                    let transactions = if state.message_is_pending {
                        state
                            .submitted_message
                            .as_ref()
                            .map(|submitted| {
                                vec![json!({
                                    "hash": STANDARD.encode([8_u8; 32]),
                                    "lt": "8001",
                                    "in_msg": { "hash_norm": submitted.message_hash }
                                })]
                            })
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    response(
                        &request,
                        json!({ "transactions": transactions, "address_book": {} }),
                    )
                }
            };
            self.wait_at_request_gate(RequestKind::PendingMessage, request.id)
                .await?;
            return Ok(response);
        }
        if request.url.contains("/api/v3/walletStates") {
            let seqno = lock(&self.state).wallet.seqno;
            return Ok(response(
                &request,
                json!({ "wallets": [{ "seqno": seqno }], "address_book": {}, "metadata": {} }),
            ));
        }

        let body: Value = serde_json::from_slice(&request.body)
            .map_err(|error| host_error(HttpHostErrorKind::Other, error.to_string()))?;
        match body.get("method").and_then(Value::as_str) {
            Some("runGetMethod") => {
                let response = self.seqno_response(&request);
                self.wait_at_request_gate(RequestKind::Seqno, request.id)
                    .await?;
                let host_error_kind = lock(&self.state).seqno_host_error;
                if let Some(kind) = host_error_kind {
                    return Err(host_error(kind, "scripted seqno transport failure"));
                }
                Ok(response)
            }
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
    conflict_next_journal_write: Mutex<Option<JournalConflict>>,
    secret_read_error: Mutex<Option<ProtectedSecretHostError>>,
    secret_store_error: Mutex<Option<ProtectedSecretHostError>>,
    secret_delete_error: Mutex<Option<ProtectedSecretHostError>>,
    journal_load_error: Mutex<Option<JournalHostError>>,
    journal_write_error_on: Mutex<Option<u64>>,
    journal_write_count: Mutex<u64>,
    platform_gate: Mutex<Option<PlatformGate>>,
    platform_changed: Condvar,
}

#[derive(Clone, Copy)]
enum JournalConflict {
    SameRecord,
    ForeignOperation,
    ForeignMessage,
}

struct PlatformGate {
    name: String,
    kind: PlatformCallKind,
    reached: bool,
    released: bool,
}

impl MemoryPlatformHost {
    pub(super) fn pause_next_platform_call(&self, name: String, kind: PlatformCallKind) {
        *lock(&self.platform_gate) = Some(PlatformGate {
            name,
            kind,
            reached: false,
            released: false,
        });
    }

    pub(super) fn wait_for_platform_call(&self, name: &str) -> Result<(), String> {
        let mut gate = lock(&self.platform_gate);
        let deadline = Instant::now() + CHECKPOINT_TIMEOUT;
        loop {
            let current = gate
                .as_ref()
                .ok_or_else(|| format!("platform checkpoint `{name}` does not exist"))?;
            if current.name != name {
                return Err(format!(
                    "expected platform checkpoint `{}`, got `{name}`",
                    current.name
                ));
            }
            if current.reached {
                return Ok(());
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "platform checkpoint `{name}` was not reached within {CHECKPOINT_TIMEOUT:?}"
                ));
            }
            gate = match self.platform_changed.wait_timeout(gate, remaining) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    pub(super) fn release_platform_call(&self, name: &str) -> Result<(), String> {
        self.wait_for_platform_call(name)?;
        let mut gate = lock(&self.platform_gate);
        let current = gate
            .as_mut()
            .ok_or_else(|| format!("platform checkpoint `{name}` disappeared"))?;
        current.released = true;
        self.platform_changed.notify_all();
        Ok(())
    }

    fn wait_at_platform_gate(&self, kind: PlatformCallKind) {
        let mut gate = lock(&self.platform_gate);
        let should_wait = matches!(gate.as_ref(), Some(current) if current.kind == kind);
        if !should_wait {
            return;
        }

        if let Some(current) = gate.as_mut() {
            current.reached = true;
        }
        self.platform_changed.notify_all();

        while gate.as_ref().is_some_and(|current| !current.released) {
            gate = wait(&self.platform_changed, gate);
        }
        *gate = None;
    }

    pub(super) fn store_test_secret(&self, secret_ref: &ProtectedSecretRef, bytes: &[u8]) {
        lock(&self.secrets).insert(secret_ref.value.clone(), bytes.to_vec());
    }

    pub(super) fn conflict_next_journal_write(&self) {
        *lock(&self.conflict_next_journal_write) = Some(JournalConflict::SameRecord);
    }

    pub(super) fn conflict_next_journal_write_with_foreign_operation(&self) {
        *lock(&self.conflict_next_journal_write) = Some(JournalConflict::ForeignOperation);
    }

    pub(super) fn conflict_next_journal_write_with_foreign_message(&self) {
        *lock(&self.conflict_next_journal_write) = Some(JournalConflict::ForeignMessage);
    }

    pub(super) fn fail_next_secret_read(&self) {
        *lock(&self.secret_read_error) = Some(ProtectedSecretHostError::Failed {
            kind: ProtectedSecretHostErrorKind::Other,
            diagnostic: "scripted protected secret failure".to_owned(),
        });
    }

    pub(super) fn fail_next_secret_store(&self) {
        *lock(&self.secret_store_error) = Some(ProtectedSecretHostError::Failed {
            kind: ProtectedSecretHostErrorKind::Other,
            diagnostic: "scripted protected secret store failure".to_owned(),
        });
    }

    pub(super) fn fail_next_secret_delete(&self) {
        *lock(&self.secret_delete_error) = Some(ProtectedSecretHostError::Failed {
            kind: ProtectedSecretHostErrorKind::Other,
            diagnostic: "scripted protected secret delete failure".to_owned(),
        });
    }

    pub(super) fn fail_next_journal_load(&self) {
        *lock(&self.journal_load_error) = Some(JournalHostError::Failed {
            kind: JournalHostErrorKind::Unavailable,
            diagnostic: "scripted journal load failure".to_owned(),
        });
    }

    pub(super) fn fail_journal_write(&self, write_number: u64) {
        *lock(&self.journal_write_error_on) = Some(write_number);
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
        self.wait_at_platform_gate(PlatformCallKind::SecretRead);
        let secret_read_error = lock(&self.secret_read_error).take();
        if let Some(error) = secret_read_error {
            return Err(error);
        }
        lock(&self.secrets)
            .get(&request.secret_ref.value)
            .cloned()
            .ok_or_else(|| secret_error("protected secret does not exist"))
    }

    async fn store_protected_secret(
        &self,
        request: ProtectedSecretStore,
    ) -> Result<(), ProtectedSecretHostError> {
        let store_error = lock(&self.secret_store_error).take();
        if let Some(error) = store_error {
            return Err(error);
        }
        let secret_ref = request.secret_ref.value;
        lock(&self.secret_user_presence).insert(secret_ref.clone(), request.require_user_presence);
        lock(&self.secrets).insert(secret_ref, request.bytes);
        Ok(())
    }

    async fn delete_protected_secret(
        &self,
        secret_ref: ProtectedSecretRef,
    ) -> Result<(), ProtectedSecretHostError> {
        let delete_error = lock(&self.secret_delete_error).take();
        if let Some(error) = delete_error {
            return Err(error);
        }
        lock(&self.secrets).remove(&secret_ref.value);
        lock(&self.secret_user_presence).remove(&secret_ref.value);
        Ok(())
    }

    async fn load_journal(
        &self,
        key: JournalKey,
    ) -> Result<Option<JournalRecord>, JournalHostError> {
        self.wait_at_platform_gate(PlatformCallKind::JournalLoad);
        let journal_load_error = lock(&self.journal_load_error).take();
        if let Some(error) = journal_load_error {
            return Err(error);
        }
        Ok(lock(&self.journal).get(&(key.record_id, key.slot)).cloned())
    }

    async fn compare_exchange_journal(
        &self,
        mutation: JournalCompareExchange,
    ) -> Result<JournalCompareExchangeResult, JournalHostError> {
        let write_number = {
            let mut count = lock(&self.journal_write_count);
            *count = count.saturating_add(1);
            *count
        };
        if lock(&self.journal_write_error_on).as_ref() == Some(&write_number) {
            return Err(JournalHostError::Failed {
                kind: JournalHostErrorKind::Unavailable,
                diagnostic: format!("scripted journal write {write_number} failure"),
            });
        }
        let key = (mutation.key.record_id, mutation.key.slot);
        let mut journal = lock(&self.journal);
        let current = journal.get(&key).cloned();
        let conflict = lock(&self.conflict_next_journal_write).take();
        if let Some(conflict) = conflict {
            let competing_identity = match conflict {
                JournalConflict::SameRecord => None,
                JournalConflict::ForeignOperation => Some((
                    "operation_id",
                    Value::String("foreign-operation".to_owned()),
                )),
                JournalConflict::ForeignMessage => {
                    Some(("message_hash", Value::String(STANDARD.encode([4_u8; 32]))))
                }
            };
            let current = current.map(|mut record| {
                if let Some((field, value)) = &competing_identity {
                    let mut payload: Value = serde_json::from_slice(&record.payload)
                        .expect("durable scenario journal must contain JSON");
                    payload[field] = value.clone();
                    record.payload = serde_json::to_vec(&payload)
                        .expect("mutated durable scenario journal must serialize");
                    record.version = record.version.saturating_add(1);
                    journal.insert(key.clone(), record.clone());
                }
                record
            });
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
