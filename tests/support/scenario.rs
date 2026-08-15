use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use futures::executor::block_on;
use wallet_engine::{
    AccountStatus, ActivityCursor, Network, ProtectedSecretRef, ProviderConfig, ResourcePhase,
    SendPhase, SendRequest, SendResult, WalletClient, WalletClientConfig, WalletClientError,
    WalletHttpHost, WalletOperationOutcome, WalletUpdate,
};

use super::host::{MemoryPlatformHost, ScenarioHttpHost};
use super::localnet::LocalnetHttpHost;

// Signing is CPU-heavy in debug builds. Parallel scenarios can legitimately
// take longer than a single isolated test without indicating a deadlock.
const STEP_TIMEOUT: Duration = Duration::from_secs(60);
const NANOGRAMS_PER_GRAM: u64 = 1_000_000_000;
const TEST_RECORD_ID: &str = "scenario-wallet";
const TEST_SECRET_REF: &str = "wallet:scenario-wallet:mnemonic";
const TESTNET_V5_ADDRESS: &str = "0QA_6fh0aRAkD7n1MNfAUx8TvyCUw2iTQfzVM-0isMze2anN";
const TEST_MNEMONIC: &str = "section garden tomato dinner season dice renew length useful spin trade intact use universe what post spike keen mandate behind concert egg doll rug";

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
    SecretFixture { valid: true }
}

pub(crate) const fn journal() -> JournalFixture {
    JournalFixture {
        conflict_next_write: false,
    }
}

pub(crate) const fn provider() -> ProviderFixture {
    ProviderFixture {
        account_status: 200,
        activity_status: 200,
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
        amount_nanograms: NANOGRAMS_PER_GRAM.to_string(),
    }
}

pub(crate) const fn refresh_wallet() -> UserAction {
    UserAction::Refresh
}

pub(crate) const fn load_more_activity() -> UserAction {
    UserAction::LoadMoreActivity
}

pub(crate) const fn spam_transfers(count: u32) -> UserAction {
    UserAction::SpamTransfers { count }
}

pub(crate) const fn own_address() -> Destination {
    Destination::SelfWallet
}

pub(crate) fn invalid_address() -> Destination {
    Destination::Address("not-a-ton-address".to_owned())
}

