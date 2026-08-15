use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures::executor::block_on;
use ton::block_tlb::{
    SEND_MODE_CARRY_ALL_BALANCE, SEND_MODE_IGNORE_ERRORS, SEND_MODE_PAY_FEES_SEPARATELY,
};
use ton::ton_core::cell::TonCell;
use ton::ton_core::traits::tlb::TLB;
use wallet_engine::{
    AccountStatus, ActivityCursor, DomainError, Network, ProtectedSecretRef, ProviderConfig,
    ResourcePhase, SendAmount, SendPhase, SendPreview, SendPreviewRequest, SendRequest, SendResult,
    WalletClient, WalletClientConfig, WalletClientError, WalletHttpHost, WalletOperationOutcome,
    WalletUpdate,
};

use super::host::{MemoryPlatformHost, PlatformCallKind, RequestKind, ScenarioHttpHost};
use super::localnet::LocalnetHttpHost;
use super::test_wallet;

// Signing is CPU-heavy in debug builds. Parallel scenarios can legitimately
// take longer than a single isolated test without indicating a deadlock.
const DEFAULT_STEP_TIMEOUT: Duration = Duration::from_secs(60);
const NANOGRAMS_PER_GRAM: u64 = 1_000_000_000;
pub(crate) const EXACT_AMOUNT_SEND_MODE: u8 =
    SEND_MODE_PAY_FEES_SEPARATELY | SEND_MODE_IGNORE_ERRORS;
pub(crate) const ALL_BALANCE_SEND_MODE: u8 = SEND_MODE_CARRY_ALL_BALANCE | SEND_MODE_IGNORE_ERRORS;
const TEST_RECORD_ID: &str = "scenario-wallet";
const TEST_SECRET_REF: &str = "wallet:scenario-wallet:mnemonic";

