use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use futures::executor::block_on;
use wallet_engine::{
    Network, ProtectedSecretRef, ProviderConfig, SendPhase, SendRequest, SendResult, WalletClient,
    WalletClientConfig, WalletClientError,
};

use super::host::{MemoryPlatformHost, ScenarioHttpHost};

// Signing is CPU-heavy in debug builds. Parallel scenarios can legitimately
// take longer than a single isolated test without indicating a deadlock.
const STEP_TIMEOUT: Duration = Duration::from_secs(15);
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
        sync_utime: 1_800_000_000,
    }
}

pub(crate) const fn submission() -> SubmissionFixture {
    SubmissionFixture { paused: None }
}

pub(crate) fn send() -> SendAction {
    SendAction {
        destination: Destination::SelfWallet,
        amount_nanograms: NANOGRAMS_PER_GRAM.to_string(),
    }
}

pub(crate) const fn own_address() -> Destination {
    Destination::SelfWallet
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

pub(crate) fn result(name: impl Into<String>) -> ResultExpectation {
    ResultExpectation { name: name.into() }
}

pub(crate) const fn submitted_message() -> SubmittedMessageExpectation {
    SubmittedMessageExpectation
}

pub(crate) const fn snapshot() -> SnapshotExpectation {
    SnapshotExpectation { send_phase: None }
}

#[derive(Clone)]
pub(crate) struct WalletFixture {
    pub(super) status: &'static str,
    pub(super) balance_nanograms: String,
    pub(super) seqno: u32,
    pub(super) sync_utime: u64,
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
}

impl SendAction {
    #[must_use]
    pub(crate) const fn to(mut self, destination: Destination) -> Self {
        self.destination = destination;
        self
    }

    #[must_use]
    pub(crate) fn grams(mut self, value: u64) -> Self {
        self.amount_nanograms = grams(value).nanograms;
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
    ResultPhase {
        operation: String,
        phase: SendPhase,
    },
    Snapshot(SnapshotExpectation),
    SubmittedMessageContainsStateInit,
}

pub(crate) struct SubmittedMessageExpectation;

impl SubmittedMessageExpectation {
    pub(crate) const fn contains_state_init(self) -> Expectation {
        Expectation::SubmittedMessageContainsStateInit
    }
}

pub(crate) struct ResultExpectation {
    name: String,
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
}

impl SnapshotExpectation {
    #[must_use]
    pub(crate) const fn send_phase(mut self, phase: SendPhase) -> Expectation {
        self.send_phase = Some(phase);
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

struct ScenarioRunner {
    name: String,
    client: Arc<WalletClient>,
    http_host: Arc<ScenarioHttpHost>,
    secret_ref: ProtectedSecretRef,
    address: String,
    operations: HashMap<String, RunningOperation>,
    results: HashMap<String, OperationResult>,
}

#[derive(Debug)]
enum OperationResult {
    Send(Result<SendResult, WalletClientError>),
    Unit(Result<(), WalletClientError>),
}

impl ScenarioRunner {
    fn new(name: &str, steps: &[Step]) -> Result<Self, String> {
        let mut wallet = wallet();
        let mut paused_submission = None;

        for step in steps {
            match step {
                Step::Given(Given::Wallet(fixture)) => wallet = fixture.clone(),
                Step::Given(Given::Submission(fixture)) => {
                    paused_submission.clone_from(&fixture.paused);
                }
                Step::When(_) | Step::Then(_) => break,
            }
        }

        let platform_host = Arc::new(MemoryPlatformHost::default());
        let secret_ref = ProtectedSecretRef {
            value: TEST_SECRET_REF.to_owned(),
        };
        platform_host.store_test_secret(&secret_ref, TEST_MNEMONIC.as_bytes());
        let http_host = Arc::new(ScenarioHttpHost::new(wallet, paused_submission));
        let client = WalletClient::new(
            WalletClientConfig {
                record_id: TEST_RECORD_ID.to_owned(),
                address: TESTNET_V5_ADDRESS.to_owned(),
                network: Network::Testnet,
                send_validity_seconds: 300,
                providers: ProviderConfig {
                    toncenter_base_url: "https://testnet.toncenter.com/api/v2".to_owned(),
                },
            },
            http_host.clone(),
            platform_host,
        )
        .map_err(|error| format!("scenario `{name}` could not create its client: {error}"))?;

        Ok(Self {
            name: name.to_owned(),
            client,
            http_host,
            secret_ref,
            address: TESTNET_V5_ADDRESS.to_owned(),
            operations: HashMap::new(),
            results: HashMap::new(),
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
                self.http_host.set_wallet(wallet);
                Ok(())
            }
            Given::Submission(submission) => {
                let name = submission
                    .paused
                    .ok_or_else(|| "submission fixture has no behavior".to_owned())?;
                self.http_host.pause_submission(name);
                Ok(())
            }
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
            When::Control(ControlStep::ResumeSubmission { name, outcome }) => {
                self.http_host.resume_submission(&name, outcome)
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
                    Some(
                        OperationResult::Send(Err(actual)) | OperationResult::Unit(Err(actual)),
                    ) if actual == &expected => Ok(()),
                    actual => Err(format!(
                        "expected `{operation}` to return {expected:?}\nactual: {actual:?}"
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
                Ok(())
            }
            Expectation::SubmittedMessageContainsStateInit => {
                match self.http_host.submitted_message() {
                    Some(message) if message.contains_state_init => Ok(()),
                    Some(_) => {
                        Err("expected submitted external message to contain StateInit".into())
                    }
                    None => Err("expected sendBoc to contain a submitted message".into()),
                }
            }
        }
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