pub(crate) fn start(name: impl Into<String>, action: SendAction) -> ActionStep {
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

pub(crate) const fn submitted_message() -> SubmittedMessageExpectation {
    SubmittedMessageExpectation
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
        activity_count: None,
        has_more: None,
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
    valid: bool,
}

impl SecretFixture {
    #[must_use]
    pub(crate) const fn invalid(mut self) -> Self {
        self.valid = false;
        self
    }
}

pub(crate) struct JournalFixture {
    conflict_next_write: bool,
}

pub(crate) struct ProviderFixture {
    account_status: u16,
    activity_status: u16,
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
}

impl JournalFixture {
    #[must_use]
    pub(crate) const fn conflicts_on_next_write(mut self) -> Self {
        self.conflict_next_write = true;
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
    amount_nanograms: String,
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
        self.amount_nanograms = grams(value).nanograms;
        self
    }

    #[must_use]
    pub(crate) fn nanograms(mut self, value: u64) -> Self {
        self.amount_nanograms = value.to_string();
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
    CancelSend,
    Refresh,
    LoadMoreActivity,
    SpamTransfers { count: u32 },
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
}

pub(crate) enum ControlStep {
    ResumeSubmission {
        name: String,
        outcome: SubmissionOutcome,
    },
}

pub(crate) enum Given {
    Wallet(WalletFixture),
    Submission(SubmissionFixture),
    Secret(SecretFixture),
    Journal(JournalFixture),
    Provider(ProviderFixture),
    Activity(ActivityFixture),
    Network(NetworkFixture),
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
    Success {
        operation: String,
    },
    ResultPhase {
        operation: String,
        phase: SendPhase,
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
    SubmittedMessageContainsStateInit,
    OnChainWallet(OnChainWalletExpectation),
}

pub(crate) struct SubmittedMessageExpectation;

impl SubmittedMessageExpectation {
    pub(crate) const fn contains_state_init(self) -> Expectation {
        Expectation::SubmittedMessageContainsStateInit
    }
}

pub(crate) struct OnChainWalletExpectation {
    active: bool,
    seqno: Option<u32>,
}

impl OnChainWalletExpectation {
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

    pub(crate) fn skipped(self) -> Expectation {
        self.outcome(WalletOperationOutcome::Skipped)
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
    activity_count: Option<usize>,
    has_more: Option<bool>,
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
    pub(crate) const fn activity_count(mut self, count: usize) -> Self {
        self.activity_count = Some(count);
        self
    }

    pub(crate) const fn has_more(mut self, has_more: bool) -> Expectation {
        self.has_more = Some(has_more);
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
}

#[derive(Debug)]
enum OperationResult {
    Send(Result<SendResult, WalletClientError>),
    Update(Box<Result<WalletUpdate, WalletClientError>>),
    Unit(Result<(), WalletClientError>),
}

impl ScenarioRunner {
    fn new(name: &str, steps: &[Step]) -> Result<Self, String> {
        let mut wallet = wallet();
        let mut paused_submission = None;
        let mut use_localnet = false;
        let mut valid_secret = true;
        let mut journal_conflict = false;
        let mut resource_statuses = (200, 200);
        let mut pages = Vec::new();

        for step in steps {
            match step {
                Step::Given(Given::Wallet(fixture)) => wallet = fixture.clone(),
                Step::Given(Given::Submission(fixture)) => {
                    paused_submission.clone_from(&fixture.paused);
                }
                Step::Given(Given::Network(NetworkFixture::Localnet)) => use_localnet = true,
                Step::Given(Given::Secret(fixture)) => valid_secret = fixture.valid,
                Step::Given(Given::Journal(fixture)) => {
                    journal_conflict = fixture.conflict_next_write;
                }
                Step::Given(Given::Provider(fixture)) => {
                    resource_statuses = (fixture.account_status, fixture.activity_status);
                }
                Step::Given(Given::Activity(fixture)) => pages.clone_from(&fixture.pages),
                Step::When(_) | Step::Then(_) => break,
            }
        }

        let platform_host = Arc::new(MemoryPlatformHost::default());
        let secret_ref = ProtectedSecretRef {
            value: TEST_SECRET_REF.to_owned(),
        };
        let secret = if valid_secret {
            TEST_MNEMONIC.as_bytes()
        } else {
            b"invalid recovery phrase"
        };
        platform_host.store_test_secret(&secret_ref, secret);
        if journal_conflict {
            platform_host.conflict_next_journal_write();
        }
        let transport = if use_localnet {
            if wallet.status != "uninitialized" {
                return Err("localnet scenarios must start with an uninitialized wallet".to_owned());
            }
            if paused_submission.is_some() {
                return Err("localnet scenarios cannot pause provider submission".to_owned());
            }

            let host = Arc::new(LocalnetHttpHost::start(
                TESTNET_V5_ADDRESS,
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
            host.set_resource_statuses(resource_statuses.0, resource_statuses.1);
            host.set_activity_pages(pages);
            ScenarioTransport {
                client_host: host.clone(),
                scripted_host: Some(host),
                localnet_host: None,
                provider_base_url: "https://testnet.toncenter.com/api/v2".to_owned(),
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
                address: TESTNET_V5_ADDRESS.to_owned(),
                network: Network::Testnet,
                send_validity_seconds: 300,
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
            address: TESTNET_V5_ADDRESS.to_owned(),
            operations: HashMap::new(),
            results: HashMap::new(),
            activity_cursors: HashMap::new(),
            named_activity: HashMap::new(),
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
                let bytes = if secret.valid {
                    TEST_MNEMONIC.as_bytes()
                } else {
                    b"invalid recovery phrase"
                };
                self.platform_host
                    .store_test_secret(&self.secret_ref, bytes);
                Ok(())
            }
            Given::Journal(journal) => {
                if journal.conflict_next_write {
                    self.platform_host.conflict_next_journal_write();
                }
                Ok(())
            }
            Given::Provider(provider) => {
                let host = self.scripted_http_host.as_ref().ok_or_else(|| {
                    "localnet scenarios cannot script provider failures".to_owned()
                })?;
                host.set_resource_statuses(provider.account_status, provider.activity_status);
                Ok(())
            }
            Given::Activity(activity) => {
                self.scripted_http_host
                    .as_ref()
                    .ok_or_else(|| "localnet scenarios cannot script activity".to_owned())?
                    .set_activity_pages(activity.pages);
                Ok(())
            }
            Given::Network(NetworkFixture::Localnet) => Ok(()),
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
                    UserAction::Send(action) => {
                        let destination = match action.destination {
                            Destination::SelfWallet => self.address.clone(),
                            Destination::Address(address) => address,
                        };
                        let request = SendRequest {
                            operation_id: format!("{name}-operation"),
                            destination,
                            amount_nanograms: action.amount_nanograms,
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
                    UserAction::Refresh => std::thread::spawn(move || {
                        let result = block_on(client.refresh());
                        let _ = sender.send(OperationResult::Update(Box::new(result)));
                    }),
                    UserAction::LoadMoreActivity => std::thread::spawn(move || {
                        let result = block_on(client.load_more_activity());
                        let _ = sender.send(OperationResult::Update(Box::new(result)));
                    }),
                    UserAction::SpamTransfers { count } => {
                        let address = self.address.clone();
                        let secret_ref = self.secret_ref.clone();
                        let operation_prefix = name.clone();
                        std::thread::spawn(move || {
                            let result = (0..count).try_for_each(|index| {
                                let request = SendRequest {
                                    operation_id: format!("{operation_prefix}-operation-{index}"),
                                    destination: address.clone(),
                                    amount_nanograms: "1".to_owned(),
                                    secret_ref: secret_ref.clone(),
                                };
                                block_on(client.send(request)).map(|_| ())
                            });
                            let _ = sender.send(OperationResult::Unit(result));
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
                    Some(
                        OperationResult::Send(Err(actual)) | OperationResult::Unit(Err(actual)),
                    ) if actual == &expected => Ok(()),
                    actual => Err(format!(
                        "expected `{operation}` to return {expected:?}\nactual: {actual:?}"
                    )),
                }
            }
            Expectation::Success { operation } => {
                self.finish_operation(&operation)?;
                match self.results.get(&operation) {
                    Some(OperationResult::Unit(Ok(()))) => Ok(()),
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
            Expectation::SubmittedMessageContainsStateInit => self
                .eventually(|| {
                    Ok(self
                        .submitted_message()
                        .is_some_and(|message| message.contains_state_init))
                })
                .map_err(|message| {
                    format!("expected submitted external message to contain StateInit\n{message}")
                }),
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
        let result = match running.receiver.recv_timeout(STEP_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                self.operations.insert(name.to_owned(), running);
                return Err(format!(
                    "operation `{name}` did not finish within {STEP_TIMEOUT:?}"
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
        let deadline = Instant::now() + STEP_TIMEOUT;
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