fn step_timeout() -> Duration {
    std::env::var("WALLET_ENGINE_SCENARIO_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map_or(DEFAULT_STEP_TIMEOUT, Duration::from_secs)
}

pub(crate) fn scenario(name: impl Into<String>) -> Scenario {
    Scenario {
        name: name.into(),
        steps: Vec::new(),
    }
}

pub(crate) fn wallet() -> WalletFixture {
    WalletFixture {
        status: "active",
        balance_nanograms: "0".to_owned(),
        seqno: 0,
        sync_utime: Some(1_800_000_000),
    }
}

pub(crate) const fn submission() -> SubmissionFixture {
    SubmissionFixture { paused: None }
}

pub(crate) const fn secret() -> SecretFixture {
    SecretFixture {
        behavior: SecretBehavior::Valid,
    }
}

pub(crate) const fn journal() -> JournalFixture {
    JournalFixture {
        conflict_next_write: false,
        load_fails: false,
        failing_write: None,
    }
}

pub(crate) const fn provider() -> ProviderFixture {
    ProviderFixture {
        account_status: 200,
        activity_status: 200,
        account_retry_after_seconds: None,
        activity_malformed: false,
        account_redirected: false,
        emulation_status: 200,
        emulation_rejected: false,
    }
}

pub(crate) const fn client() -> ClientFixture {
    ClientFixture {
        send_validity_seconds: 300,
    }
}

pub(crate) fn activity_pages(pages: &[usize]) -> ActivityFixture {
    ActivityFixture {
        pages: pages.to_vec(),
    }
}

pub(crate) const fn network() -> NetworkFixtureBuilder {
    NetworkFixtureBuilder
}

pub(crate) fn send() -> SendAction {
    SendAction {
        destination: Destination::SelfWallet,
        amount: SendAmount::exact(NANOGRAMS_PER_GRAM.to_string()),
        comment: None,
    }
}

pub(crate) const fn preview_send(action: SendAction) -> UserAction {
    UserAction::PreviewSend(action)
}

pub(crate) const fn refresh_wallet() -> UserAction {
    UserAction::Refresh
}

pub(crate) const fn load_more_activity() -> UserAction {
    UserAction::LoadMoreActivity
}

pub(crate) const fn cancel_refresh() -> UserAction {
    UserAction::CancelRefresh
}

pub(crate) const fn cancel_load_more_activity() -> UserAction {
    UserAction::CancelLoadMoreActivity
}

pub(crate) const fn shutdown_client() -> UserAction {
    UserAction::Shutdown
}

pub(crate) const fn wait_for_change(after_revision: u64) -> UserAction {
    UserAction::WaitForChange { after_revision }
}

pub(crate) fn pause_next_account_request(name: impl Into<String>) -> ControlStep {
    ControlStep::PauseRequest {
        name: name.into(),
        kind: RequestKind::Account,
    }
}

pub(crate) fn pause_next_activity_request(name: impl Into<String>) -> ControlStep {
    ControlStep::PauseRequest {
        name: name.into(),
        kind: RequestKind::Activity,
    }
}

pub(crate) fn pause_next_seqno_request(name: impl Into<String>) -> ControlStep {
    ControlStep::PauseRequest {
        name: name.into(),
        kind: RequestKind::Seqno,
    }
}

pub(crate) fn pause_next_emulation_request(name: impl Into<String>) -> ControlStep {
    ControlStep::PauseRequest {
        name: name.into(),
        kind: RequestKind::Emulation,
    }
}

pub(crate) fn pause_next_journal_load(name: impl Into<String>) -> ControlStep {
    ControlStep::PausePlatformCall {
        name: name.into(),
        kind: PlatformCallKind::JournalLoad,
    }
}

pub(crate) fn pause_next_secret_read(name: impl Into<String>) -> ControlStep {
    ControlStep::PausePlatformCall {
        name: name.into(),
        kind: PlatformCallKind::SecretRead,
    }
}

pub(crate) fn wait_for_request(name: impl Into<String>) -> ControlStep {
    ControlStep::WaitForRequest { name: name.into() }
}

pub(crate) fn release_request(name: impl Into<String>) -> ControlStep {
    ControlStep::ReleaseRequest { name: name.into() }
}

pub(crate) fn wait_for_platform_call(name: impl Into<String>) -> ControlStep {
    ControlStep::WaitForPlatformCall { name: name.into() }
}

pub(crate) fn release_platform_call(name: impl Into<String>) -> ControlStep {
    ControlStep::ReleasePlatformCall { name: name.into() }
}

pub(crate) const fn fail_next_activity_request(status: u16) -> ControlStep {
    ControlStep::FailNextActivityRequest { status }
}

pub(crate) const fn cancel_next_activity_request_at_host() -> ControlStep {
    ControlStep::CancelNextActivityRequestAtHost
}

pub(crate) const fn spam_transfers(count: u32) -> UserAction {
    UserAction::SpamTransfers { count }
}

pub(crate) const fn replay_last_submission() -> UserAction {
    UserAction::ReplayLastSubmission
}

pub(crate) const fn own_address() -> Destination {
    Destination::SelfWallet
}

pub(crate) fn address(value: impl Into<String>) -> Destination {
    Destination::Address(value.into())
}

pub(crate) fn invalid_address() -> Destination {
    Destination::Address("not-a-ton-address".to_owned())
}

pub(crate) fn start(name: impl Into<String>, action: impl Into<UserAction>) -> ActionStep {
    ActionStep {
        name: name.into(),
        action: action.into(),
        wait: false,
    }
}

pub(crate) fn call(name: impl Into<String>, action: impl Into<UserAction>) -> ActionStep {
    ActionStep {
        name: name.into(),
        action: action.into(),
        wait: true,
    }
}

pub(crate) const fn cancel_send() -> UserAction {
    UserAction::CancelSend
}

pub(crate) const fn cancel_send_preview() -> UserAction {
    UserAction::CancelSendPreview
}

pub(crate) fn resume(name: impl Into<String>, outcome: SubmissionOutcome) -> ControlStep {
    ControlStep::ResumeSubmission {
        name: name.into(),
        outcome,
    }
}

pub(crate) const fn submission_accepted() -> SubmissionOutcome {
    SubmissionOutcome::Accepted
}

pub(crate) const fn submission_timeout() -> SubmissionOutcome {
    SubmissionOutcome::Timeout
}

pub(crate) const fn submission_malformed() -> SubmissionOutcome {
    SubmissionOutcome::MalformedSuccess
}

pub(crate) fn submission_http_failure(
    status: u16,
    diagnostic: impl Into<String>,
) -> SubmissionOutcome {
    SubmissionOutcome::HttpFailure {
        status,
        diagnostic: diagnostic.into(),
    }
}

pub(crate) fn submission_rejected(diagnostic: impl Into<String>) -> SubmissionOutcome {
    SubmissionOutcome::Rejected(diagnostic.into())
}

pub(crate) fn send_phase(name: impl Into<String>, phase: SendPhase) -> Expectation {
    Expectation::SendPhase {
        operation: name.into(),
        phase,
    }
}

pub(crate) fn error(name: impl Into<String>, expected: WalletClientError) -> Expectation {
    Expectation::Error {
        operation: name.into(),
        expected,
    }
}

pub(crate) fn emulation_failed(name: impl Into<String>) -> Expectation {
    Expectation::EmulationFailed {
        operation: name.into(),
    }
}

pub(crate) fn emulation_message_not_accepted(name: impl Into<String>) -> Expectation {
    Expectation::EmulationMessageNotAccepted {
        operation: name.into(),
    }
}

pub(crate) fn succeeds(name: impl Into<String>) -> Expectation {
    Expectation::Success {
        operation: name.into(),
    }
}

pub(crate) fn send_failed(diagnostic: impl Into<String>) -> WalletClientError {
    WalletClientError::SendFailed {
        diagnostic: diagnostic.into(),
    }
}

pub(crate) fn result(name: impl Into<String>) -> ResultExpectation {
    ResultExpectation { name: name.into() }
}

pub(crate) fn update(name: impl Into<String>) -> UpdateExpectation {
    UpdateExpectation { name: name.into() }
}

pub(crate) const fn account_status(status: AccountStatus) -> Expectation {
    Expectation::AccountStatus(status)
}

pub(crate) const fn activity_present() -> Expectation {
    Expectation::ActivityPresent
}

pub(crate) fn remember_activity_cursor(name: impl Into<String>) -> Expectation {
    Expectation::RememberActivityCursor(name.into())
}

pub(crate) fn pagination_used_cursor(name: impl Into<String>) -> Expectation {
    Expectation::PaginationUsedCursor(name.into())
}

pub(crate) fn remember_activity_as(name: impl Into<String>) -> Expectation {
    Expectation::RememberActivity {
        name: name.into(),
        only_new: false,
    }
}

pub(crate) fn remember_new_activity_as(name: impl Into<String>) -> Expectation {
    Expectation::RememberActivity {
        name: name.into(),
        only_new: true,
    }
}

pub(crate) fn activity_is(names: &[&str]) -> Expectation {
    Expectation::ActivityIs(names.iter().map(|name| (*name).to_owned()).collect())
}

pub(crate) fn request_was_cancelled(name: impl Into<String>) -> Expectation {
    Expectation::RequestWasCancelled(name.into())
}

pub(crate) fn remember_revision(name: impl Into<String>) -> Expectation {
    Expectation::RememberRevision(name.into())
}

pub(crate) fn revision_is(name: impl Into<String>) -> Expectation {
    Expectation::RevisionIs(name.into())
}

pub(crate) fn returned_snapshot_revision_is(
    operation: impl Into<String>,
    revision: impl Into<String>,
) -> Expectation {
    Expectation::ReturnedSnapshotRevisionIs {
        operation: operation.into(),
        revision: revision.into(),
    }
}

pub(crate) fn returned_snapshot_revision_is_greater_than(
    operation: impl Into<String>,
    after_revision: u64,
) -> Expectation {
    Expectation::ReturnedSnapshotRevisionIsGreaterThan {
        operation: operation.into(),
        after_revision,
    }
}

pub(crate) const fn protected_secret_was_not_read() -> Expectation {
    Expectation::SecretReadCount(0)
}

pub(crate) const fn journal_is_empty() -> Expectation {
    Expectation::JournalIsEmpty
}

pub(crate) const fn no_message_was_submitted() -> Expectation {
    Expectation::NoSubmittedMessage
}

pub(crate) const fn submitted_message() -> SubmittedMessageExpectation {
    SubmittedMessageExpectation
}

pub(crate) const fn message_was_submitted() -> Expectation {
    Expectation::SubmittedMessagePresent
}

pub(crate) const fn on_chain_wallet() -> OnChainWalletExpectation {
    OnChainWalletExpectation {
        active: false,
        seqno: None,
    }
}

pub(crate) const fn snapshot() -> SnapshotExpectation {
    SnapshotExpectation {
        send_phase: None,
        account_phase: None,
        activity_phase: None,
        pagination_phase: None,
        activity_count: None,
        has_more: None,
        account_error: None,
        activity_error: None,
        send_error_message: None,
    }
}

#[derive(Clone)]
pub(crate) struct WalletFixture {
    pub(super) status: &'static str,
    pub(super) balance_nanograms: String,
    pub(super) seqno: u32,
    pub(super) sync_utime: Option<u64>,
}

impl WalletFixture {
    #[must_use]
    pub(crate) const fn active(mut self) -> Self {
        self.status = "active";
        self
    }

    #[must_use]
    pub(crate) const fn uninitialized(mut self) -> Self {
        self.status = "uninitialized";
        self.seqno = 0;
        self
    }

    #[must_use]
    pub(crate) const fn frozen(mut self) -> Self {
        self.status = "frozen";
        self
    }

    #[must_use]
    pub(crate) const fn unknown(mut self) -> Self {
        self.status = "provider-specific-state";
        self
    }

    #[must_use]
    pub(crate) const fn without_sync_time(mut self) -> Self {
        self.sync_utime = None;
        self
    }

    #[must_use]
    pub(crate) const fn sync_time(mut self, sync_utime: u64) -> Self {
        self.sync_utime = Some(sync_utime);
        self
    }

    #[must_use]
    pub(crate) fn balance(mut self, amount: GramAmount) -> Self {
        self.balance_nanograms = amount.nanograms;
        self
    }

    #[must_use]
    pub(crate) const fn seqno(mut self, seqno: u32) -> Self {
        self.seqno = seqno;
        self
    }
}

pub(crate) struct GramAmount {
    nanograms: String,
}

impl GramAmount {
    pub(crate) fn as_nanograms(&self) -> String {
        self.nanograms.clone()
    }
}

pub(crate) fn grams(value: u64) -> GramAmount {
    let Some(nanograms) = value.checked_mul(NANOGRAMS_PER_GRAM) else {
        panic!("GRAM fixture exceeds u64 nanograms");
    };

    GramAmount {
        nanograms: nanograms.to_string(),
    }
}

pub(crate) struct SubmissionFixture {
    paused: Option<String>,
}

pub(crate) struct SecretFixture {
    behavior: SecretBehavior,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SecretBehavior {
    Valid,
    Invalid,
    AnotherWallet,
    HostFailure,
}

impl SecretFixture {
    #[must_use]
    pub(crate) const fn invalid(mut self) -> Self {
        self.behavior = SecretBehavior::Invalid;
        self
    }

    #[must_use]
    pub(crate) const fn belongs_to_another_wallet(mut self) -> Self {
        self.behavior = SecretBehavior::AnotherWallet;
        self
    }

    #[must_use]
    pub(crate) const fn host_fails(mut self) -> Self {
        self.behavior = SecretBehavior::HostFailure;
        self
    }
}

pub(crate) struct JournalFixture {
    conflict_next_write: bool,
    load_fails: bool,
    failing_write: Option<u64>,
}

pub(crate) struct ClientFixture {
    send_validity_seconds: u32,
}

impl ClientFixture {
    #[must_use]
    pub(crate) const fn send_validity_seconds(mut self, seconds: u32) -> Self {
        self.send_validity_seconds = seconds;
        self
    }
}

pub(crate) struct ProviderFixture {
    pub(super) account_status: u16,
    pub(super) activity_status: u16,
    pub(super) account_retry_after_seconds: Option<u64>,
    pub(super) activity_malformed: bool,
    pub(super) account_redirected: bool,
    pub(super) emulation_status: u16,
    pub(super) emulation_rejected: bool,
}

pub(crate) struct ActivityFixture {
    pages: Vec<usize>,
}

impl ProviderFixture {
    #[must_use]
    pub(crate) const fn account_fails(mut self, status: u16) -> Self {
        self.account_status = status;
        self
    }

    #[must_use]
    pub(crate) const fn activity_fails(mut self, status: u16) -> Self {
        self.activity_status = status;
        self
    }

    #[must_use]
    pub(crate) const fn account_is_rate_limited(mut self, retry_after_seconds: u64) -> Self {
        self.account_status = 429;
        self.account_retry_after_seconds = Some(retry_after_seconds);
        self
    }

    #[must_use]
    pub(crate) const fn activity_returns_malformed_json(mut self) -> Self {
        self.activity_malformed = true;
        self
    }

    #[must_use]
    pub(crate) const fn account_redirects(mut self) -> Self {
        self.account_redirected = true;
        self
    }

    #[must_use]
    pub(crate) const fn emulation_fails(mut self, status: u16) -> Self {
        self.emulation_status = status;
        self
    }

    #[must_use]
    pub(crate) const fn emulation_rejects_transfer(mut self) -> Self {
        self.emulation_rejected = true;
        self
    }
}

impl JournalFixture {
    #[must_use]
    pub(crate) const fn conflicts_on_next_write(mut self) -> Self {
        self.conflict_next_write = true;
        self
    }

    #[must_use]
    pub(crate) const fn load_fails(mut self) -> Self {
        self.load_fails = true;
        self
    }

    #[must_use]
    pub(crate) const fn write_fails(mut self, write_number: u64) -> Self {
        self.failing_write = Some(write_number);
        self
    }
}

pub(crate) struct NetworkFixtureBuilder;

impl NetworkFixtureBuilder {
    pub(crate) const fn localnet(self) -> NetworkFixture {
        NetworkFixture::Localnet
    }
}

pub(crate) enum NetworkFixture {
    Localnet,
}

impl SubmissionFixture {
    #[must_use]
    pub(crate) fn paused(mut self, name: impl Into<String>) -> Self {
        self.paused = Some(name.into());
        self
    }
}

pub(crate) struct SendAction {
    destination: Destination,
    amount: SendAmount,
    comment: Option<String>,
}

pub(crate) enum Destination {
    SelfWallet,
    Address(String),
}

impl SendAction {
    #[must_use]
    pub(crate) fn to(mut self, destination: Destination) -> Self {
        self.destination = destination;
        self
    }

    #[must_use]
    pub(crate) fn grams(mut self, value: u64) -> Self {
        self.amount = SendAmount::exact(grams(value).nanograms);
        self
    }

    #[must_use]
    pub(crate) fn nanograms(mut self, value: u64) -> Self {
        self.amount = SendAmount::exact(value.to_string());
        self
    }

    #[must_use]
    pub(crate) fn all(mut self) -> Self {
        self.amount = SendAmount::All;
        self
    }

    #[must_use]
    pub(crate) fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }
}

pub(crate) struct ActionStep {
    name: String,
    action: UserAction,
    wait: bool,
}

pub(crate) enum UserAction {
    Send(SendAction),
    PreviewSend(SendAction),
    CancelSend,
    CancelSendPreview,
    Refresh,
    CancelRefresh,
    LoadMoreActivity,
    CancelLoadMoreActivity,
    Shutdown,
    WaitForChange { after_revision: u64 },
    SpamTransfers { count: u32 },
    ReplayLastSubmission,
}

impl From<SendAction> for UserAction {
    fn from(value: SendAction) -> Self {
        Self::Send(value)
    }
}

#[derive(Clone)]
pub(crate) enum SubmissionOutcome {
    Accepted,
    Rejected(String),
    Timeout,
    MalformedSuccess,
    HttpFailure { status: u16, diagnostic: String },
}

pub(crate) enum ControlStep {
    ResumeSubmission {
        name: String,
        outcome: SubmissionOutcome,
    },
    PauseRequest {
        name: String,
        kind: RequestKind,
    },
    WaitForRequest {
        name: String,
    },
    ReleaseRequest {
        name: String,
    },
    PausePlatformCall {
        name: String,
        kind: PlatformCallKind,
    },
    WaitForPlatformCall {
        name: String,
    },
    ReleasePlatformCall {
        name: String,
    },
    FailNextActivityRequest {
        status: u16,
    },
    CancelNextActivityRequestAtHost,
}

pub(crate) enum Given {
    Wallet(WalletFixture),
    Submission(SubmissionFixture),
    Secret(SecretFixture),
    Journal(JournalFixture),
    Provider(ProviderFixture),
    Activity(ActivityFixture),
    Network(NetworkFixture),
    Client(ClientFixture),
}

impl From<WalletFixture> for Given {
    fn from(value: WalletFixture) -> Self {
        Self::Wallet(value)
    }
}

impl From<SubmissionFixture> for Given {
    fn from(value: SubmissionFixture) -> Self {
        Self::Submission(value)
    }
}

impl From<SecretFixture> for Given {
    fn from(value: SecretFixture) -> Self {
        Self::Secret(value)
    }
}

impl From<JournalFixture> for Given {
    fn from(value: JournalFixture) -> Self {
        Self::Journal(value)
    }
}

impl From<ProviderFixture> for Given {
    fn from(value: ProviderFixture) -> Self {
        Self::Provider(value)
    }
}

impl From<ActivityFixture> for Given {
    fn from(value: ActivityFixture) -> Self {
        Self::Activity(value)
    }
}

impl From<NetworkFixture> for Given {
    fn from(value: NetworkFixture) -> Self {
        Self::Network(value)
    }
}

impl From<ClientFixture> for Given {
    fn from(value: ClientFixture) -> Self {
        Self::Client(value)
    }
}

pub(crate) enum When {
    Action(ActionStep),
    Control(ControlStep),
}

impl From<ActionStep> for When {
    fn from(value: ActionStep) -> Self {
        Self::Action(value)
    }
}

impl From<ControlStep> for When {
    fn from(value: ControlStep) -> Self {
        Self::Control(value)
    }
}

pub(crate) enum Expectation {
    SendPhase {
        operation: String,
        phase: SendPhase,
    },
    Error {
        operation: String,
        expected: WalletClientError,
    },
    EmulationFailed {
        operation: String,
    },
    EmulationMessageNotAccepted {
        operation: String,
    },
    Success {
        operation: String,
    },
    ResultPhase {
        operation: String,
        phase: SendPhase,
    },
    ResultPreviewed {
        operation: String,
    },
    ResultEmulationAction {
        operation: String,
        kind: String,
    },
    UpdateOutcome {
        operation: String,
        outcome: WalletOperationOutcome,
    },
    UpdateAddedItems {
        operation: String,
        count: u64,
    },
    UpdateAddedAnyItems {
        operation: String,
    },
    Snapshot(SnapshotExpectation),
    AccountStatus(AccountStatus),
    ActivityPresent,
    RememberActivityCursor(String),
    PaginationUsedCursor(String),
    RememberActivity {
        name: String,
        only_new: bool,
    },
    ActivityIs(Vec<String>),
    RequestWasCancelled(String),
    RememberRevision(String),
    RevisionIs(String),
    ReturnedSnapshotRevisionIs {
        operation: String,
        revision: String,
    },
    ReturnedSnapshotRevisionIsGreaterThan {
        operation: String,
        after_revision: u64,
    },
    SecretReadCount(u64),
    JournalIsEmpty,
    NoSubmittedMessage,
    SubmittedMessageContainsStateInit,
    SubmittedMessageUsesMode(u8),
    SubmittedMessageHasComment(String),
    SubmittedMessagePresent,
    OnChainWallet(OnChainWalletExpectation),
}

pub(crate) struct SubmittedMessageExpectation;

impl SubmittedMessageExpectation {
    pub(crate) const fn contains_state_init(self) -> Expectation {
        Expectation::SubmittedMessageContainsStateInit
    }

    pub(crate) const fn uses_send_mode(self, mode: u8) -> Expectation {
        Expectation::SubmittedMessageUsesMode(mode)
    }

    pub(crate) fn has_comment(self, comment: impl Into<String>) -> Expectation {
        Expectation::SubmittedMessageHasComment(comment.into())
    }
}

pub(crate) struct OnChainWalletExpectation {
    active: bool,
    seqno: Option<u32>,
}

impl OnChainWalletExpectation {
    pub(crate) const fn uninitialized(self) -> Expectation {
        Expectation::OnChainWallet(self)
    }

    #[must_use]
    pub(crate) const fn active(mut self) -> Self {
        self.active = true;
        self
    }

    pub(crate) const fn seqno(mut self, seqno: u32) -> Expectation {
        self.seqno = Some(seqno);
        Expectation::OnChainWallet(self)
    }
}

pub(crate) struct ResultExpectation {
    name: String,
}

pub(crate) struct UpdateExpectation {
    name: String,
}

impl UpdateExpectation {
    pub(crate) fn completed(self) -> Expectation {
        Expectation::UpdateOutcome {
            operation: self.name,
            outcome: WalletOperationOutcome::Completed,
        }
    }

    pub(crate) fn partially_completed(self) -> Expectation {
        self.outcome(WalletOperationOutcome::PartiallyCompleted)
    }

    pub(crate) fn failed(self) -> Expectation {
        self.outcome(WalletOperationOutcome::Failed)
    }

    pub(crate) fn cancelled(self) -> Expectation {
        self.outcome(WalletOperationOutcome::Cancelled)
    }

    pub(crate) fn skipped(self) -> Expectation {
        self.outcome(WalletOperationOutcome::Skipped)
    }

    pub(crate) fn superseded(self) -> Expectation {
        self.outcome(WalletOperationOutcome::Superseded)
    }

    pub(crate) fn added_items(self, count: u64) -> Expectation {
        Expectation::UpdateAddedItems {
            operation: self.name,
            count,
        }
    }

    pub(crate) fn added_any_items(self) -> Expectation {
        Expectation::UpdateAddedAnyItems {
            operation: self.name,
        }
    }

    fn outcome(self, outcome: WalletOperationOutcome) -> Expectation {
        Expectation::UpdateOutcome {
            operation: self.name,
            outcome,
        }
    }
}

impl ResultExpectation {
    pub(crate) fn previewed(self) -> Expectation {
        Expectation::ResultPreviewed {
            operation: self.name,
        }
    }

    #[must_use]
    pub(crate) fn submitted(self) -> Expectation {
        self.phase(SendPhase::Submitted)
    }

    #[must_use]
    pub(crate) fn failed(self) -> Expectation {
        self.phase(SendPhase::Failed)
    }

    #[must_use]
    pub(crate) fn submission_unknown(self) -> Expectation {
        self.phase(SendPhase::SubmissionUnknown)
    }

    pub(crate) fn emulated_action(self, kind: impl Into<String>) -> Expectation {
        Expectation::ResultEmulationAction {
            operation: self.name,
            kind: kind.into(),
        }
    }

    fn phase(self, phase: SendPhase) -> Expectation {
        Expectation::ResultPhase {
            operation: self.name,
            phase,
        }
    }
}

pub(crate) struct SnapshotExpectation {
    send_phase: Option<SendPhase>,
    account_phase: Option<ResourcePhase>,
    activity_phase: Option<ResourcePhase>,
    pagination_phase: Option<ResourcePhase>,
    activity_count: Option<usize>,
    has_more: Option<bool>,
    account_error: Option<DomainError>,
    activity_error: Option<DomainError>,
    send_error_message: Option<String>,
}

impl SnapshotExpectation {
    #[must_use]
    pub(crate) const fn send_phase(mut self, phase: SendPhase) -> Expectation {
        self.send_phase = Some(phase);
        Expectation::Snapshot(self)
    }

    #[must_use]
    pub(crate) const fn account_phase(mut self, phase: ResourcePhase) -> Self {
        self.account_phase = Some(phase);
        self
    }

    pub(crate) const fn activity_phase(mut self, phase: ResourcePhase) -> Expectation {
        self.activity_phase = Some(phase);
        Expectation::Snapshot(self)
    }

    #[must_use]
    pub(crate) const fn pagination_phase(mut self, phase: ResourcePhase) -> Self {
        self.pagination_phase = Some(phase);
        self
    }

    #[must_use]
    pub(crate) const fn activity_count(mut self, count: usize) -> Self {
        self.activity_count = Some(count);
        self
    }

    pub(crate) const fn has_more(mut self, has_more: bool) -> Expectation {
        self.has_more = Some(has_more);
        Expectation::Snapshot(self)
    }

    #[must_use]
    pub(crate) fn account_error(mut self, error: DomainError) -> Self {
        self.account_error = Some(error);
        self
    }

    pub(crate) fn activity_error(mut self, error: DomainError) -> Expectation {
        self.activity_error = Some(error);
        Expectation::Snapshot(self)
    }

    pub(crate) fn send_error_message(mut self, message: impl Into<String>) -> Expectation {
        self.send_error_message = Some(message.into());
        Expectation::Snapshot(self)
    }
}

enum Step {
    Given(Given),
    When(When),
    Then(Expectation),
}

pub(crate) struct Scenario {
    name: String,
    steps: Vec<Step>,
}

impl Scenario {
    #[must_use]
    pub(crate) fn given(mut self, fixture: impl Into<Given>) -> Self {
        self.steps.push(Step::Given(fixture.into()));
        self
    }

    #[must_use]
    pub(crate) fn when(mut self, action: impl Into<When>) -> Self {
        self.steps.push(Step::When(action.into()));
        self
    }

    #[must_use]
    pub(crate) fn then(mut self, expectation: Expectation) -> Self {
        self.steps.push(Step::Then(expectation));
        self
    }

    pub(crate) fn run(self) {
        if let Err(failure) = ScenarioRunner::new(&self.name, &self.steps)
            .and_then(|mut runner| runner.run(self.steps))
        {
            panic!("{failure}");
        }
    }
}

struct RunningOperation {
    receiver: Receiver<OperationResult>,
    thread: Option<JoinHandle<()>>,
}

struct ScenarioTransport {
    client_host: Arc<dyn WalletHttpHost>,
    scripted_host: Option<Arc<ScenarioHttpHost>>,
    localnet_host: Option<Arc<LocalnetHttpHost>>,
    provider_base_url: String,
}

struct ScenarioRunner {
    name: String,
    client: Arc<WalletClient>,
    platform_host: Arc<MemoryPlatformHost>,
    scripted_http_host: Option<Arc<ScenarioHttpHost>>,
    localnet_http_host: Option<Arc<LocalnetHttpHost>>,
    secret_ref: ProtectedSecretRef,
    address: String,
    operations: HashMap<String, RunningOperation>,
    results: HashMap<String, OperationResult>,
    activity_cursors: HashMap<String, ActivityCursor>,
    named_activity: HashMap<String, HashSet<String>>,
    named_revisions: HashMap<String, u64>,
}

#[derive(Debug)]
enum OperationResult {
    Send(Result<SendResult, WalletClientError>),
    Preview(Result<SendPreview, WalletClientError>),
    Update(Box<Result<WalletUpdate, WalletClientError>>),
    Snapshot(Box<Result<wallet_engine::WalletSnapshot, WalletClientError>>),
    Unit(Result<(), WalletClientError>),
    Harness(Result<(), String>),
}

impl OperationResult {
    fn error(&self) -> Option<&WalletClientError> {
        match self {
            Self::Send(Err(error)) | Self::Preview(Err(error)) | Self::Unit(Err(error)) => {
                Some(error)
            }
            Self::Update(result) => result.as_ref().as_ref().err(),
            Self::Snapshot(result) => result.as_ref().as_ref().err(),
            Self::Send(Ok(_)) | Self::Preview(Ok(_)) | Self::Unit(Ok(())) | Self::Harness(_) => {
                None
            }
        }
    }
}

impl ScenarioRunner {
    fn new(name: &str, steps: &[Step]) -> Result<Self, String> {
        let mut wallet = wallet();
        let mut paused_submission = None;
        let mut use_localnet = false;
        let mut secret_behavior = SecretBehavior::Valid;
        let mut journal_conflict = false;
        let mut journal_load_fails = false;
        let mut failing_journal_write = None;
        let mut provider_fixture = provider();
        let mut pages = Vec::new();
        let mut send_validity_seconds = 300;

        for step in steps {
            match step {
                Step::Given(Given::Wallet(fixture)) => wallet = fixture.clone(),
                Step::Given(Given::Submission(fixture)) => {
                    paused_submission.clone_from(&fixture.paused);
                }
                Step::Given(Given::Network(NetworkFixture::Localnet)) => use_localnet = true,
                Step::Given(Given::Secret(fixture)) => secret_behavior = fixture.behavior,
                Step::Given(Given::Journal(fixture)) => {
                    journal_conflict = fixture.conflict_next_write;
                    journal_load_fails = fixture.load_fails;
                    failing_journal_write = fixture.failing_write;
                }
                Step::Given(Given::Provider(fixture)) => {
                    provider_fixture = ProviderFixture {
                        account_status: fixture.account_status,
                        activity_status: fixture.activity_status,
                        account_retry_after_seconds: fixture.account_retry_after_seconds,
                        activity_malformed: fixture.activity_malformed,
                        account_redirected: fixture.account_redirected,
                        emulation_status: fixture.emulation_status,
                        emulation_rejected: fixture.emulation_rejected,
                    };
                }
                Step::Given(Given::Activity(fixture)) => pages.clone_from(&fixture.pages),
                Step::Given(Given::Client(fixture)) => {
                    send_validity_seconds = fixture.send_validity_seconds;
                }
                Step::When(_) | Step::Then(_) => break,
            }
        }

        let platform_host = Arc::new(MemoryPlatformHost::default());
        let secret_ref = ProtectedSecretRef {
            value: TEST_SECRET_REF.to_owned(),
        };
        let secret = match secret_behavior {
            SecretBehavior::Valid | SecretBehavior::HostFailure => {
                test_wallet().recovery_phrase_bytes()
            }
            SecretBehavior::Invalid => b"invalid recovery phrase",
            SecretBehavior::AnotherWallet => test_wallet().other_recovery_phrase_bytes(),
        };
        platform_host.store_test_secret(&secret_ref, secret);
        if secret_behavior == SecretBehavior::HostFailure {
            platform_host.fail_next_secret_read();
        }
        if journal_conflict {
            platform_host.conflict_next_journal_write();
        }
        if journal_load_fails {
            platform_host.fail_next_journal_load();
        }
        if let Some(write_number) = failing_journal_write {
            platform_host.fail_journal_write(write_number);
        }
        let transport = if use_localnet {
            if wallet.status != "uninitialized" {
                return Err("localnet scenarios must start with an uninitialized wallet".to_owned());
            }
            if paused_submission.is_some() {
                return Err("localnet scenarios cannot pause provider submission".to_owned());
            }

            let host = Arc::new(LocalnetHttpHost::start(
                test_wallet().testnet_v5_address(),
                &wallet.balance_nanograms,
            )?);
            let provider_base_url = host.provider_base_url();
            ScenarioTransport {
                client_host: host.clone(),
                scripted_host: None,
                localnet_host: Some(host),
                provider_base_url,
            }
        } else {
            let host = Arc::new(ScenarioHttpHost::new(wallet, paused_submission));
            host.set_provider_behavior(&provider_fixture);
            host.set_activity_pages(pages);
            ScenarioTransport {
                client_host: host.clone(),
                scripted_host: Some(host),
                localnet_host: None,
                provider_base_url: "https://testnet.toncenter.com".to_owned(),
            }
        };
        let ScenarioTransport {
            client_host,
            scripted_host,
            localnet_host,
            provider_base_url,
        } = transport;
        let client = WalletClient::new(
            WalletClientConfig {
                record_id: TEST_RECORD_ID.to_owned(),
                address: test_wallet().testnet_v5_address().to_owned(),
                public_key: test_wallet().public_key(),
                network: Network::Testnet,
                send_validity_seconds,
                providers: ProviderConfig {
                    toncenter_base_url: provider_base_url,
                },
            },
            client_host,
            platform_host.clone(),
        )
        .map_err(|error| format!("scenario `{name}` could not create its client: {error}"))?;

        Ok(Self {
            name: name.to_owned(),
            client,
            platform_host,
            scripted_http_host: scripted_host,
            localnet_http_host: localnet_host,
            secret_ref,
            address: test_wallet().testnet_v5_address().to_owned(),
            operations: HashMap::new(),
            results: HashMap::new(),
            activity_cursors: HashMap::new(),
            named_activity: HashMap::new(),
            named_revisions: HashMap::new(),
        })
    }

    fn run(&mut self, steps: Vec<Step>) -> Result<(), String> {
        for (index, step) in steps.into_iter().enumerate() {
            let step_number = index + 1;
            let result = match step {
                Step::Given(given) => self.execute_given(given),
                Step::When(when) => self.execute_when(when),
                Step::Then(expectation) => self.assert(expectation),
            };

            if let Err(message) = result {
                return Err(format!(
                    "scenario: {}\nstep {step_number} failed:\n{message}",
                    self.name
                ));
            }
        }

        if self.operations.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "scenario ended with running operations: {:?}",
                self.operations.keys().collect::<Vec<_>>()
            ))
        }
    }

    fn execute_given(&self, given: Given) -> Result<(), String> {
        match given {
            Given::Wallet(wallet) => {
                if let Some(http_host) = &self.scripted_http_host {
                    http_host.set_wallet(wallet);
                }
                Ok(())
            }
            Given::Submission(submission) => {
                let http_host = self.scripted_http_host.as_ref().ok_or_else(|| {
                    "localnet scenarios cannot change provider submission behavior".to_owned()
                })?;
                let name = submission
                    .paused
                    .ok_or_else(|| "submission fixture has no behavior".to_owned())?;
                http_host.pause_submission(name);
                Ok(())
            }
            Given::Secret(secret) => {
                let bytes = match secret.behavior {
                    SecretBehavior::Valid | SecretBehavior::HostFailure => {
                        test_wallet().recovery_phrase_bytes()
                    }
                    SecretBehavior::Invalid => b"invalid recovery phrase",
                    SecretBehavior::AnotherWallet => test_wallet().other_recovery_phrase_bytes(),
                };
                self.platform_host
                    .store_test_secret(&self.secret_ref, bytes);
                if secret.behavior == SecretBehavior::HostFailure {
                    self.platform_host.fail_next_secret_read();
                }
                Ok(())
            }
            Given::Journal(journal) => {
                if journal.conflict_next_write {
                    self.platform_host.conflict_next_journal_write();
                }
                if journal.load_fails {
                    self.platform_host.fail_next_journal_load();
                }
                if let Some(write_number) = journal.failing_write {
                    self.platform_host.fail_journal_write(write_number);
                }
                Ok(())
            }
            Given::Provider(provider) => {
                let host = self.scripted_http_host.as_ref().ok_or_else(|| {
                    "localnet scenarios cannot script provider failures".to_owned()
                })?;
                host.set_provider_behavior(&provider);
                Ok(())
            }
            Given::Activity(activity) => {
                self.scripted_http_host
                    .as_ref()
                    .ok_or_else(|| "localnet scenarios cannot script activity".to_owned())?
                    .set_activity_pages(activity.pages);
                Ok(())
            }
            Given::Network(NetworkFixture::Localnet) | Given::Client(_) => Ok(()),
        }
    }

    fn execute_when(&mut self, when: When) -> Result<(), String> {
        match when {
            When::Action(step) => {
                let name = step.name;
                if self.operations.contains_key(&name) || self.results.contains_key(&name) {
                    return Err(format!("operation `{name}` already exists"));
                }

                let client = self.client.clone();
                let (sender, receiver) = channel();
                let thread = match step.action {
                    UserAction::PreviewSend(action) => {
                        let destination = match action.destination {
                            Destination::SelfWallet => self.address.clone(),
                            Destination::Address(address) => address,
                        };
                        let request = SendPreviewRequest {
                            destination,
                            amount: action.amount,
                            comment: action.comment,
                        };
                        std::thread::spawn(move || {
                            let result = block_on(client.preview_send(request));
                            let _ = sender.send(OperationResult::Preview(result));
                        })
                    }
                    UserAction::Send(action) => {
                        let destination = match action.destination {
                            Destination::SelfWallet => self.address.clone(),
                            Destination::Address(address) => address,
                        };
                        let request = SendRequest {
                            operation_id: format!("{name}-operation"),
                            destination,
                            amount: action.amount,
                            comment: action.comment,
                            secret_ref: self.secret_ref.clone(),
                        };
                        std::thread::spawn(move || {
                            let result = block_on(client.send(request));
                            let _ = sender.send(OperationResult::Send(result));
                        })
                    }
                    UserAction::CancelSend => std::thread::spawn(move || {
                        let result = block_on(client.cancel_send());
                        let _ = sender.send(OperationResult::Unit(result));
                    }),
                    UserAction::CancelSendPreview => std::thread::spawn(move || {
                        let result = block_on(client.cancel_send_preview());
                        let _ = sender.send(OperationResult::Unit(result));
                    }),
                    UserAction::Refresh => std::thread::spawn(move || {
                        let result = block_on(client.refresh());
                        let _ = sender.send(OperationResult::Update(Box::new(result)));
                    }),
                    UserAction::CancelRefresh => std::thread::spawn(move || {
                        let result = block_on(client.cancel_refresh());
                        let _ = sender.send(OperationResult::Unit(result));
                    }),
                    UserAction::LoadMoreActivity => std::thread::spawn(move || {
                        let result = block_on(client.load_more_activity());
                        let _ = sender.send(OperationResult::Update(Box::new(result)));
                    }),
                    UserAction::CancelLoadMoreActivity => std::thread::spawn(move || {
                        let result = block_on(client.cancel_load_more_activity());
                        let _ = sender.send(OperationResult::Unit(result));
                    }),
                    UserAction::Shutdown => std::thread::spawn(move || {
                        let result = block_on(client.shutdown());
                        let _ = sender.send(OperationResult::Unit(result));
                    }),
                    UserAction::WaitForChange { after_revision } => std::thread::spawn(move || {
                        let result = block_on(client.wait_for_change(after_revision));
                        let _ = sender.send(OperationResult::Snapshot(Box::new(result)));
                    }),
                    UserAction::SpamTransfers { count } => {
                        let localnet = self.localnet_http_host.clone();
                        std::thread::spawn(move || {
                            let result = localnet
                                .ok_or_else(|| {
                                    "spam transfers require `.given(network().localnet())`"
                                        .to_owned()
                                })
                                .and_then(|host| host.spam_transfers(count));
                            let _ = sender.send(OperationResult::Harness(result));
                        })
                    }
                    UserAction::ReplayLastSubmission => {
                        let localnet = self.localnet_http_host.clone();
                        std::thread::spawn(move || {
                            let result = localnet
                                .ok_or_else(|| {
                                    "submission replay requires `.given(network().localnet())`"
                                        .to_owned()
                                })
                                .and_then(|host| host.replay_last_submission());
                            let _ = sender.send(OperationResult::Harness(result));
                        })
                    }
                };
                self.operations.insert(
                    name.clone(),
                    RunningOperation {
                        receiver,
                        thread: Some(thread),
                    },
                );

                if step.wait {
                    self.finish_operation(&name)?;
                }

                Ok(())
            }
            When::Control(ControlStep::ResumeSubmission { name, outcome }) => self
                .scripted_http_host
                .as_ref()
                .ok_or_else(|| "localnet scenarios cannot resume provider submission".to_owned())?
                .resume_submission(&name, outcome),
            When::Control(ControlStep::PauseRequest { name, kind }) => {
                if let Some(host) = &self.scripted_http_host {
                    host.pause_next_request(name, kind);
                } else if let Some(host) = &self.localnet_http_host {
                    host.pause_next_request(name, kind);
                } else {
                    return Err("scenario has no HTTP host".to_owned());
                }
                Ok(())
            }
            When::Control(ControlStep::WaitForRequest { name }) => {
                if let Some(host) = &self.scripted_http_host {
                    host.wait_for_request(&name)
                } else if let Some(host) = &self.localnet_http_host {
                    host.wait_for_request(&name)
                } else {
                    Err("scenario has no HTTP host".to_owned())
                }
            }
            When::Control(ControlStep::ReleaseRequest { name }) => {
                if let Some(host) = &self.scripted_http_host {
                    host.release_request(&name)
                } else if let Some(host) = &self.localnet_http_host {
                    host.release_request(&name)
                } else {
                    Err("scenario has no HTTP host".to_owned())
                }
            }
            When::Control(ControlStep::PausePlatformCall { name, kind }) => {
                self.platform_host.pause_next_platform_call(name, kind);
                Ok(())
            }
            When::Control(ControlStep::WaitForPlatformCall { name }) => {
                self.platform_host.wait_for_platform_call(&name)
            }
            When::Control(ControlStep::ReleasePlatformCall { name }) => {
                self.platform_host.release_platform_call(&name)
            }
            When::Control(ControlStep::FailNextActivityRequest { status }) => {
                let host = self.scripted_http_host.as_ref().ok_or_else(|| {
                    "localnet scenarios cannot script provider failures".to_owned()
                })?;
                host.fail_next_activity_response(status);
                Ok(())
            }
            When::Control(ControlStep::CancelNextActivityRequestAtHost) => {
                let host = self.scripted_http_host.as_ref().ok_or_else(|| {
                    "localnet scenarios cannot script transport cancellation".to_owned()
                })?;
                host.cancel_next_activity_response();
                Ok(())
            }
        }
    }

    fn assert(&mut self, expectation: Expectation) -> Result<(), String> {
        match expectation {
            Expectation::SendPhase { operation, phase } => {
                let operation_id = format!("{operation}-operation");
                self.eventually(|| {
                    let snapshot = self.client.snapshot().map_err(|error| error.to_string())?;
                    if snapshot.send.operation_id.as_deref() == Some(operation_id.as_str())
                        && snapshot.send.phase == phase
                    {
                        Ok(true)
                    } else {
                        Ok(false)
                    }
                })
                .map_err(|message| {
                    format!("expected operation `{operation}` to reach {phase:?}\n{message}")
                })
            }
            Expectation::Error {
                operation,
                expected,
            } => {
                self.finish_operation(&operation)?;
                match self.results.get(&operation) {
                    Some(result) if result.error() == Some(&expected) => Ok(()),
                    actual => Err(format!(
                        "expected `{operation}` to return {expected:?}\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::EmulationFailed { operation } => {
                self.finish_operation(&operation)?;
                match self
                    .results
                    .get(&operation)
                    .and_then(OperationResult::error)
                {
                    Some(WalletClientError::EmulationFailed { diagnostic })
                        if !diagnostic.is_empty() =>
                    {
                        Ok(())
                    }
                    actual => Err(format!(
                        "expected `{operation}` to return a nonempty EmulationFailed diagnostic\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::EmulationMessageNotAccepted { operation } => {
                self.finish_operation(&operation)?;
                match self
                    .results
                    .get(&operation)
                    .and_then(OperationResult::error)
                {
                    Some(WalletClientError::EmulationMessageNotAccepted { diagnostic })
                        if !diagnostic.is_empty() =>
                    {
                        Ok(())
                    }
                    actual => Err(format!(
                        "expected `{operation}` to return a nonempty EmulationMessageNotAccepted diagnostic\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::Success { operation } => {
                self.finish_operation(&operation)?;
                match self.results.get(&operation) {
                    Some(OperationResult::Unit(Ok(())) | OperationResult::Harness(Ok(()))) => {
                        Ok(())
                    }
                    actual => Err(format!(
                        "expected `{operation}` to succeed\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::ResultPhase { operation, phase } => {
                self.finish_operation(&operation)?;
                match self.results.get(&operation) {
                    Some(OperationResult::Send(Ok(result))) if result.phase == phase => Ok(()),
                    actual => Err(format!(
                        "expected `{operation}` to finish with {phase:?}\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::ResultPreviewed { operation } => {
                self.finish_operation(&operation)?;
                match self.results.get(&operation) {
                    Some(OperationResult::Preview(Ok(preview)))
                        if preview.emulation.transaction_count > 0
                            && !preview.emulation.wallet_fees_nanograms.is_empty()
                            && !preview.emulation.trace_fees_nanograms.is_empty()
                            && STANDARD
                                .decode(&preview.message_boc_base64)
                                .ok()
                                .is_some_and(|bytes| TonCell::from_boc(bytes).is_ok()) =>
                    {
                        Ok(())
                    }
                    actual => Err(format!(
                        "expected `{operation}` to return a validated emulation preview\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::ResultEmulationAction { operation, kind } => {
                self.finish_operation(&operation)?;
                match self.results.get(&operation) {
                    Some(OperationResult::Preview(Ok(preview)))
                        if preview.emulation.actions.iter().any(|action| {
                            action.kind == kind
                                && action.succeeded
                                && !action.accounts.is_empty()
                                && !action.transaction_hashes.is_empty()
                                && serde_json::from_str::<serde_json::Value>(&action.details_json)
                                    .is_ok_and(|details| details.is_object())
                        }) =>
                    {
                        Ok(())
                    }
                    actual => Err(format!(
                        "expected `{operation}` to expose a successful `{kind}` emulation action with validated accounts, hashes, and JSON details\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::UpdateOutcome { operation, outcome } => {
                self.finish_operation(&operation)?;
                match self.results.get(&operation) {
                    Some(OperationResult::Update(result)) if matches!(result.as_ref(), Ok(update) if update.outcome == outcome) => {
                        Ok(())
                    }
                    actual => Err(format!(
                        "expected `{operation}` to finish with {outcome:?}\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::UpdateAddedItems { operation, count } => {
                self.finish_operation(&operation)?;
                match self.results.get(&operation) {
                    Some(OperationResult::Update(result)) if matches!(result.as_ref(), Ok(update) if update.activity_items_added == count) => {
                        Ok(())
                    }
                    actual => Err(format!(
                        "expected `{operation}` to add {count} activity items\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::UpdateAddedAnyItems { operation } => {
                self.finish_operation(&operation)?;
                match self.results.get(&operation) {
                    Some(OperationResult::Update(result)) if matches!(result.as_ref(), Ok(update) if update.activity_items_added > 0) => {
                        Ok(())
                    }
                    actual => Err(format!(
                        "expected `{operation}` to add activity items\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::Snapshot(expectation) => {
                let snapshot = self.client.snapshot().map_err(|error| error.to_string())?;
                if let Some(expected) = expectation.send_phase
                    && snapshot.send.phase != expected
                {
                    return Err(format!(
                        "expected snapshot send phase {expected:?}\nactual: {:?}",
                        snapshot.send.phase
                    ));
                }
                if let Some(expected) = expectation.account_phase
                    && snapshot.account_resource.phase != expected
                {
                    return Err(format!(
                        "expected account phase {expected:?}\nactual: {:?}",
                        snapshot.account_resource.phase
                    ));
                }
                if let Some(expected) = expectation.activity_phase
                    && snapshot.activity_resource.phase != expected
                {
                    return Err(format!(
                        "expected activity phase {expected:?}\nactual: {:?}",
                        snapshot.activity_resource.phase
                    ));
                }
                if let Some(expected) = expectation.pagination_phase
                    && snapshot.activity_pagination_resource.phase != expected
                {
                    return Err(format!(
                        "expected pagination phase {expected:?}\nactual: {:?}",
                        snapshot.activity_pagination_resource.phase
                    ));
                }
                if let Some(expected) = expectation.activity_count
                    && snapshot.activity.len() != expected
                {
                    return Err(format!(
                        "expected {expected} activity items\nactual: {}",
                        snapshot.activity.len()
                    ));
                }
                if let Some(expected) = expectation.has_more
                    && snapshot.activity_has_more != expected
                {
                    return Err(format!(
                        "expected activity_has_more={expected}\nactual: {}",
                        snapshot.activity_has_more
                    ));
                }
                if let Some(expected) = expectation.account_error
                    && snapshot.account_resource.error.as_ref() != Some(&expected)
                {
                    return Err(format!(
                        "expected account error {expected:#?}\nactual: {:#?}",
                        snapshot.account_resource.error
                    ));
                }
                if let Some(expected) = expectation.activity_error
                    && snapshot.activity_resource.error.as_ref() != Some(&expected)
                {
                    return Err(format!(
                        "expected activity error {expected:#?}\nactual: {:#?}",
                        snapshot.activity_resource.error
                    ));
                }
                if let Some(expected) = expectation.send_error_message
                    && snapshot.send.error_message.as_deref() != Some(expected.as_str())
                {
                    return Err(format!(
                        "expected send error message `{expected}`\nactual: {:?}",
                        snapshot.send.error_message
                    ));
                }
                Ok(())
            }
            Expectation::AccountStatus(expected) => {
                let snapshot = self.client.snapshot().map_err(|error| error.to_string())?;
                let actual = snapshot.account.as_ref().map(|account| account.status);
                if actual == Some(expected) {
                    Ok(())
                } else {
                    Err(format!(
                        "expected account status {expected:?}\nactual: {actual:?}"
                    ))
                }
            }
            Expectation::ActivityPresent => {
                let snapshot = self.client.snapshot().map_err(|error| error.to_string())?;
                if snapshot.activity.is_empty() {
                    Err("expected refreshed activity to contain an item".to_owned())
                } else {
                    Ok(())
                }
            }
            Expectation::RememberActivityCursor(name) => {
                let cursor = self
                    .client
                    .snapshot()
                    .map_err(|error| error.to_string())?
                    .activity_cursor
                    .ok_or_else(|| "expected an activity cursor".to_owned())?;
                self.activity_cursors.insert(name, cursor);
                Ok(())
            }
            Expectation::PaginationUsedCursor(name) => {
                let cursor = self
                    .activity_cursors
                    .get(&name)
                    .ok_or_else(|| format!("activity cursor `{name}` was not remembered"))?;
                let request = self
                    .localnet_http_host
                    .as_ref()
                    .and_then(|host| host.last_activity_request())
                    .ok_or_else(|| "expected a localnet activity request".to_owned())?;
                let parsed = url::Url::parse(&request).map_err(|error| error.to_string())?;
                let query: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
                if query.get("lt") == Some(&cursor.logical_time)
                    && query.get("hash").map(String::as_str) == Some(cursor.hash.as_str())
                {
                    Ok(())
                } else {
                    Err(format!(
                        "expected pagination cursor {cursor:?}\nrequest: {request}"
                    ))
                }
            }
            Expectation::RememberActivity { name, only_new } => {
                let mut ids: HashSet<_> = self
                    .client
                    .snapshot()
                    .map_err(|error| error.to_string())?
                    .activity
                    .into_iter()
                    .map(|item| item.id)
                    .collect();
                if only_new {
                    for known in self.named_activity.values() {
                        ids.retain(|id| !known.contains(id));
                    }
                }
                if ids.is_empty() {
                    return Err(format!("activity set `{name}` is empty"));
                }
                self.named_activity.insert(name, ids);
                Ok(())
            }
            Expectation::ActivityIs(names) => {
                let mut expected = HashSet::new();
                for name in names {
                    let ids = self
                        .named_activity
                        .get(&name)
                        .ok_or_else(|| format!("activity set `{name}` was not remembered"))?;
                    expected.extend(ids.iter().cloned());
                }
                let actual: HashSet<_> = self
                    .client
                    .snapshot()
                    .map_err(|error| error.to_string())?
                    .activity
                    .into_iter()
                    .map(|item| item.id)
                    .collect();
                if actual == expected {
                    Ok(())
                } else {
                    Err(format!(
                        "expected activity sets to match\nexpected: {expected:?}\nactual: {actual:?}"
                    ))
                }
            }
            Expectation::RequestWasCancelled(name) => {
                let cancelled = if let Some(host) = &self.scripted_http_host {
                    host.request_was_cancelled(&name)?
                } else if let Some(host) = &self.localnet_http_host {
                    host.request_was_cancelled(&name)?
                } else {
                    return Err("scenario has no HTTP host".to_owned());
                };
                if cancelled {
                    Ok(())
                } else {
                    Err(format!("request at checkpoint `{name}` was not cancelled"))
                }
            }
            Expectation::RememberRevision(name) => {
                let revision = self
                    .client
                    .snapshot()
                    .map_err(|error| error.to_string())?
                    .revision;
                self.named_revisions.insert(name, revision);
                Ok(())
            }
            Expectation::RevisionIs(name) => {
                let expected = self
                    .named_revisions
                    .get(&name)
                    .copied()
                    .ok_or_else(|| format!("revision `{name}` was not remembered"))?;
                let actual = self
                    .client
                    .snapshot()
                    .map_err(|error| error.to_string())?
                    .revision;
                if actual == expected {
                    Ok(())
                } else {
                    Err(format!(
                        "expected revision `{name}` to remain {expected}, got {actual}"
                    ))
                }
            }
            Expectation::ReturnedSnapshotRevisionIs {
                operation,
                revision,
            } => {
                self.finish_operation(&operation)?;
                let expected = self
                    .named_revisions
                    .get(&revision)
                    .copied()
                    .ok_or_else(|| format!("revision `{revision}` was not remembered"))?;
                match self.results.get(&operation) {
                    Some(OperationResult::Snapshot(result)) if matches!(result.as_ref(), Ok(snapshot) if snapshot.revision == expected) => {
                        Ok(())
                    }
                    actual => Err(format!(
                        "expected `{operation}` to return remembered revision `{revision}` ({expected})\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::ReturnedSnapshotRevisionIsGreaterThan {
                operation,
                after_revision,
            } => {
                self.finish_operation(&operation)?;
                match self.results.get(&operation) {
                    Some(OperationResult::Snapshot(result)) if matches!(result.as_ref(), Ok(snapshot) if snapshot.revision > after_revision) => {
                        Ok(())
                    }
                    actual => Err(format!(
                        "expected `{operation}` to return a revision greater than {after_revision}\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::SecretReadCount(expected) => {
                let actual = self.platform_host.secret_read_count();
                if actual == expected {
                    Ok(())
                } else {
                    Err(format!(
                        "expected protected secret read count {expected}, got {actual}"
                    ))
                }
            }
            Expectation::JournalIsEmpty => {
                if self.platform_host.journal_is_empty() {
                    Ok(())
                } else {
                    Err("expected the durable send journal to remain empty".to_owned())
                }
            }
            Expectation::NoSubmittedMessage => {
                if self.submitted_message().is_none() {
                    Ok(())
                } else {
                    Err("expected no external message submission".to_owned())
                }
            }
            Expectation::SubmittedMessageContainsStateInit => self
                .eventually(|| {
                    Ok(self
                        .submitted_message()
                        .is_some_and(|message| message.contains_state_init))
                })
                .map_err(|message| {
                    format!("expected submitted external message to contain StateInit\n{message}")
                }),
            Expectation::SubmittedMessageUsesMode(expected) => {
                let actual = self
                    .submitted_message()
                    .ok_or_else(|| "expected an external message submission".to_owned())?
                    .send_modes;
                if actual == [expected] {
                    Ok(())
                } else {
                    Err(format!(
                        "expected submitted external message mode [{expected}], actual: {actual:?}"
                    ))
                }
            }
            Expectation::SubmittedMessageHasComment(expected) => {
                let actual = self
                    .submitted_message()
                    .ok_or_else(|| "expected an external message submission".to_owned())?
                    .comment;
                if actual.as_deref() == Some(expected.as_str()) {
                    Ok(())
                } else {
                    Err(format!(
                        "expected submitted message comment {expected:?}, actual: {actual:?}"
                    ))
                }
            }
            Expectation::SubmittedMessagePresent => {
                if self.submitted_message().is_some() {
                    Ok(())
                } else {
                    Err("expected an external message submission".to_owned())
                }
            }
            Expectation::OnChainWallet(expectation) => self
                .localnet_http_host
                .as_ref()
                .ok_or_else(|| {
                    "on-chain expectations require `.given(network().localnet())`".to_owned()
                })?
                .assert_wallet(expectation.active, expectation.seqno),
        }
    }

    fn submitted_message(&self) -> Option<super::host::SubmittedMessage> {
        self.scripted_http_host
            .as_ref()
            .and_then(|host| host.submitted_message())
            .or_else(|| {
                self.localnet_http_host
                    .as_ref()
                    .and_then(|host| host.submitted_message())
            })
    }

    fn finish_operation(&mut self, name: &str) -> Result<(), String> {
        if self.results.contains_key(name) {
            return Ok(());
        }

        let mut running = self
            .operations
            .remove(name)
            .ok_or_else(|| format!("operation `{name}` does not exist"))?;
        let timeout = step_timeout();
        let result = match running.receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                self.operations.insert(name.to_owned(), running);
                return Err(format!(
                    "operation `{name}` did not finish within {timeout:?}"
                ));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(format!("operation `{name}` worker disconnected"));
            }
        };

        if let Some(thread) = running.thread.take()
            && thread.join().is_err()
        {
            return Err(format!("operation `{name}` worker panicked"));
        }
        self.results.insert(name.to_owned(), result);
        Ok(())
    }

    fn eventually(
        &self,
        mut predicate: impl FnMut() -> Result<bool, String>,
    ) -> Result<(), String> {
        let deadline = Instant::now() + step_timeout();
        while Instant::now() < deadline {
            if predicate()? {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(2));
        }

        let snapshot = self.client.snapshot().map_err(|error| error.to_string())?;
        Err(format!("last snapshot: {snapshot:#?}"))
    }
}
