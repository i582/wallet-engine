use std::{
    env,
    error::Error,
    io,
    net::TcpListener,
    num::{NonZeroU32, NonZeroUsize},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use futures::executor::block_on;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Map;
use ton::block_tlb::StateInit;
use ton::ton_core::traits::tlb::TLB as _;
use ton::ton_wallet::{WALLET_SUBWALLET_ID_DEFAULT_TESTNET, WalletVersion};
use ton_connect_client::{IncomingRequest, TonConnectClient, TonConnectClientConfig};
use ton_connect_core::{
    ConnectEvent, ConnectEventPayload, ConnectItem, ConnectItemReply, ConnectLink, DeviceInfo,
    DevicePlatform, Ed25519PublicKey, Feature, FriendlyAddress, HeartbeatMode, HttpBridgeUrl,
    KnownAppRequest, NetworkId, PreparedBridgePost, RawMessage, RequestContextError,
    ReturnStrategy, RpcErrorCode, SendTransactionFeature, SessionCrypto, SignMessageFeature,
    SignMessageResult as ProtocolSignMessageResult, TonAddressItemReply, TransactionPayload,
    WalletResponse, WalletResponseError, WalletResponseSuccess, WalletResult, WalletStateInit,
};
use ton_core::cell::TonCell;
use wallet_engine::{
    Boc, ImportWalletRequest, Network, NonEmptyString, ProviderConfig, SendAmount, SendExpiration,
    SendIntent, SendMessage, SendMessageBody, SendPhase, SendRequest, SignMessageRequest,
    TonAddressString, TonConnectAccountInfo, TonConnectDevice, TonConnectDevicePlatform,
    TonConnectIncomingRequest, TonConnectRpcErrorCode, TonConnectSession, TonConnectSessionConfig,
    WalletClient, WalletClientConfig, WalletLifecycle, ton_connect_session_from_link,
};

use super::{host::MemoryPlatformHost, localnet::LocalnetHttpHost, test_wallet};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const PROCESS_START_TIMEOUT: Duration = Duration::from_secs(10);
const EVENT_TIMEOUT: Duration = Duration::from_secs(10);
const WALLET_ADDRESS: &str = "{wallet_address}";
const DEPLOYMENT_ADDRESS: &str = "{deployment_address}";
const DEPLOYMENT_STATE_INIT: &str = "{deployment_state_init}";
const TEST_SIGNED_BOC: &str = "te6ccgEBAQEAAgAAAA==";
const TON_TESTNET_NETWORK_ID: &str = "-3";
const ENGINE_RECORD_ID: &str = "ton-connect-localnet-wallet";

/// Starts a named Given/When/Then scenario and records its steps for deferred execution.
pub(crate) fn scenario(name: impl Into<String>) -> Scenario {
    Scenario {
        name: name.into(),
        steps: Vec::new(),
    }
}

/// Starts configuration of the official bridge process fixture.
pub(crate) const fn bridge() -> BridgeFixtureBuilder {
    BridgeFixtureBuilder
}

/// Starts configuration of the headless TypeScript dApp fixture.
pub(crate) const fn dapp() -> DappFixtureBuilder {
    DappFixtureBuilder
}

const ACTOR_ORIGIN: &str = "{actor_origin}";

/// Starts configuration of the deterministic Rust wallet fixture.
pub(crate) const fn wallet() -> WalletFixtureBuilder {
    WalletFixtureBuilder
}

/// Commands the dApp actor to create a TON Connect link and begin listening for a wallet.
pub(crate) const fn dapp_connects() -> When {
    When::DappConnects
}

/// Commands the wallet actor to approve the pending connection with its configured account.
pub(crate) const fn wallet_approves_connect() -> When {
    When::WalletApprovesConnect
}

/// Commands the connected dApp to send a disconnect request through the bridge.
pub(crate) const fn dapp_disconnects() -> When {
    When::DappDisconnects
}

/// Commands the wallet to consume and acknowledge the dApp's disconnect request.
pub(crate) const fn wallet_answers_disconnect() -> When {
    When::WalletAnswersDisconnect
}

/// Commands the connected dApp to send the configured transaction through the SDK.
pub(crate) const fn dapp_sends_transaction(transaction: DappTransactionConfig) -> When {
    When::DappSendsTransaction(transaction)
}

/// Commands the connected dApp to request an internal-message signature without broadcasting it.
pub(crate) const fn dapp_signs_message(transaction: DappTransactionConfig) -> When {
    When::DappSignsMessage(transaction)
}

/// Commands the wallet to assert the pending message and return a signed `BoC`.
pub(crate) const fn wallet_approves_transaction(expected_message: DappTransactionMessage) -> When {
    When::WalletAnswersTransaction(
        TransactionDecision::Approve,
        DappTransactionMessages::One(expected_message),
    )
}

/// Commands the wallet to assert an exact two-message batch and return a signed `BoC`.
pub(crate) const fn wallet_approves_transaction_messages(
    first: DappTransactionMessage,
    second: DappTransactionMessage,
) -> When {
    When::WalletAnswersTransaction(
        TransactionDecision::Approve,
        DappTransactionMessages::Two(first, second),
    )
}

/// Commands the protocol wallet to validate one `signMessage` payload and return a signed BOC.
pub(crate) const fn wallet_signs_message(expected_message: DappTransactionMessage) -> When {
    When::WalletSignsMessage(DappTransactionMessages::One(expected_message))
}

/// Commands the protocol wallet to validate an ordered two-message signing request.
pub(crate) const fn wallet_signs_message_messages(
    first: DappTransactionMessage,
    second: DappTransactionMessage,
) -> When {
    When::WalletSignsMessage(DappTransactionMessages::Two(first, second))
}

/// Commands wallet-engine to create a durable internal-signed message without submitting it.
pub(crate) const fn wallet_signs_message_on_localnet(
    expected_message: DappTransactionMessage,
) -> When {
    When::WalletSignsMessageOnLocalnet(DappTransactionMessages::One(expected_message))
}

/// Starts configuration of the independent localnet relayer action.
pub(crate) const fn relayer_submits_signed_message() -> RelayerSubmissionBuilder {
    RelayerSubmissionBuilder
}

pub(crate) struct RelayerSubmissionBuilder;

impl RelayerSubmissionBuilder {
    /// Attaches this TON value to the signed internal request and submits it from the relayer wallet.
    pub(crate) const fn with_attached_value_nanograms(self, value: &'static str) -> When {
        let Self = self;
        When::RelayerSubmitsSignedMessage(value)
    }
}

/// Commands wallet-engine to sign, submit, and acknowledge one TON Connect transaction on localnet.
pub(crate) const fn wallet_executes_transaction_on_localnet(
    expected_message: DappTransactionMessage,
) -> When {
    When::WalletExecutesTransactionOnLocalnet(DappTransactionMessages::One(expected_message))
}

/// Commands the wallet to assert the pending message and reject it with protocol error 300.
pub(crate) const fn wallet_rejects_transaction(expected_message: DappTransactionMessage) -> When {
    When::WalletAnswersTransaction(
        TransactionDecision::UserReject,
        DappTransactionMessages::One(expected_message),
    )
}

/// Commands the wallet to assert that the pending transaction is expired and return error 1.
pub(crate) const fn wallet_rejects_expired_transaction(
    expected_message: DappTransactionMessage,
) -> When {
    When::WalletAnswersTransaction(
        TransactionDecision::Expired,
        DappTransactionMessages::One(expected_message),
    )
}

/// Commands the wallet to verify a mismatched `from` address and return protocol error 1.
pub(crate) const fn wallet_rejects_transaction_for_account_mismatch(
    expected_message: DappTransactionMessage,
) -> When {
    When::WalletAnswersTransaction(
        TransactionDecision::AccountMismatch,
        DappTransactionMessages::One(expected_message),
    )
}

/// Commands the wallet to assert the pending message and return unknown-app error 100.
pub(crate) const fn wallet_rejects_transaction_from_unknown_app(
    expected_message: DappTransactionMessage,
) -> When {
    When::WalletAnswersTransaction(
        TransactionDecision::UnknownApp,
        DappTransactionMessages::One(expected_message),
    )
}

/// Expects a complete protocol-v2 `tc` link with exactly one copy of every
/// required query parameter, valid client and trace IDs, the `back` return
/// strategy, no embedded request or extensions, and a connect request whose
/// manifest URL and `ton_addr` network match the configured dApp.
pub(crate) const fn connect_link_created() -> Expectation {
    Expectation::ConnectLinkCreated
}

/// Expects the connect request to reference the configured manifest, every
/// served manifest field to match the dApp fixture, and its icon to be reachable.
pub(crate) const fn manifest_available() -> Expectation {
    Expectation::ManifestAvailable
}

/// Expects the dApp to observe the configured runtime values, exact wallet
/// account and device capabilities, no connector error, and ordered connect events.
pub(crate) const fn dapp_connected() -> Expectation {
    Expectation::DappConnected
}

/// Expects the dApp and wallet to finish the disconnect handshake and clear session state.
pub(crate) const fn dapp_disconnected() -> Expectation {
    Expectation::DappDisconnected
}

/// Expects the SDK to reject a wallet account from another network with a
/// structured `WalletWrongNetworkError` containing both expected and actual IDs.
pub(crate) const fn dapp_rejected_wrong_network() -> Expectation {
    Expectation::DappRejectedWrongNetwork
}

/// Expects the dApp to receive the exact signed `BoC` for its pending transaction.
pub(crate) const fn dapp_received_transaction_success() -> Expectation {
    Expectation::DappReceivedTransactionSuccess
}

/// Expects the SDK to return the exact internal BOC produced by `signMessage`.
pub(crate) const fn dapp_received_sign_message_success() -> Expectation {
    Expectation::DappReceivedSignMessageSuccess
}

/// Expects the dApp SDK to surface the wallet's rejection as `UserRejectsError`.
pub(crate) const fn dapp_received_transaction_rejection() -> Expectation {
    Expectation::DappReceivedTransactionRejection
}

/// Expects the SDK to reject a transaction for a network other than the connected account.
pub(crate) const fn dapp_rejected_transaction_wrong_network() -> Expectation {
    Expectation::DappRejectedTransactionPreflight(TransactionPreflightRejection::WrongNetwork)
}

/// Expects the SDK to reject a transaction that exceeds the wallet's advertised message limit.
pub(crate) const fn dapp_rejected_transaction_for_message_limit() -> Expectation {
    Expectation::DappRejectedTransactionPreflight(TransactionPreflightRejection::MessageLimit)
}

/// Expects the SDK to reject extra currencies that the wallet did not advertise.
pub(crate) const fn dapp_rejected_transaction_for_extra_currency() -> Expectation {
    Expectation::DappRejectedTransactionPreflight(TransactionPreflightRejection::ExtraCurrency)
}

/// Expects a wallet error 1 to become the SDK's high-level bad-request error.
pub(crate) const fn dapp_received_transaction_bad_request() -> Expectation {
    Expectation::DappReceivedTransactionBadRequest
}

/// Expects an account-mismatch wallet error to become the SDK's high-level bad-request error.
pub(crate) const fn dapp_received_transaction_account_mismatch() -> Expectation {
    Expectation::DappReceivedTransactionAccountMismatch
}

/// Expects wallet error 100 to become the SDK's high-level unknown-app error.
pub(crate) const fn dapp_received_transaction_unknown_app() -> Expectation {
    Expectation::DappReceivedTransactionUnknownApp
}

/// Starts an on-chain expectation for the account derived from deployment `StateInit`.
pub(crate) const fn deployment_target() -> DeploymentTargetExpectationBuilder {
    DeploymentTargetExpectationBuilder
}

/// Starts an on-chain expectation for the wallet that signed the TON Connect request.
pub(crate) const fn source_wallet_account() -> OnChainAccountExpectationBuilder {
    OnChainAccountExpectationBuilder::new(OnChainAccountTarget::SourceWallet)
}

pub(crate) struct DeploymentTargetExpectationBuilder;

impl DeploymentTargetExpectationBuilder {
    /// Requires the deterministic target to have no active account before submission.
    pub(crate) const fn absent(self) -> Expectation {
        let Self = self;
        Expectation::DeploymentTargetAbsent
    }

    /// Requires the deterministic target to become active after the deploy message.
    pub(crate) const fn active(self) -> OnChainAccountExpectationBuilder {
        let Self = self;
        OnChainAccountExpectationBuilder::new(OnChainAccountTarget::Deployment).active()
    }
}

pub(crate) struct OnChainAccountExpectationBuilder {
    target: OnChainAccountTarget,
    state: &'static str,
    balance_range: Option<(&'static str, &'static str)>,
}

impl OnChainAccountExpectationBuilder {
    /// Creates an account assertion with no assumed state or balance requirement.
    const fn new(target: OnChainAccountTarget) -> Self {
        Self {
            target,
            state: "",
            balance_range: None,
        }
    }

    /// Requires the selected localnet account to be deployed and active.
    #[must_use]
    pub(crate) const fn active(mut self) -> Self {
        self.state = "active";
        self
    }

    /// Requires the account balance to remain inside an inclusive nanogram range.
    #[must_use]
    pub(crate) const fn balance_between(
        mut self,
        minimum: &'static str,
        maximum: &'static str,
    ) -> Self {
        self.balance_range = Some((minimum, maximum));
        self
    }

    /// Finalizes the account expectation with an exact wallet `seqno` value.
    pub(crate) const fn seqno(self, seqno: u32) -> Expectation {
        Expectation::OnChainAccount(OnChainAccountExpectation {
            target: self.target,
            state: self.state,
            balance_range: self.balance_range,
            seqno,
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) enum OnChainAccountTarget {
    Deployment,
    SourceWallet,
}

pub(crate) struct OnChainAccountExpectation {
    target: OnChainAccountTarget,
    state: &'static str,
    balance_range: Option<(&'static str, &'static str)>,
    seqno: u32,
}

pub(crate) struct Scenario {
    name: String,
    steps: Vec<Step>,
}

impl Scenario {
    /// Adds one process or actor fixture that must be available to later scenario steps.
    #[must_use]
    pub(crate) fn given(mut self, fixture: impl Into<Given>) -> Self {
        self.steps.push(Step::Given(fixture.into()));
        self
    }

    /// Adds an actor action to the ordered scenario program.
    #[must_use]
    pub(crate) fn when(mut self, action: When) -> Self {
        self.steps.push(Step::When(action));
        self
    }

    /// Adds an observable postcondition to the ordered scenario program.
    #[must_use]
    pub(crate) fn then(mut self, expectation: Expectation) -> Self {
        self.steps.push(Step::Then(expectation));
        self
    }

    /// Executes every recorded step, attaching the scenario name and step number to failures.
    pub(crate) fn run(self) -> TestResult {
        ScenarioRunner::new(self.name)?.run(self.steps)
    }
}

enum Step {
    Given(Given),
    When(When),
    Then(Expectation),
}

pub(crate) enum Given {
    Bridge(BridgeFixture),
    Dapp(DappFixture),
    Wallet(WalletFixture),
}

pub(crate) enum When {
    DappConnects,
    WalletApprovesConnect,
    DappDisconnects,
    WalletAnswersDisconnect,
    DappSendsTransaction(DappTransactionConfig),
    DappSignsMessage(DappTransactionConfig),
    WalletAnswersTransaction(TransactionDecision, DappTransactionMessages),
    WalletExecutesTransactionOnLocalnet(DappTransactionMessages),
    WalletSignsMessage(DappTransactionMessages),
    WalletSignsMessageOnLocalnet(DappTransactionMessages),
    RelayerSubmitsSignedMessage(&'static str),
}

pub(crate) enum Expectation {
    ConnectLinkCreated,
    ManifestAvailable,
    DappConnected,
    DappDisconnected,
    DappRejectedWrongNetwork,
    DappReceivedTransactionSuccess,
    DappReceivedSignMessageSuccess,
    DappReceivedTransactionRejection,
    DappRejectedTransactionPreflight(TransactionPreflightRejection),
    DappReceivedTransactionBadRequest,
    DappReceivedTransactionAccountMismatch,
    DappReceivedTransactionUnknownApp,
    DeploymentTargetAbsent,
    OnChainAccount(OnChainAccountExpectation),
}

#[derive(Clone, Copy)]
pub(crate) enum TransactionDecision {
    Approve,
    UserReject,
    Expired,
    AccountMismatch,
    UnknownApp,
}

#[derive(Clone, Copy)]
pub(crate) enum TransactionPreflightRejection {
    WrongNetwork,
    MessageLimit,
    ExtraCurrency,
}

#[derive(Clone)]
pub(crate) struct BridgeFixture {
    storage: BridgeStorage,
}

#[derive(Clone, Copy)]
enum BridgeStorage {
    Memory,
}

pub(crate) struct BridgeFixtureBuilder;

impl BridgeFixtureBuilder {
    /// Selects the official Go `bridge3` implementation.
    #[must_use]
    pub(crate) const fn official(self) -> Self {
        self
    }

    /// Uses the bridge's in-memory backend so every scenario starts without persisted sessions.
    #[must_use]
    pub(crate) const fn in_memory(self) -> BridgeFixture {
        let Self = self;
        BridgeFixture {
            storage: BridgeStorage::Memory,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DappConfig {
    manifest_url: &'static str,
    universal_link: &'static str,
    in_network: Option<&'static str>,
    manifest: DappManifestConfig,
}

impl DappConfig {
    /// Defines the long-lived dApp configuration and the manifest served by its actor.
    #[must_use]
    pub(crate) const fn new(manifest_url: &'static str, manifest: DappManifestConfig) -> Self {
        Self {
            manifest_url,
            universal_link: "tc://",
            in_network: None,
            manifest,
        }
    }

    /// Sets the universal/deep link prefix used when the SDK creates a connection URL.
    #[must_use]
    pub(crate) const fn universal_link(mut self, universal_link: &'static str) -> Self {
        self.universal_link = universal_link;
        self
    }

    /// Restricts connection to one TON network ID and includes it in the `ton_addr` request.
    #[must_use]
    pub(crate) const fn in_network(mut self, network: &'static str) -> Self {
        self.in_network = Some(network);
        self
    }

    /// Replaces the scenario origin placeholder and injects the runtime bridge URL.
    fn render(&self, actor_origin: &str, bridge_url: &str) -> RenderedDappConfig {
        RenderedDappConfig {
            bridge_url: bridge_url.to_owned(),
            manifest_url: render_actor_origin(self.manifest_url, actor_origin),
            universal_link: self.universal_link.to_owned(),
            in_network: self.in_network.map(str::to_owned),
            manifest: RenderedDappManifestConfig {
                url: render_actor_origin(self.manifest.url, actor_origin),
                name: self.manifest.name.to_owned(),
                icon_url: render_actor_origin(self.manifest.icon_url, actor_origin),
            },
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DappManifestConfig {
    url: &'static str,
    name: &'static str,
    icon_url: &'static str,
}

impl DappManifestConfig {
    /// Defines the exact manifest fields that the local HTTPS dApp actor serves.
    #[must_use]
    pub(crate) const fn new(url: &'static str, name: &'static str, icon_url: &'static str) -> Self {
        Self {
            url,
            name,
            icon_url,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct DappTransactionConfig {
    validity: DappTransactionValidity,
    network: &'static str,
    from: Option<&'static str>,
    messages: DappTransactionMessages,
}

#[derive(Clone, Copy)]
pub(crate) struct DappTransactionMessage {
    destination: &'static str,
    amount: &'static str,
    payload: Option<&'static str>,
    state_init: Option<&'static str>,
    extra_currency: Option<DappExtraCurrency>,
}

#[derive(Clone, Copy)]
enum DappTransactionValidity {
    Future(u64),
    Past(u64),
}

#[derive(Clone, Copy)]
pub(crate) enum DappTransactionMessages {
    One(DappTransactionMessage),
    Two(DappTransactionMessage, DappTransactionMessage),
}

#[derive(Clone, Copy)]
struct DappExtraCurrency {
    id: u32,
    amount: &'static str,
}

struct RenderedDappTransaction {
    sdk_request: serde_json::Value,
    wire_request: TransactionPayload,
}

impl DappTransactionConfig {
    /// Defines a raw one-message transaction with a five-minute validity window.
    #[must_use]
    pub(crate) const fn new(network: &'static str, message: DappTransactionMessage) -> Self {
        Self {
            validity: DappTransactionValidity::Future(300),
            network,
            from: None,
            messages: DappTransactionMessages::One(message),
        }
    }

    /// Overrides how long the generated transaction remains valid after the action starts.
    #[must_use]
    pub(crate) const fn valid_for_seconds(mut self, seconds: u64) -> Self {
        self.validity = DappTransactionValidity::Future(seconds);
        self
    }

    /// Makes the generated validity timestamp precede the action by the selected duration.
    #[must_use]
    pub(crate) const fn expired_seconds_ago(mut self, seconds: u64) -> Self {
        self.validity = DappTransactionValidity::Past(seconds);
        self
    }

    /// Overrides the connected account used as the transaction's fixed `from` address.
    #[must_use]
    pub(crate) const fn from(mut self, from: &'static str) -> Self {
        self.from = Some(from);
        self
    }

    /// Adds a second raw message for capability-limit scenarios.
    #[must_use]
    pub(crate) const fn and_message(mut self, message: DappTransactionMessage) -> Self {
        self.messages = match self.messages {
            DappTransactionMessages::One(first) | DappTransactionMessages::Two(first, _) => {
                DappTransactionMessages::Two(first, message)
            }
        };
        self
    }

    /// Resolves the wallet-address placeholder and timestamps the transaction for this run.
    fn render(&self, wallet: &WalletFixture) -> TestResult<RenderedDappTransaction> {
        let account = wallet_account(wallet)?;
        let from = self
            .from
            .map_or_else(|| account.address.to_string(), str::to_owned);
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let valid_until = match self.validity {
            DappTransactionValidity::Future(seconds) => now
                .checked_add(seconds)
                .ok_or_else(|| failure("transaction validity timestamp overflow"))?,
            DappTransactionValidity::Past(seconds) => now
                .checked_sub(seconds)
                .ok_or_else(|| failure("transaction validity timestamp underflow"))?,
        };
        let messages = self.messages.render(wallet)?;

        let mut wire_transaction = Map::new();
        let _ = wire_transaction.insert("valid_until".to_owned(), valid_until.into());
        let _ = wire_transaction.insert("network".to_owned(), self.network.into());
        let _ = wire_transaction.insert("from".to_owned(), from.clone().into());
        let _ = wire_transaction.insert("messages".to_owned(), serde_json::to_value(&messages)?);
        let wire_request = serde_json::from_value(serde_json::Value::Object(wire_transaction))?;

        let mut sdk_messages = serde_json::to_value(messages)?;
        let sdk_messages = sdk_messages
            .as_array_mut()
            .ok_or_else(|| failure("serialized transaction messages are not an array"))?;
        for sdk_message in sdk_messages.iter_mut() {
            if let Some(object) = sdk_message.as_object_mut()
                && let Some(extra_currency) = object.remove("extra_currency")
            {
                let _ = object.insert("extraCurrency".to_owned(), extra_currency);
            }
        }
        let mut sdk_request = Map::new();
        let _ = sdk_request.insert("validUntil".to_owned(), valid_until.into());
        let _ = sdk_request.insert("network".to_owned(), self.network.into());
        let _ = sdk_request.insert("from".to_owned(), from.into());
        let _ = sdk_request.insert(
            "messages".to_owned(),
            serde_json::Value::Array(sdk_messages.clone()),
        );
        Ok(RenderedDappTransaction {
            sdk_request: serde_json::Value::Object(sdk_request),
            wire_request,
        })
    }
}

impl DappTransactionMessages {
    /// Renders every expected message in order using the active wallet fixture.
    fn render(&self, wallet: &WalletFixture) -> TestResult<Vec<RawMessage>> {
        match self {
            Self::One(message) => Ok(vec![message.render(wallet)?]),
            Self::Two(first, second) => Ok(vec![first.render(wallet)?, second.render(wallet)?]),
        }
    }
}

impl DappTransactionMessage {
    /// Defines one raw TON transfer message expected by both the dApp and wallet actors.
    #[must_use]
    pub(crate) const fn new(destination: &'static str, amount: &'static str) -> Self {
        Self {
            destination,
            amount,
            payload: None,
            state_init: None,
            extra_currency: None,
        }
    }

    /// Adds a base64-encoded one-cell body to the expected transfer message.
    #[must_use]
    pub(crate) const fn payload(mut self, payload: &'static str) -> Self {
        self.payload = Some(payload);
        self
    }

    /// Adds a base64-encoded one-cell `StateInit` to the expected transfer message.
    #[must_use]
    pub(crate) const fn state_init(mut self, state_init: &'static str) -> Self {
        self.state_init = Some(state_init);
        self
    }

    /// Adds one extra-currency amount to the expected raw transfer message.
    #[must_use]
    pub(crate) const fn extra_currency(mut self, id: u32, amount: &'static str) -> Self {
        self.extra_currency = Some(DappExtraCurrency { id, amount });
        self
    }

    /// Resolves dynamic placeholders and validates the message with the production wire type.
    fn render(&self, wallet: &WalletFixture) -> TestResult<RawMessage> {
        let account = wallet_account(wallet)?;
        let destination = FriendlyAddress::from_raw(
            account.address,
            true,
            wallet.network == TON_TESTNET_NETWORK_ID,
        )?;
        let deployment = (self.destination.contains(DEPLOYMENT_ADDRESS)
            || self
                .state_init
                .is_some_and(|value| value.contains(DEPLOYMENT_STATE_INIT)))
        .then(test_deployment_target)
        .transpose()?;
        let destination = self
            .destination
            .replace(WALLET_ADDRESS, destination.as_str());
        let destination = deployment.as_ref().map_or_else(
            || destination.clone(),
            |target| destination.replace(DEPLOYMENT_ADDRESS, target.address.as_str()),
        );
        let mut message = Map::new();
        let _ = message.insert("address".to_owned(), destination.into());
        let _ = message.insert("amount".to_owned(), self.amount.into());
        if let Some(payload) = self.payload {
            let _ = message.insert("payload".to_owned(), payload.into());
        }
        if let Some(state_init) = self.state_init {
            let state_init = deployment.as_ref().map_or_else(
                || state_init.to_owned(),
                |target| state_init.replace(DEPLOYMENT_STATE_INIT, &target.state_init),
            );
            let _ = message.insert("stateInit".to_owned(), state_init.into());
        }
        if let Some(extra_currency) = self.extra_currency {
            let _ = message.insert(
                "extra_currency".to_owned(),
                serde_json::json!({ extra_currency.id.to_string(): extra_currency.amount }),
            );
        }
        Ok(serde_json::from_value(serde_json::Value::Object(message))?)
    }
}

#[derive(Clone)]
pub(crate) struct DappFixture {
    config: DappConfig,
}

pub(crate) struct DappFixtureBuilder;

impl DappFixtureBuilder {
    /// Finalizes the dApp fixture with one complete, reusable configuration value.
    #[must_use]
    pub(crate) const fn config(self, config: DappConfig) -> DappFixture {
        let Self = self;
        DappFixture { config }
    }
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderedDappConfig {
    bridge_url: String,
    manifest_url: String,
    universal_link: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    in_network: Option<String>,
    manifest: RenderedDappManifestConfig,
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderedDappManifestConfig {
    url: String,
    name: String,
    icon_url: String,
}

#[derive(Clone)]
pub(crate) struct WalletFixture {
    network: String,
    max_messages: u32,
    extra_currency_supported: bool,
    backend: WalletBackend,
}

#[derive(Clone)]
enum WalletBackend {
    Protocol,
    EngineLocalnet { balance_nanograms: &'static str },
}

pub(crate) struct WalletFixtureBuilder;

pub(crate) struct EngineLocalnetWalletFixtureBuilder {
    wallet: WalletFixture,
}

impl WalletFixtureBuilder {
    /// Configures the network reported by the deterministic wallet account.
    #[must_use]
    pub(crate) fn network(self, network: impl Into<String>) -> WalletFixture {
        let Self = self;
        WalletFixture {
            network: network.into(),
            max_messages: 1,
            extra_currency_supported: false,
            backend: WalletBackend::Protocol,
        }
    }
}

impl WalletFixture {
    /// Overrides the `SendTransaction.maxMessages` capability reported during connect.
    #[must_use]
    pub(crate) const fn max_messages(mut self, max_messages: u32) -> Self {
        self.max_messages = max_messages;
        self
    }

    /// Overrides the wallet's advertised support for TEP-92 extra currencies.
    #[must_use]
    pub(crate) const fn extra_currency_supported(mut self, supported: bool) -> Self {
        self.extra_currency_supported = supported;
        self
    }

    /// Selects the wallet-engine and Acton localnet backend for approved TON Connect sends.
    #[must_use]
    pub(crate) const fn on_localnet(self) -> EngineLocalnetWalletFixtureBuilder {
        let mut wallet = self;
        wallet.max_messages = 255;
        EngineLocalnetWalletFixtureBuilder { wallet }
    }
}

impl EngineLocalnetWalletFixtureBuilder {
    /// Funds the uninitialized source wallet with this exact nanogram balance.
    #[must_use]
    pub(crate) fn with_balance_nanograms(
        mut self,
        balance_nanograms: &'static str,
    ) -> WalletFixture {
        self.wallet.backend = WalletBackend::EngineLocalnet { balance_nanograms };
        self.wallet
    }
}

impl From<BridgeFixture> for Given {
    /// Converts a bridge fixture into a generic scenario precondition.
    fn from(value: BridgeFixture) -> Self {
        Self::Bridge(value)
    }
}

impl From<DappFixture> for Given {
    /// Converts a dApp fixture into a generic scenario precondition.
    fn from(value: DappFixture) -> Self {
        Self::Dapp(value)
    }
}

impl From<WalletFixture> for Given {
    /// Converts a wallet fixture into a generic scenario precondition.
    fn from(value: WalletFixture) -> Self {
        Self::Wallet(value)
    }
}

struct EngineWalletHarness {
    localnet: Arc<LocalnetHttpHost>,
    client: Arc<WalletClient>,
    account: TonConnectAccountInfo,
}

impl EngineWalletHarness {
    /// Imports the stable test wallet, funds it on localnet, and creates its real client.
    fn start(balance_nanograms: &str) -> TestResult<Self> {
        let platform_host = Arc::new(MemoryPlatformHost::default());
        let lifecycle = WalletLifecycle::new(platform_host.clone());
        let descriptor = block_on(lifecycle.import_wallet(ImportWalletRequest {
            record_id: ENGINE_RECORD_ID.to_owned(),
            network: Network::Testnet,
            recovery_words: test_wallet().recovery_words(),
        }))?;
        let account = lifecycle.ton_connect_account(descriptor.clone())?;
        let expected_account = engine_account_info()?;
        if account != expected_account
            || descriptor.address.as_str() != test_wallet().testnet_address()
        {
            return Err(failure(format!(
                "wallet lifecycle derived unexpected localnet account: {account:?}"
            )));
        }

        let localnet = Arc::new(
            LocalnetHttpHost::start(descriptor.address.as_str(), balance_nanograms)
                .map_err(failure)?,
        );
        let client = WalletClient::new(
            WalletClientConfig {
                record_id: NonEmptyString::try_from(ENGINE_RECORD_ID)?,
                address: descriptor.address,
                public_key: descriptor.public_key,
                local_secret_ref: Some(descriptor.secret_ref),
                network: Network::Testnet,
                send_validity_seconds: 300,
                resolution_margin_seconds: 60,
                providers: ProviderConfig {
                    toncenter_base_url: localnet.provider_base_url(),
                    dns_root_address: None,
                    request_timeout_ms: 15_000,
                },
            },
            localnet.clone(),
            platform_host,
        )?;
        Ok(Self {
            localnet,
            client,
            account,
        })
    }
}

struct ScenarioRunner {
    name: String,
    http: Client,
    bridge_fixture: Option<BridgeFixture>,
    dapp_fixture: Option<DappFixture>,
    wallet_fixture: Option<WalletFixture>,
    bridge_process: Option<ManagedChild>,
    dapp_process: Option<ManagedChild>,
    bridge_url: Option<String>,
    dapp_url: Option<String>,
    connect_link: Option<String>,
    transaction_sdk_request: Option<serde_json::Value>,
    transaction_request: Option<TransactionPayload>,
    expected_signed_boc: Option<String>,
    sign_message_sdk_request: Option<serde_json::Value>,
    sign_message_request: Option<TransactionPayload>,
    expected_internal_boc: Option<Boc>,
    wallet_client: Option<TonConnectClient>,
    wallet_session: Option<Arc<TonConnectSession>>,
    engine_wallet: Option<EngineWalletHarness>,
}

impl ScenarioRunner {
    /// Creates an isolated runner with an HTTPS client that trusts only this test's local setup.
    fn new(name: String) -> TestResult<Self> {
        Ok(Self {
            name,
            http: Client::builder()
                .timeout(Duration::from_secs(2))
                .danger_accept_invalid_certs(true)
                .build()?,
            bridge_fixture: None,
            dapp_fixture: None,
            wallet_fixture: None,
            bridge_process: None,
            dapp_process: None,
            bridge_url: None,
            dapp_url: None,
            connect_link: None,
            transaction_sdk_request: None,
            transaction_request: None,
            expected_signed_boc: None,
            sign_message_sdk_request: None,
            sign_message_request: None,
            expected_internal_boc: None,
            wallet_client: None,
            wallet_session: None,
            engine_wallet: None,
        })
    }

    /// Runs scenario steps sequentially and stops at the first failed action or expectation.
    fn run(mut self, steps: Vec<Step>) -> TestResult {
        for (index, step) in steps.into_iter().enumerate() {
            let step_number = index
                .checked_add(1)
                .ok_or_else(|| failure("scenario step index overflow"))?;
            if let Err(error) = self.execute(step) {
                return Err(failure(format!(
                    "scenario {:?}, step {} failed: {error}",
                    self.name, step_number
                )));
            }
        }
        Ok(())
    }

    /// Dispatches one DSL step to its fixture, action, or assertion implementation.
    fn execute(&mut self, step: Step) -> TestResult {
        match step {
            Step::Given(given) => {
                self.apply_fixture(given);
                Ok(())
            }
            Step::When(When::DappConnects) => self.connect_dapp(),
            Step::When(When::WalletApprovesConnect) => self.approve_connect(),
            Step::When(When::DappDisconnects) => self.disconnect_dapp(),
            Step::When(When::WalletAnswersDisconnect) => self.answer_disconnect(),
            Step::When(When::DappSendsTransaction(transaction)) => {
                self.send_transaction(&transaction)
            }
            Step::When(When::DappSignsMessage(transaction)) => self.sign_message(&transaction),
            Step::When(When::WalletAnswersTransaction(decision, expected_messages)) => {
                self.answer_transaction(decision, &expected_messages)
            }
            Step::When(When::WalletExecutesTransactionOnLocalnet(expected_messages)) => {
                self.execute_transaction_on_localnet(&expected_messages)
            }
            Step::When(When::WalletSignsMessage(expected_messages)) => {
                self.answer_sign_message(&expected_messages)
            }
            Step::When(When::WalletSignsMessageOnLocalnet(expected_messages)) => {
                self.sign_message_on_localnet(&expected_messages)
            }
            Step::When(When::RelayerSubmitsSignedMessage(value)) => {
                self.relay_signed_message(value)
            }
            Step::Then(Expectation::ConnectLinkCreated) => self.assert_connect_link(),
            Step::Then(Expectation::ManifestAvailable) => self.assert_manifest_available(),
            Step::Then(Expectation::DappConnected) => self.assert_dapp_connected(),
            Step::Then(Expectation::DappDisconnected) => self.assert_dapp_disconnected(),
            Step::Then(Expectation::DappRejectedWrongNetwork) => {
                self.assert_dapp_rejected_wrong_network()
            }
            Step::Then(Expectation::DappReceivedTransactionSuccess) => {
                self.assert_dapp_received_transaction_success()
            }
            Step::Then(Expectation::DappReceivedSignMessageSuccess) => {
                self.assert_dapp_received_sign_message_success()
            }
            Step::Then(Expectation::DappReceivedTransactionRejection) => {
                self.assert_dapp_received_transaction_rejection()
            }
            Step::Then(Expectation::DappRejectedTransactionPreflight(reason)) => {
                self.assert_dapp_rejected_transaction_preflight(reason)
            }
            Step::Then(Expectation::DappReceivedTransactionBadRequest) => {
                self.assert_dapp_received_transaction_bad_request()
            }
            Step::Then(Expectation::DappReceivedTransactionAccountMismatch) => {
                self.assert_dapp_received_transaction_account_mismatch()
            }
            Step::Then(Expectation::DappReceivedTransactionUnknownApp) => {
                self.assert_dapp_received_transaction_unknown_app()
            }
            Step::Then(Expectation::DeploymentTargetAbsent) => {
                self.assert_deployment_target_absent()
            }
            Step::Then(Expectation::OnChainAccount(expectation)) => {
                self.assert_on_chain_account(&expectation)
            }
        }
    }

    /// Stores a fixture until process startup or a wallet action needs it.
    fn apply_fixture(&mut self, given: Given) {
        match given {
            Given::Bridge(fixture) => self.bridge_fixture = Some(fixture),
            Given::Dapp(fixture) => self.dapp_fixture = Some(fixture),
            Given::Wallet(fixture) => self.wallet_fixture = Some(fixture),
        }
    }

    /// Starts missing actors, asks the SDK for a connect link, and retains that link for the wallet.
    fn connect_dapp(&mut self) -> TestResult {
        self.ensure_processes()?;
        let dapp_url = self
            .dapp_url
            .as_deref()
            .ok_or_else(|| failure("dApp process is not running"))?;
        let response = self
            .http
            .post(format!("{dapp_url}/command"))
            .json(&serde_json::json!({ "type": "connect" }))
            .send()?
            .error_for_status()?
            .json::<ConnectCommandResponse>()?;
        self.connect_link = Some(response.link);
        Ok(())
    }

    /// Sends the deterministic wallet's encrypted connect event to the dApp through the bridge.
    ///
    /// A network-mismatch scenario intentionally bypasses the production client's semantic
    /// validation while retaining the real authenticated bridge codec, because a conforming wallet
    /// cannot otherwise produce the invalid response needed to test the dApp SDK's rejection path.
    fn approve_connect(&mut self) -> TestResult {
        let link = self
            .connect_link
            .as_deref()
            .ok_or_else(|| failure("dApp has not created a connect link"))?;
        let bridge_url = self
            .bridge_url
            .as_deref()
            .ok_or_else(|| failure("bridge process is not running"))?;
        let wallet_fixture = self
            .wallet_fixture
            .as_ref()
            .ok_or_else(|| failure("wallet fixture is missing"))?;
        let parsed_link = ConnectLink::parse(link)?;
        let payload = ConnectEventPayload {
            items: vec![ConnectItemReply::TonAddress(wallet_account(
                wallet_fixture,
            )?)],
            device: test_device(wallet_fixture)?,
        };
        let requested_network = parsed_link.request().and_then(|request| {
            request.items.as_slice().iter().find_map(|item| match item {
                ConnectItem::TonAddr { network } => network.as_ref(),
                ConnectItem::TonProof { .. } | ConnectItem::Unsupported { .. } => None,
            })
        });
        let ttl = NonZeroU32::new(300).ok_or_else(|| failure("invalid message TTL"))?;

        if matches!(wallet_fixture.backend, WalletBackend::EngineLocalnet { .. }) {
            if wallet_fixture.max_messages != 255 || wallet_fixture.extra_currency_supported {
                return Err(failure(
                    "wallet-engine localnet profile advertises 255 messages without extra currencies",
                ));
            }
            let account = self
                .engine_wallet
                .as_ref()
                .ok_or_else(|| failure("wallet-engine localnet profile is not running"))?
                .account
                .clone();
            let session = ton_connect_session_from_link(
                link.to_owned(),
                TonConnectSessionConfig {
                    bridge_url: bridge_url.to_owned(),
                    max_event_bytes: 64 * 1024,
                    message_ttl_seconds: ttl.get(),
                },
            )?;
            let prompt = session
                .connect_prompt()?
                .ok_or_else(|| failure("wallet-engine did not expose the connect prompt"))?;
            let expected_manifest = parsed_link
                .request()
                .ok_or_else(|| failure("connect link has no request"))?
                .manifest_url
                .as_str();
            if prompt.manifest_url != expected_manifest
                || prompt.requested_network.as_deref() != requested_network.map(NetworkId::as_str)
            {
                return Err(failure(format!(
                    "wallet-engine exposed a different connect prompt: {prompt:?}"
                )));
            }
            let post = session.approve_connect(
                account,
                None,
                TonConnectDevice {
                    platform: TonConnectDevicePlatform::Linux,
                    app_name: "wallet-engine-test".to_owned(),
                    app_version: "0.1.0".to_owned(),
                },
            )?;
            drop(
                self.http
                    .post(&post.url)
                    .header("content-type", "text/plain; charset=utf-8")
                    .body(post.body)
                    .send()?
                    .error_for_status()?,
            );
            session.complete_pending_post()?;
            self.wallet_session = Some(session);
            return Ok(());
        }

        if requested_network
            .is_some_and(|network| network.as_str() != wallet_fixture.network.as_str())
        {
            let crypto = SessionCrypto::generate()?;
            let event = ConnectEvent::Connect {
                id: 0,
                payload,
                response: None,
            };
            let post = HttpBridgeUrl::try_from(bridge_url)?.prepare_post(
                &crypto,
                parsed_link.client_id(),
                ttl,
                None,
                parsed_link.trace_id(),
                &event,
            )?;
            drop(
                self.http
                    .post(post.url().clone())
                    .header("content-type", "text/plain; charset=utf-8")
                    .body(post.body().as_str().to_owned())
                    .send()?
                    .error_for_status()?,
            );
            return Ok(());
        }

        let config = TonConnectClientConfig::new(
            HttpBridgeUrl::try_from(bridge_url)?,
            NonZeroUsize::new(64 * 1024).ok_or_else(|| failure("invalid event limit"))?,
            ttl,
            HeartbeatMode::Message,
        );
        let mut client = TonConnectClient::from_parsed_link(&parsed_link, config)?;
        let post = client.approve_connect(payload, None)?;
        drop(
            self.http
                .post(post.url().clone())
                .header("content-type", "text/plain; charset=utf-8")
                .body(post.body().as_str().to_owned())
                .send()?
                .error_for_status()?,
        );
        self.wallet_client = Some(client);
        Ok(())
    }

    /// Renders a deterministic transaction, sends it to the dApp actor, and retains it for checks.
    fn send_transaction(&mut self, config: &DappTransactionConfig) -> TestResult {
        let dapp_url = self
            .dapp_url
            .as_deref()
            .ok_or_else(|| failure("dApp process is not running"))?;
        let wallet = self
            .wallet_fixture
            .as_ref()
            .ok_or_else(|| failure("wallet fixture is missing"))?;
        let transaction = config.render(wallet)?;
        drop(
            self.http
                .post(format!("{dapp_url}/command"))
                .json(&serde_json::json!({
                    "type": "send_transaction",
                    "transaction": transaction.sdk_request,
                }))
                .send()?
                .error_for_status()?,
        );
        self.transaction_sdk_request = Some(transaction.sdk_request);
        self.transaction_request = Some(transaction.wire_request);
        Ok(())
    }

    /// Renders a deterministic payload and asks the official SDK to call `signMessage`.
    fn sign_message(&mut self, config: &DappTransactionConfig) -> TestResult {
        let dapp_url = self
            .dapp_url
            .as_deref()
            .ok_or_else(|| failure("dApp process is not running"))?;
        let wallet = self
            .wallet_fixture
            .as_ref()
            .ok_or_else(|| failure("wallet fixture is missing"))?;
        let transaction = config.render(wallet)?;
        drop(
            self.http
                .post(format!("{dapp_url}/command"))
                .json(&serde_json::json!({
                    "type": "sign_message",
                    "transaction": transaction.sdk_request,
                }))
                .send()?
                .error_for_status()?,
        );
        self.sign_message_sdk_request = Some(transaction.sdk_request);
        self.sign_message_request = Some(transaction.wire_request);
        Ok(())
    }

    /// Asks the dApp actor to initiate an asynchronous TON Connect disconnect handshake.
    fn disconnect_dapp(&self) -> TestResult {
        let dapp_url = self
            .dapp_url
            .as_deref()
            .ok_or_else(|| failure("dApp process is not running"))?;
        drop(
            self.http
                .post(format!("{dapp_url}/command"))
                .json(&serde_json::json!({ "type": "disconnect" }))
                .send()?
                .error_for_status()?,
        );
        Ok(())
    }

    /// Reads the dApp disconnect request from SSE, validates it, and posts the wallet response.
    fn answer_disconnect(&mut self) -> TestResult {
        let incoming = self.receive_wallet_request()?;
        if incoming.request().method != "disconnect" {
            return Err(failure(format!(
                "expected disconnect request, got {}",
                incoming.request().method
            )));
        }
        let wallet_response = WalletResponse::Success(WalletResponseSuccess {
            result: WalletResult::Object(Map::new()),
            id: incoming.request().id.clone(),
        });
        let post = self
            .wallet_client
            .as_ref()
            .ok_or_else(|| failure("wallet is not connected"))?
            .prepare_response(&incoming, &wallet_response)?;
        self.post_bridge_message(&post)
    }

    /// Validates the pending `sendTransaction` request and returns the selected wallet outcome.
    fn answer_transaction(
        &mut self,
        decision: TransactionDecision,
        expected_messages: &DappTransactionMessages,
    ) -> TestResult {
        let uses_engine = self
            .wallet_fixture
            .as_ref()
            .is_some_and(|wallet| matches!(&wallet.backend, WalletBackend::EngineLocalnet { .. }));
        if uses_engine {
            return self.answer_engine_transaction(decision, expected_messages);
        }

        let incoming = self.receive_wallet_request()?;
        let KnownAppRequest::SendTransaction(request) = incoming.decode()? else {
            return Err(failure(format!(
                "expected sendTransaction request, got {}",
                incoming.request().method
            )));
        };
        let expected = self
            .transaction_request
            .as_ref()
            .ok_or_else(|| failure("dApp has not sent a transaction"))?;
        if &request.payload != expected {
            return Err(failure(format!(
                "wallet received a different transaction: expected {expected:?}, got {:?}",
                request.payload
            )));
        }
        let wallet = self
            .wallet_fixture
            .as_ref()
            .ok_or_else(|| failure("wallet fixture is missing"))?;
        let expected_messages = expected_messages.render(wallet)?;
        let TransactionPayload::Raw(payload) = &request.payload else {
            return Err(failure("wallet expected a raw transaction message"));
        };
        if payload.messages.as_slice() != expected_messages.as_slice() {
            return Err(failure(format!(
                "wallet received different messages: expected {expected_messages:?}, got {:?}",
                payload.messages
            )));
        }

        let expected_context_error = match decision {
            TransactionDecision::Expired => Some(RequestContextError::Expired),
            TransactionDecision::AccountMismatch => Some(RequestContextError::AccountMismatch),
            TransactionDecision::Approve
            | TransactionDecision::UserReject
            | TransactionDecision::UnknownApp => None,
        };
        if let Some(expected_error) = expected_context_error {
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let account = wallet_account(wallet)?;
            let active_network = NetworkId::try_from(wallet.network.as_str())?;
            if request
                .payload
                .validate_context(now, &active_network, &account.address)
                != Err(expected_error)
            {
                return Err(failure(format!(
                    "wallet expected context error {expected_error:?}, got {:?}",
                    request.payload
                )));
            }
        }

        let response = match decision {
            TransactionDecision::Approve => {
                self.expected_signed_boc = Some(TEST_SIGNED_BOC.to_owned());
                WalletResponse::Success(WalletResponseSuccess {
                    result: WalletResult::String(TEST_SIGNED_BOC.to_owned()),
                    id: request.id,
                })
            }
            TransactionDecision::UserReject => WalletResponse::Error {
                error: WalletResponseError {
                    code: RpcErrorCode::UserDeclined,
                    message: "User rejected transaction".to_owned(),
                    data: None,
                },
                id: request.id,
            },
            TransactionDecision::Expired => WalletResponse::Error {
                error: WalletResponseError {
                    code: RpcErrorCode::BadRequest,
                    message: "Transaction has expired".to_owned(),
                    data: None,
                },
                id: request.id,
            },
            TransactionDecision::AccountMismatch => WalletResponse::Error {
                error: WalletResponseError {
                    code: RpcErrorCode::BadRequest,
                    message: "Transaction signer does not match connected account".to_owned(),
                    data: None,
                },
                id: request.id,
            },
            TransactionDecision::UnknownApp => WalletResponse::Error {
                error: WalletResponseError {
                    code: RpcErrorCode::UnknownApp,
                    message: "Unknown or revoked dApp session".to_owned(),
                    data: None,
                },
                id: request.id,
            },
        };
        let post = self
            .wallet_client
            .as_ref()
            .ok_or_else(|| failure("wallet is not connected"))?
            .prepare_response(&incoming, &response)?;
        self.post_bridge_message(&post)
    }

    /// Validates a protocol `signMessage` request and returns a deterministic internal BOC.
    fn answer_sign_message(&mut self, expected_messages: &DappTransactionMessages) -> TestResult {
        let incoming = self.receive_wallet_request()?;
        let KnownAppRequest::SignMessage(request) = incoming.decode()? else {
            return Err(failure(format!(
                "expected signMessage request, got {}",
                incoming.request().method
            )));
        };
        let expected = self
            .sign_message_request
            .as_ref()
            .ok_or_else(|| failure("dApp has not requested a message signature"))?;
        if &request.payload != expected {
            return Err(failure(format!(
                "wallet received a different signMessage payload: expected {expected:?}, got {:?}",
                request.payload
            )));
        }
        self.assert_protocol_messages(&request.payload, expected_messages)?;

        let internal_boc = Boc::try_from(TEST_SIGNED_BOC)?;
        let protocol_result = ProtocolSignMessageResult {
            internal_boc: ton_connect_core::CellBoc::try_from(TEST_SIGNED_BOC)?,
        };
        let serde_json::Value::Object(result) = serde_json::to_value(protocol_result)? else {
            return Err(failure("signMessage result did not serialize as an object"));
        };
        let response = WalletResponse::Success(WalletResponseSuccess {
            result: WalletResult::Object(result),
            id: request.id,
        });
        let post = self
            .wallet_client
            .as_ref()
            .ok_or_else(|| failure("wallet is not connected"))?
            .prepare_response(&incoming, &response)?;
        self.expected_internal_boc = Some(internal_boc);
        self.post_bridge_message(&post)
    }

    /// Compares a decrypted protocol payload with the exact expected ordered message batch.
    fn assert_protocol_messages(
        &self,
        payload: &TransactionPayload,
        expected_messages: &DappTransactionMessages,
    ) -> TestResult {
        let wallet = self
            .wallet_fixture
            .as_ref()
            .ok_or_else(|| failure("wallet fixture is missing"))?;
        let expected_messages = expected_messages.render(wallet)?;
        let TransactionPayload::Raw(payload) = payload else {
            return Err(failure("wallet expected a raw transaction-shaped payload"));
        };
        if payload.messages.as_slice() != expected_messages.as_slice() {
            return Err(failure(format!(
                "wallet received different messages: expected {expected_messages:?}, got {:?}",
                payload.messages
            )));
        }
        Ok(())
    }

    /// Validates and rejects a pending production-session send without submitting it on-chain.
    fn answer_engine_transaction(
        &mut self,
        decision: TransactionDecision,
        expected_messages: &DappTransactionMessages,
    ) -> TestResult {
        if !matches!(decision, TransactionDecision::UserReject) {
            return Err(failure(
                "wallet-engine localnet profile only supports an explicit user rejection here",
            ));
        }
        let incoming = self.receive_engine_wallet_request()?;
        let TonConnectIncomingRequest::SendTransaction {
            id,
            request: send_request,
            ..
        } = incoming
        else {
            return Err(failure(format!(
                "wallet-engine decoded an unexpected TON Connect request: {incoming:?}"
            )));
        };
        self.assert_engine_send_request(&send_request, expected_messages)?;

        let session = self
            .wallet_session
            .as_ref()
            .ok_or_else(|| failure("production TON Connect session is not connected"))?;
        let post = session.prepare_error(
            id,
            TonConnectRpcErrorCode::UserDeclined,
            "User rejected transaction".to_owned(),
        )?;
        drop(
            self.http
                .post(&post.url)
                .header("content-type", "text/plain; charset=utf-8")
                .body(post.body)
                .send()?
                .error_for_status()?,
        );
        session.complete_pending_post()?;
        self.expected_signed_boc = None;
        Ok(())
    }

    /// Executes one decoded TON Connect send with the real wallet engine and acknowledges its BOC.
    fn execute_transaction_on_localnet(
        &mut self,
        expected_messages: &DappTransactionMessages,
    ) -> TestResult {
        let incoming = self.receive_engine_wallet_request()?;
        let TonConnectIncomingRequest::SendTransaction {
            id,
            request: send_request,
            ..
        } = incoming
        else {
            return Err(failure(format!(
                "wallet-engine decoded an unexpected TON Connect request: {incoming:?}"
            )));
        };
        self.assert_engine_send_request(&send_request, expected_messages)?;

        let result = block_on(
            self.engine_wallet
                .as_ref()
                .ok_or_else(|| failure("wallet-engine localnet profile is not running"))?
                .client
                .send(send_request),
        )?;
        if result.phase != SendPhase::Submitted {
            return Err(failure(format!(
                "wallet-engine did not submit the TON Connect transfer: {result:?}"
            )));
        }
        let signed_boc = result.signed_boc.to_base64();
        let session = self
            .wallet_session
            .as_ref()
            .ok_or_else(|| failure("production TON Connect session is not connected"))?;
        let post = session.prepare_send_success(id, signed_boc.clone())?;
        drop(
            self.http
                .post(&post.url)
                .header("content-type", "text/plain; charset=utf-8")
                .body(post.body)
                .send()?
                .error_for_status()?,
        );
        session.complete_pending_post()?;
        self.expected_signed_boc = Some(signed_boc);
        Ok(())
    }

    /// Uses the production session and engine to create a durable relayer-facing internal BOC.
    fn sign_message_on_localnet(
        &mut self,
        expected_messages: &DappTransactionMessages,
    ) -> TestResult {
        let incoming = self.receive_engine_wallet_request()?;
        let TonConnectIncomingRequest::SignMessage {
            id,
            request: sign_request,
            ..
        } = incoming
        else {
            return Err(failure(format!(
                "wallet-engine decoded an unexpected TON Connect request: {incoming:?}"
            )));
        };
        self.assert_engine_sign_request(&sign_request, expected_messages)?;

        let result = block_on(
            self.engine_wallet
                .as_ref()
                .ok_or_else(|| failure("wallet-engine localnet profile is not running"))?
                .client
                .sign_message(sign_request),
        )?;
        if result.phase != SendPhase::HandedOff {
            return Err(failure(format!(
                "wallet-engine did not hand off the TON Connect signed message: {result:?}"
            )));
        }
        let internal_boc = result.internal_boc;
        let session = self
            .wallet_session
            .as_ref()
            .ok_or_else(|| failure("production TON Connect session is not connected"))?;
        let post = session.prepare_sign_message_success(id, internal_boc.to_base64())?;
        drop(
            self.http
                .post(&post.url)
                .header("content-type", "text/plain; charset=utf-8")
                .body(post.body)
                .send()?
                .error_for_status()?,
        );
        session.complete_pending_post()?;
        self.expected_internal_boc = Some(internal_boc);
        Ok(())
    }

    /// Delivers the last signed internal BOC through the independent localnet relayer wallet.
    fn relay_signed_message(&self, attached_nanograms: &str) -> TestResult {
        let attached_nanograms = attached_nanograms
            .parse::<u64>()
            .map_err(|error| failure(format!("relayer value is invalid: {error}")))?;
        let internal_boc = self
            .expected_internal_boc
            .as_ref()
            .ok_or_else(|| failure("wallet has not handed off a signed internal message"))?;
        self.engine_wallet
            .as_ref()
            .ok_or_else(|| failure("wallet-engine localnet profile is not running"))?
            .localnet
            .relay_signed_message(internal_boc, attached_nanograms)
            .map_err(failure)
    }

    /// Compares wallet-engine's public send intent with the exact decrypted protocol payload.
    fn assert_engine_send_request(
        &self,
        actual: &SendRequest,
        expected_messages: &DappTransactionMessages,
    ) -> TestResult {
        let payload = self
            .transaction_request
            .as_ref()
            .ok_or_else(|| failure("dApp has not sent a transaction"))?;
        let expected = SendRequest {
            operation_id: actual.operation_id.clone(),
            force: false,
            intent: self.expected_engine_intent(payload, expected_messages)?,
        };
        if actual != &expected || !actual.operation_id.as_str().starts_with("ton-connect:") {
            return Err(failure(format!(
                "wallet-engine produced a different send intent: expected {expected:?}, got {actual:?}"
            )));
        }
        Ok(())
    }

    /// Compares wallet-engine's internal-sign request with the decrypted protocol payload.
    fn assert_engine_sign_request(
        &self,
        actual: &SignMessageRequest,
        expected_messages: &DappTransactionMessages,
    ) -> TestResult {
        let payload = self
            .sign_message_request
            .as_ref()
            .ok_or_else(|| failure("dApp has not requested a message signature"))?;
        let expected = SignMessageRequest {
            operation_id: actual.operation_id.clone(),
            force: false,
            intent: self.expected_engine_intent(payload, expected_messages)?,
        };
        if actual != &expected || !actual.operation_id.as_str().starts_with("ton-connect:") {
            return Err(failure(format!(
                "wallet-engine produced a different signMessage intent: expected {expected:?}, got {actual:?}"
            )));
        }
        Ok(())
    }

    /// Reconstructs the engine intent expected from one raw protocol payload.
    fn expected_engine_intent(
        &self,
        transaction: &TransactionPayload,
        expected_messages: &DappTransactionMessages,
    ) -> TestResult<SendIntent> {
        self.assert_protocol_messages(transaction, expected_messages)?;
        let TransactionPayload::Raw(payload) = transaction else {
            return Err(failure("wallet-engine expected a raw transaction payload"));
        };
        let messages = payload
            .messages
            .as_slice()
            .iter()
            .map(|message| {
                let body = message
                    .payload
                    .as_ref()
                    .map(|value| Boc::try_from(value.as_bytes().to_vec()))
                    .transpose()?
                    .map_or(SendMessageBody::Empty, |boc| SendMessageBody::RawPayload {
                        boc,
                    });
                Ok(SendMessage {
                    destination: TonAddressString::try_from(message.address.to_string())?,
                    amount: SendAmount::exact(message.amount.as_str().to_owned())?,
                    body,
                    bounce: false,
                    state_init: message
                        .state_init
                        .as_ref()
                        .map(|value| Boc::try_from(value.as_bytes().to_vec()))
                        .transpose()?,
                })
            })
            .collect::<TestResult<Vec<_>>>()?;
        Ok(SendIntent {
            expiration: payload
                .valid_until
                .map_or(SendExpiration::EngineDefault, |value| {
                    SendExpiration::Exact {
                        unix_timestamp: value,
                    }
                }),
            messages,
        })
    }

    /// Reads one SSE request through the production FFI-safe TON Connect session wrapper.
    fn receive_engine_wallet_request(&self) -> TestResult<TonConnectIncomingRequest> {
        let session = self
            .wallet_session
            .as_ref()
            .ok_or_else(|| failure("production TON Connect session is not connected"))?;
        let subscription = session.begin_events_subscription()?;
        let mut response = self
            .http
            .get(subscription)
            .timeout(EVENT_TIMEOUT)
            .send()?
            .error_for_status()?;
        let deadline = Instant::now()
            .checked_add(EVENT_TIMEOUT)
            .ok_or_else(|| failure("event timeout overflow"))?;
        let mut buffer = [0_u8; 8 * 1024];
        loop {
            if Instant::now() >= deadline {
                return Err(failure(
                    "wallet-engine did not receive the TON Connect request",
                ));
            }
            let read = io::Read::read(&mut response, &mut buffer)?;
            if read == 0 {
                continue;
            }
            let chunk = buffer
                .get(..read)
                .ok_or_else(|| failure("SSE read exceeded its buffer"))?;
            let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let events = session.ingest_sse_chunk(chunk.to_vec(), now)?;
            if let Some(event) = events.into_iter().next() {
                return Ok(event);
            }
        }
    }

    /// Reads one authenticated dApp request from the wallet's resumable SSE subscription.
    fn receive_wallet_request(&mut self) -> TestResult<IncomingRequest> {
        let client = self
            .wallet_client
            .as_mut()
            .ok_or_else(|| failure("wallet is not connected"))?;
        let subscription = client.begin_events_subscription();
        let mut response = self
            .http
            .get(subscription)
            .timeout(EVENT_TIMEOUT)
            .send()?
            .error_for_status()?;
        let deadline = Instant::now()
            .checked_add(EVENT_TIMEOUT)
            .ok_or_else(|| failure("event timeout overflow"))?;
        let mut buffer = [0_u8; 8 * 1024];
        loop {
            if Instant::now() >= deadline {
                return Err(failure("wallet did not receive a dApp request"));
            }
            let read = io::Read::read(&mut response, &mut buffer)?;
            if read == 0 {
                continue;
            }
            let chunk = buffer
                .get(..read)
                .ok_or_else(|| failure("SSE read exceeded its buffer"))?;
            let events = client.ingest_sse_chunk(chunk)?;
            if let Some(event) = events.into_iter().next() {
                return Ok(event);
            }
        }
    }

    /// Posts one already encrypted wallet event or response to the official bridge.
    fn post_bridge_message(&self, post: &PreparedBridgePost) -> TestResult {
        drop(
            self.http
                .post(post.url().clone())
                .header("content-type", "text/plain; charset=utf-8")
                .body(post.body().as_str().to_owned())
                .send()?
                .error_for_status()?,
        );
        Ok(())
    }

    /// Parses and validates every stable field of the connection link produced by the dApp SDK.
    fn assert_connect_link(&self) -> TestResult {
        let link = self
            .connect_link
            .as_deref()
            .ok_or_else(|| failure("connect link was not created"))?;
        let url = url::Url::parse(link)?;
        if url.scheme() != "tc" || url.fragment().is_some() {
            return Err(failure(format!(
                "connect link must use the tc scheme without a fragment: {link}"
            )));
        }
        let query = url.query_pairs().collect::<Vec<_>>();
        let expected_parameters = ["v", "id", "r", "trace_id"];
        if query.len() != expected_parameters.len()
            || query
                .iter()
                .any(|(name, _)| !expected_parameters.contains(&name.as_ref()))
        {
            return Err(failure(format!(
                "connect link has unexpected query parameters: {query:?}"
            )));
        }
        for expected in expected_parameters {
            let count = query
                .iter()
                .filter(|(name, _)| name.as_ref() == expected)
                .count();
            if count != 1 {
                return Err(failure(format!(
                    "connect link parameter {expected:?} must occur once, got {count}"
                )));
            }
        }
        let version = query
            .iter()
            .find_map(|(name, value)| (name == "v").then_some(value.as_ref()));
        if version != Some("2") {
            return Err(failure(format!(
                "connect link protocol version must be 2, got {version:?}"
            )));
        }

        let parsed = ConnectLink::parse(link)?;
        let client_id = parsed.client_id().to_string();
        if client_id.len() != 64
            || !client_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(failure(format!(
                "connect link client ID is not 32-byte lowercase hex: {client_id:?}"
            )));
        }
        if parsed.trace_id().is_none() {
            return Err(failure("connect link has no trace ID"));
        }
        if !matches!(parsed.return_strategy(), ReturnStrategy::Back) {
            return Err(failure(format!(
                "unexpected return strategy: {:?}",
                parsed.return_strategy()
            )));
        }
        if parsed.embedded_request().is_some() || !parsed.extensions().is_empty() {
            return Err(failure(
                "connect link unexpectedly contains an embedded request or extensions",
            ));
        }

        let request = parsed
            .request()
            .ok_or_else(|| failure("connect link has no connect request"))?;
        let dapp_url = self
            .dapp_url
            .as_deref()
            .ok_or_else(|| failure("dApp process is not running"))?;
        let bridge_url = self
            .bridge_url
            .as_deref()
            .ok_or_else(|| failure("bridge process is not running"))?;
        let dapp = self
            .dapp_fixture
            .as_ref()
            .ok_or_else(|| failure("dApp fixture is missing"))?;
        let expected_config = dapp.config.render(dapp_url, bridge_url);
        if request.manifest_url.as_str() != expected_config.manifest_url {
            return Err(failure(format!(
                "connect request manifest URL {:?} does not match configured URL {:?}",
                request.manifest_url.as_str(),
                expected_config.manifest_url
            )));
        }
        match request.items.as_slice() {
            [ConnectItem::TonAddr { network }]
                if network.as_ref().map(NetworkId::as_str)
                    == expected_config.in_network.as_deref() => {}
            items => {
                return Err(failure(format!(
                    "connect request must contain one ton_addr item for configured network {:?}, got {items:?}",
                    expected_config.in_network
                )));
            }
        }
        Ok(())
    }

    /// Downloads the configured manifest and icon and compares them with the connect request.
    fn assert_manifest_available(&self) -> TestResult {
        let link = self
            .connect_link
            .as_deref()
            .ok_or_else(|| failure("connect link was not created"))?;
        let dapp_url = self
            .dapp_url
            .as_deref()
            .ok_or_else(|| failure("dApp process is not running"))?;
        let bridge_url = self
            .bridge_url
            .as_deref()
            .ok_or_else(|| failure("bridge process is not running"))?;
        let fixture = self
            .dapp_fixture
            .as_ref()
            .ok_or_else(|| failure("dApp fixture is missing"))?;
        let expected = fixture.config.render(dapp_url, bridge_url);
        let parsed_link = ConnectLink::parse(link)?;
        let request = parsed_link
            .request()
            .ok_or_else(|| failure("connect link has no connect request"))?;
        if request.manifest_url.as_str() != expected.manifest_url {
            return Err(failure(format!(
                "connect request manifest URL {:?} does not match configured URL {:?}",
                request.manifest_url.as_str(),
                expected.manifest_url
            )));
        }
        let actual = self
            .http
            .get(&expected.manifest_url)
            .send()?
            .error_for_status()?
            .json::<DappManifestResponse>()?;
        if actual.url != expected.manifest.url
            || actual.name != expected.manifest.name
            || actual.icon_url != expected.manifest.icon_url
        {
            return Err(failure(format!(
                "manifest differs from dApp config: expected {:?}, got {actual:?}",
                expected.manifest
            )));
        }
        drop(
            self.http
                .get(&expected.manifest.icon_url)
                .send()?
                .error_for_status()?,
        );
        Ok(())
    }

    /// Verifies the full configuration, account, device, error state, and journal seen by the dApp.
    fn assert_dapp_connected(&self) -> TestResult {
        let state = self.wait_for_dapp_state("connected", true)?;
        self.assert_dapp_connection_snapshot(&state)?;
        assert_journal_order(
            &state.journal,
            &["connect_link_created", "wallet_connected"],
        )
    }

    /// Compares one connected actor snapshot with every stable dApp and wallet fixture field.
    fn assert_dapp_connection_snapshot(&self, state: &DappState) -> TestResult {
        let expected_config = self.expected_dapp_config()?;
        if state.config != expected_config {
            return Err(failure(format!(
                "dApp used a different config: expected {expected_config:?}, got {:?}",
                state.config
            )));
        }
        let wallet = self
            .wallet_fixture
            .as_ref()
            .ok_or_else(|| failure("wallet fixture is missing"))?;
        let expected_account = expected_dapp_account(wallet)?;
        if state.account.as_ref() != Some(&expected_account) {
            return Err(failure(format!(
                "dApp received a different account: expected {expected_account:?}, got {:?}",
                state.account
            )));
        }
        let expected_device = expected_dapp_device(wallet)?;
        if state.device.as_ref() != Some(&expected_device) {
            return Err(failure(format!(
                "dApp received different device capabilities: expected {expected_device:?}, got {:?}",
                state.device
            )));
        }
        if state.error.is_some() {
            return Err(failure(format!(
                "dApp reported an error after successful connect: {:?}",
                state.error
            )));
        }
        Ok(())
    }

    /// Verifies that both peers completed disconnection and the dApp cleared transient state.
    fn assert_dapp_disconnected(&self) -> TestResult {
        let state = self.wait_for_dapp_state("disconnected", false)?;
        if state.device.is_some() || state.error.is_some() {
            return Err(failure(format!(
                "dApp retained device state or error after disconnect: {state:?}"
            )));
        }
        assert_journal_order(
            &state.journal,
            &[
                "connect_link_created",
                "wallet_connected",
                "disconnect_requested",
                "wallet_disconnected",
                "dapp_disconnected",
            ],
        )
    }

    /// Verifies the SDK's structured error when the wallet reports a network other than requested.
    fn assert_dapp_rejected_wrong_network(&self) -> TestResult {
        let state = self.wait_for_dapp_state("error", false)?;
        let expected_config = self.expected_dapp_config()?;
        if state.config != expected_config || state.device.is_some() {
            return Err(failure(format!(
                "dApp retained wallet state after network rejection: {state:?}"
            )));
        }
        let error = state
            .error
            .as_ref()
            .ok_or_else(|| failure("dApp did not report the network error"))?;
        if error.name != "WalletWrongNetworkError" || !error.message.contains("wrong network") {
            return Err(failure(format!(
                "dApp reported an unexpected network error: {error:?}"
            )));
        }
        let wallet = self
            .wallet_fixture
            .as_ref()
            .ok_or_else(|| failure("wallet fixture is missing"))?;
        let expected_network = expected_config
            .in_network
            .as_deref()
            .ok_or_else(|| failure("dApp config has no in_network"))?;
        let cause = error
            .cause
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| failure("network error has no structured cause"))?;
        if cause
            .get("expectedChainId")
            .and_then(serde_json::Value::as_str)
            != Some(expected_network)
            || cause
                .get("actualChainId")
                .and_then(serde_json::Value::as_str)
                != Some(wallet.network.as_str())
        {
            return Err(failure(format!(
                "network error cause does not match dApp/wallet networks: {cause:?}"
            )));
        }
        assert_journal_order(&state.journal, &["connect_link_created", "connector_error"])
    }

    /// Verifies the exact transaction echoed by the actor and its successful SDK result.
    fn assert_dapp_received_transaction_success(&self) -> TestResult {
        let state = self.wait_for_dapp_transaction_state("success")?;
        self.assert_dapp_connection_snapshot(&state)?;
        self.assert_transaction_request(&state)?;
        if state.transaction.error.is_some() {
            return Err(failure(format!(
                "dApp reported an error for a successful transaction: {:?}",
                state.transaction.error
            )));
        }
        let result = state
            .transaction
            .result
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| failure("dApp transaction success has no object result"))?;
        let expected_boc = self
            .expected_signed_boc
            .as_deref()
            .ok_or_else(|| failure("wallet did not retain the signed transaction BOC"))?;
        if result.get("boc").and_then(serde_json::Value::as_str) != Some(expected_boc)
            || result
                .keys()
                .any(|key| !matches!(key.as_str(), "boc" | "traceId"))
            || result
                .get("traceId")
                .is_some_and(|trace_id| trace_id.as_str().is_none_or(str::is_empty))
        {
            return Err(failure(format!(
                "dApp received an unexpected transaction result: {result:?}"
            )));
        }
        assert_journal_order(
            &state.journal,
            &[
                "connect_link_created",
                "wallet_connected",
                "transaction_requested",
                "transaction_sent",
                "transaction_succeeded",
            ],
        )
    }

    /// Verifies the exact `signMessage` request and the internal BOC returned by the SDK.
    fn assert_dapp_received_sign_message_success(&self) -> TestResult {
        let state = self.wait_for_dapp_sign_message_state("success")?;
        self.assert_dapp_connection_snapshot(&state)?;
        let expected_request = self
            .sign_message_sdk_request
            .as_ref()
            .ok_or_else(|| failure("dApp has not requested a message signature"))?;
        if state.sign_message.request.as_ref() != Some(expected_request) {
            return Err(failure(format!(
                "dApp retained a different signMessage request: expected {expected_request:?}, got {:?}",
                state.sign_message.request
            )));
        }
        if state.sign_message.error.is_some() {
            return Err(failure(format!(
                "dApp reported an error for a successful signMessage request: {:?}",
                state.sign_message.error
            )));
        }
        let result = state
            .sign_message
            .result
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| failure("dApp signMessage success has no object result"))?;
        let expected_boc = self
            .expected_internal_boc
            .as_ref()
            .map(Boc::to_base64)
            .ok_or_else(|| failure("wallet did not retain the signed internal BOC"))?;
        if result
            .get("internalBoc")
            .and_then(serde_json::Value::as_str)
            != Some(expected_boc.as_str())
            || result
                .keys()
                .any(|key| !matches!(key.as_str(), "internalBoc" | "traceId"))
            || result
                .get("traceId")
                .is_some_and(|trace_id| trace_id.as_str().is_none_or(str::is_empty))
        {
            return Err(failure(format!(
                "dApp received an unexpected signMessage result: {result:?}"
            )));
        }
        assert_journal_order(
            &state.journal,
            &[
                "connect_link_created",
                "wallet_connected",
                "sign_message_requested",
                "sign_message_sent",
                "sign_message_succeeded",
            ],
        )
    }

    /// Verifies that a wallet error 300 becomes the SDK's high-level user-rejection error.
    fn assert_dapp_received_transaction_rejection(&self) -> TestResult {
        let state = self.wait_for_dapp_transaction_state("error")?;
        self.assert_dapp_connection_snapshot(&state)?;
        self.assert_transaction_request(&state)?;
        if state.transaction.result.is_some() {
            return Err(failure(format!(
                "dApp retained a result for a rejected transaction: {:?}",
                state.transaction.result
            )));
        }
        let error = state
            .transaction
            .error
            .as_ref()
            .ok_or_else(|| failure("dApp did not report transaction rejection"))?;
        if error.name != "UserRejectsError"
            || !error.message.contains("User rejected transaction")
            || error.cause.is_some()
        {
            return Err(failure(format!(
                "dApp reported an unexpected transaction rejection: {error:?}"
            )));
        }
        assert_journal_order(
            &state.journal,
            &[
                "connect_link_created",
                "wallet_connected",
                "transaction_requested",
                "transaction_sent",
                "transaction_failed",
            ],
        )
    }

    /// Verifies SDK preflight errors and proves that no request reached the bridge transport.
    fn assert_dapp_rejected_transaction_preflight(
        &self,
        reason: TransactionPreflightRejection,
    ) -> TestResult {
        let state = self.wait_for_dapp_transaction_state("error")?;
        self.assert_dapp_connection_snapshot(&state)?;
        self.assert_transaction_request(&state)?;
        if state.transaction.result.is_some() {
            return Err(failure(format!(
                "dApp retained a result for a preflight-rejected transaction: {:?}",
                state.transaction.result
            )));
        }
        let error = state
            .transaction
            .error
            .as_ref()
            .ok_or_else(|| failure("dApp did not report transaction preflight rejection"))?;
        match reason {
            TransactionPreflightRejection::WrongNetwork => {
                let wallet = self
                    .wallet_fixture
                    .as_ref()
                    .ok_or_else(|| failure("wallet fixture is missing"))?;
                let requested_network = self
                    .transaction_sdk_request
                    .as_ref()
                    .and_then(|request| request.get("network"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| failure("transaction request has no network"))?;
                let expected_cause = serde_json::json!({
                    "expectedChainId": wallet.network,
                    "actualChainId": requested_network,
                });
                if error.name != "WalletWrongNetworkError"
                    || !error.message.contains("wrong network")
                    || error.cause.as_ref() != Some(&expected_cause)
                {
                    return Err(failure(format!(
                        "dApp reported an unexpected transaction network error: {error:?}"
                    )));
                }
            }
            TransactionPreflightRejection::MessageLimit => {
                Self::assert_unsupported_transaction_feature(
                    error,
                    2,
                    false,
                    "Max support messages number is 1, but 2 is required",
                )?;
            }
            TransactionPreflightRejection::ExtraCurrency => {
                Self::assert_unsupported_transaction_feature(
                    error,
                    1,
                    true,
                    "Extra currencies support is required",
                )?;
            }
        }
        if state.journal.iter().any(|event| {
            matches!(
                event.kind.as_str(),
                "transaction_sent" | "transaction_succeeded"
            )
        }) {
            return Err(failure(format!(
                "SDK preflight rejection still reached transport: {:?}",
                state.journal
            )));
        }
        assert_journal_order(
            &state.journal,
            &[
                "connect_link_created",
                "wallet_connected",
                "transaction_requested",
                "transaction_failed",
            ],
        )
    }

    /// Compares a capability preflight failure with the SDK's structured required-feature cause.
    fn assert_unsupported_transaction_feature(
        error: &DappError,
        min_messages: u64,
        extra_currency_required: bool,
        message_fragment: &str,
    ) -> TestResult {
        let expected_cause = serde_json::json!({
            "requiredFeature": {
                "featureName": "SendTransaction",
                "value": {
                    "minMessages": min_messages,
                    "extraCurrencyRequired": extra_currency_required,
                },
            },
        });
        if error.name != "WalletNotSupportFeatureError"
            || !error.message.contains(message_fragment)
            || error.cause.as_ref() != Some(&expected_cause)
        {
            return Err(failure(format!(
                "dApp reported an unexpected feature error: {error:?}"
            )));
        }
        Ok(())
    }

    /// Verifies that wallet error 1 becomes `BadRequestError` after a request reached transport.
    fn assert_dapp_received_transaction_bad_request(&self) -> TestResult {
        self.assert_dapp_received_transaction_error("BadRequestError", "Transaction has expired")
    }

    /// Verifies SDK mapping of a wallet-side fixed-signer validation failure.
    fn assert_dapp_received_transaction_account_mismatch(&self) -> TestResult {
        self.assert_dapp_received_transaction_error(
            "BadRequestError",
            "Transaction signer does not match connected account",
        )
    }

    /// Verifies that wallet error 100 becomes the SDK's `UnknownAppError`.
    fn assert_dapp_received_transaction_unknown_app(&self) -> TestResult {
        self.assert_dapp_received_transaction_error(
            "UnknownAppError",
            "Unknown or revoked dApp session",
        )
    }

    /// Verifies one terminal wallet protocol error and its transport journal ordering.
    fn assert_dapp_received_transaction_error(
        &self,
        expected_name: &str,
        expected_message: &str,
    ) -> TestResult {
        let state = self.wait_for_dapp_transaction_state("error")?;
        self.assert_dapp_connection_snapshot(&state)?;
        self.assert_transaction_request(&state)?;
        if state.transaction.result.is_some() {
            return Err(failure(format!(
                "dApp retained a result for an errored transaction: {:?}",
                state.transaction.result
            )));
        }
        let error = state
            .transaction
            .error
            .as_ref()
            .ok_or_else(|| failure("dApp did not report the wallet protocol error"))?;
        if error.name != expected_name
            || !error.message.contains(expected_message)
            || error.cause.is_some()
        {
            return Err(failure(format!(
                "dApp reported an unexpected protocol error: {error:?}"
            )));
        }
        assert_journal_order(
            &state.journal,
            &[
                "connect_link_created",
                "wallet_connected",
                "transaction_requested",
                "transaction_sent",
                "transaction_failed",
            ],
        )
    }

    /// Verifies the deploy destination has neither active code nor funds before the send.
    fn assert_deployment_target_absent(&self) -> TestResult {
        let target = test_deployment_target()?;
        self.engine_wallet
            .as_ref()
            .ok_or_else(|| failure("wallet-engine localnet profile is not running"))?
            .localnet
            .assert_account_absent(&target.address)
            .map_err(failure)
    }

    /// Verifies one source or deployment account against localnet state, balance, and seqno.
    fn assert_on_chain_account(&self, expectation: &OnChainAccountExpectation) -> TestResult {
        if expectation.state.is_empty() {
            return Err(failure("on-chain account expectation has no state"));
        }
        let engine = self
            .engine_wallet
            .as_ref()
            .ok_or_else(|| failure("wallet-engine localnet profile is not running"))?;
        let address = match expectation.target {
            OnChainAccountTarget::Deployment => test_deployment_target()?.address,
            OnChainAccountTarget::SourceWallet => engine.account.address.clone(),
        };
        engine
            .localnet
            .assert_account(&address, expectation.state, Some(expectation.seqno))
            .map_err(failure)?;
        if let Some((minimum, maximum)) = expectation.balance_range {
            let minimum = minimum.parse::<u128>()?;
            let maximum = maximum.parse::<u128>()?;
            let actual = engine.localnet.account_balance(&address).map_err(failure)?;
            if actual < minimum || actual > maximum {
                return Err(failure(format!(
                    "on-chain balance for {address} must be in {minimum}..={maximum}, got {actual}"
                )));
            }
        }
        Ok(())
    }

    /// Compares the dApp actor's retained SDK request with the exact value sent by the harness.
    fn assert_transaction_request(&self, state: &DappState) -> TestResult {
        let expected = self
            .transaction_sdk_request
            .as_ref()
            .ok_or_else(|| failure("dApp has not sent a transaction"))?;
        if state.transaction.request.as_ref() != Some(expected) {
            return Err(failure(format!(
                "dApp retained a different transaction: expected {expected:?}, got {:?}",
                state.transaction.request
            )));
        }
        Ok(())
    }

    /// Renders the scenario's dApp fixture using the actual actor and bridge origins.
    fn expected_dapp_config(&self) -> TestResult<RenderedDappConfig> {
        let dapp_url = self
            .dapp_url
            .as_deref()
            .ok_or_else(|| failure("dApp process is not running"))?;
        let bridge_url = self
            .bridge_url
            .as_deref()
            .ok_or_else(|| failure("bridge process is not running"))?;
        let fixture = self
            .dapp_fixture
            .as_ref()
            .ok_or_else(|| failure("dApp fixture is missing"))?;
        Ok(fixture.config.render(dapp_url, bridge_url))
    }

    /// Polls the actor until both status and account presence match the expected stable state.
    fn wait_for_dapp_state(
        &self,
        expected_status: &str,
        expects_account: bool,
    ) -> TestResult<DappState> {
        let dapp_url = self
            .dapp_url
            .as_deref()
            .ok_or_else(|| failure("dApp process is not running"))?;
        let deadline = Instant::now()
            .checked_add(EVENT_TIMEOUT)
            .ok_or_else(|| failure("event timeout overflow"))?;
        let mut last_state = None;
        while Instant::now() < deadline {
            if let Ok(response) = self.http.get(format!("{dapp_url}/state")).send()
                && let Ok(response) = response.error_for_status()
                && let Ok(state) = response.json::<DappState>()
            {
                if state.status == expected_status && state.account.is_some() == expects_account {
                    return Ok(state);
                }
                last_state = Some(state);
            }
            thread::sleep(Duration::from_millis(50));
        }

        Err(failure(format!(
            "dApp did not reach {expected_status:?} before timeout; last state: {last_state:?}"
        )))
    }

    /// Polls the actor until the asynchronous `sendTransaction` promise reaches a terminal state.
    fn wait_for_dapp_transaction_state(&self, expected_status: &str) -> TestResult<DappState> {
        let dapp_url = self
            .dapp_url
            .as_deref()
            .ok_or_else(|| failure("dApp process is not running"))?;
        let deadline = Instant::now()
            .checked_add(EVENT_TIMEOUT)
            .ok_or_else(|| failure("event timeout overflow"))?;
        let mut last_state = None;
        while Instant::now() < deadline {
            if let Ok(response) = self.http.get(format!("{dapp_url}/state")).send()
                && let Ok(response) = response.error_for_status()
                && let Ok(state) = response.json::<DappState>()
            {
                if state.transaction.status == expected_status {
                    return Ok(state);
                }
                last_state = Some(state);
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err(failure(format!(
            "dApp transaction did not reach {expected_status:?} before timeout; last state: {last_state:?}"
        )))
    }

    /// Polls until the official SDK's asynchronous `signMessage` call is terminal.
    fn wait_for_dapp_sign_message_state(&self, expected_status: &str) -> TestResult<DappState> {
        let dapp_url = self
            .dapp_url
            .as_deref()
            .ok_or_else(|| failure("dApp process is not running"))?;
        let deadline = Instant::now()
            .checked_add(EVENT_TIMEOUT)
            .ok_or_else(|| failure("event timeout overflow"))?;
        let mut last_state = None;
        while Instant::now() < deadline {
            if let Ok(response) = self.http.get(format!("{dapp_url}/state")).send()
                && let Ok(response) = response.error_for_status()
                && let Ok(state) = response.json::<DappState>()
            {
                if state.sign_message.status == expected_status {
                    return Ok(state);
                }
                last_state = Some(state);
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err(failure(format!(
            "dApp signMessage did not reach {expected_status:?} before timeout; last state: {last_state:?}"
        )))
    }

    /// Lazily starts the bridge and dApp exactly once in dependency order.
    fn ensure_processes(&mut self) -> TestResult {
        if self.engine_wallet.is_none() {
            let wallet = self
                .wallet_fixture
                .as_ref()
                .ok_or_else(|| failure("wallet fixture is missing"))?;
            if let WalletBackend::EngineLocalnet { balance_nanograms } = wallet.backend {
                if wallet.network != TON_TESTNET_NETWORK_ID {
                    return Err(failure(
                        "wallet-engine localnet profile requires the TON testnet network ID",
                    ));
                }
                self.engine_wallet = Some(EngineWalletHarness::start(balance_nanograms)?);
            }
        }

        if self.bridge_process.is_none() {
            let fixture = self
                .bridge_fixture
                .as_ref()
                .ok_or_else(|| failure("bridge fixture is missing"))?;
            let (process, url) = start_bridge(&self.http, fixture)?;
            self.bridge_process = Some(process);
            self.bridge_url = Some(url);
        }

        if self.dapp_process.is_none() {
            let fixture = self
                .dapp_fixture
                .as_ref()
                .ok_or_else(|| failure("dApp fixture is missing"))?;
            let bridge_url = self
                .bridge_url
                .as_deref()
                .ok_or_else(|| failure("bridge process is not running"))?;
            let (process, url) = start_dapp(&self.http, fixture, bridge_url)?;
            self.dapp_process = Some(process);
            self.dapp_url = Some(url);
        }

        Ok(())
    }
}

#[derive(Deserialize)]
struct ConnectCommandResponse {
    link: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DappState {
    status: String,
    account: Option<DappAccount>,
    device: Option<DappDevice>,
    config: RenderedDappConfig,
    error: Option<DappError>,
    transaction: DappTransactionState,
    sign_message: DappTransactionState,
    journal: Vec<DappJournalEvent>,
}

#[derive(Debug, Deserialize)]
struct DappTransactionState {
    status: String,
    request: Option<serde_json::Value>,
    result: Option<serde_json::Value>,
    error: Option<DappError>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DappAccount {
    address: String,
    chain: String,
    public_key: Option<String>,
    wallet_state_init: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DappDevice {
    platform: String,
    app_name: String,
    app_version: String,
    max_protocol_version: u32,
    features: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct DappError {
    name: String,
    message: String,
    cause: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct DappJournalEvent {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DappManifestResponse {
    url: String,
    name: String,
    icon_url: String,
}

struct ManagedChild {
    name: &'static str,
    child: Child,
}

impl ManagedChild {
    /// Fails readiness polling immediately if a child process has already exited.
    fn ensure_running(&mut self) -> TestResult {
        if let Some(status) = self.child.try_wait()? {
            return Err(failure(format!(
                "{} exited before becoming ready: {status}",
                self.name
            )));
        }
        Ok(())
    }
}

impl Drop for ManagedChild {
    /// Terminates and reaps every test child even when a scenario fails early.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts the official bridge on free loopback ports and waits for its metrics readiness endpoint.
fn start_bridge(http: &Client, fixture: &BridgeFixture) -> TestResult<(ManagedChild, String)> {
    let binary = env::var_os("TON_CONNECT_BRIDGE_BIN").map_or_else(
        || PathBuf::from("/tmp/ton-connect-research/bridge/bridge3"),
        PathBuf::from,
    );
    if !binary.is_file() {
        return Err(failure(format!(
            "official bridge binary not found at {}; set TON_CONNECT_BRIDGE_BIN",
            binary.display()
        )));
    }
    let port = free_port()?;
    let metrics_port = free_port()?;
    let storage = match fixture.storage {
        BridgeStorage::Memory => "memory",
    };
    let child = Command::new(&binary)
        .env("PORT", port.to_string())
        .env("METRICS_PORT", metrics_port.to_string())
        .env("STORAGE", storage)
        .env("NTP_ENABLED", "false")
        .env("CORS_ENABLE", "true")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut process = ManagedChild {
        name: "official TON Connect bridge",
        child,
    };
    let base_url = format!("http://127.0.0.1:{port}");
    wait_until_ready(
        http,
        &mut process,
        &format!("http://127.0.0.1:{metrics_port}/readyz"),
    )?;
    Ok((process, format!("{base_url}/bridge")))
}

/// Starts the compiled HTTPS dApp actor with rendered JSON configuration and test TLS material.
fn start_dapp(
    http: &Client,
    fixture: &DappFixture,
    bridge_url: &str,
) -> TestResult<(ManagedChild, String)> {
    let actor =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/ton-connect/dapp/dist/server.js");
    if !actor.is_file() {
        return Err(failure(format!(
            "TypeScript dApp is not built at {}; run npm ci && npm run build in tests/ton-connect/dapp",
            actor.display()
        )));
    }
    let port = free_port()?;
    let base_url = format!("https://127.0.0.1:{port}");
    let rendered_config = fixture.config.render(&base_url, bridge_url);
    let actor_config = serde_json::to_string(&rendered_config)?;
    let fixtures =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/ton-connect/dapp/fixtures");
    let tls_key = fixtures.join("localhost-key.pem");
    let tls_certificate = fixtures.join("localhost-cert.pem");
    let child = Command::new(env::var_os("NODE").unwrap_or_else(|| "node".into()))
        .arg(actor)
        .env("PORT", port.to_string())
        .env("TON_CONNECT_DAPP_CONFIG", actor_config)
        .env("TON_CONNECT_TLS_KEY", tls_key)
        .env("TON_CONNECT_TLS_CERTIFICATE", tls_certificate)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut process = ManagedChild {
        name: "TypeScript TON Connect dApp",
        child,
    };
    wait_until_ready(http, &mut process, &format!("{base_url}/health"))?;
    Ok((process, base_url))
}

/// Expands every `{actor_origin}` placeholder without changing other fixture text.
fn render_actor_origin(value: &str, actor_origin: &str) -> String {
    value.replace(ACTOR_ORIGIN, actor_origin)
}

/// Polls one health endpoint while also checking that its owning process remains alive.
fn wait_until_ready(http: &Client, process: &mut ManagedChild, url: &str) -> TestResult {
    let deadline = Instant::now()
        .checked_add(PROCESS_START_TIMEOUT)
        .ok_or_else(|| failure("process start timeout overflow"))?;
    while Instant::now() < deadline {
        process.ensure_running()?;
        if let Ok(response) = http.get(url).send()
            && response.status().is_success()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(failure(format!(
        "{} did not become ready at {url}",
        process.name
    )))
}

/// Reserves an ephemeral loopback port long enough to discover its operating-system assignment.
fn free_port() -> TestResult<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

struct DeploymentTarget {
    address: String,
    state_init: String,
}

/// Selects the protocol-only account or the real wallet-engine account for this fixture.
fn wallet_account(wallet: &WalletFixture) -> TestResult<TonAddressItemReply> {
    match wallet.backend {
        WalletBackend::Protocol => test_account(&wallet.network),
        WalletBackend::EngineLocalnet { .. } => {
            let account = engine_account_info()?;
            let public_key = <[u8; 32]>::try_from(account.public_key.as_slice())
                .map_err(|_| failure("test wallet public key must contain 32 bytes"))?;
            Ok(TonAddressItemReply::new(
                account.address.parse()?,
                NetworkId::try_from(account.network.as_str())?,
                WalletStateInit::try_from(account.wallet_state_init.as_str())?,
                Ed25519PublicKey::from_bytes(public_key),
            ))
        }
    }
}

/// Derives the public TON Connect account material used by the wallet-engine localnet profile.
fn engine_account_info() -> TestResult<TonConnectAccountInfo> {
    let (state, public_key) = test_wallet_state(test_wallet().recovery_phrase_bytes())?;
    Ok(TonConnectAccountInfo {
        address: state.derive_address(0)?.to_string(),
        network: TON_TESTNET_NETWORK_ID.to_owned(),
        wallet_state_init: state.as_str().to_owned(),
        public_key,
    })
}

/// Derives a second wallet `StateInit` used as a deterministic deployable contract.
fn test_deployment_target() -> TestResult<DeploymentTarget> {
    let (state, _) = test_wallet_state(test_wallet().other_recovery_phrase_bytes())?;
    let address = FriendlyAddress::from_raw(state.derive_address(0)?, true, true)?;
    Ok(DeploymentTarget {
        address: address.as_str().to_owned(),
        state_init: state.as_str().to_owned(),
    })
}

/// Builds the testnet wallet `StateInit` and public key from one mnemonic.
fn test_wallet_state(mnemonic: &[u8]) -> TestResult<(WalletStateInit, Vec<u8>)> {
    let mnemonic = std::str::from_utf8(mnemonic)?;
    let key_pair = test_wallet::rotation_anchor_key_pair(mnemonic)?;
    let code = WalletVersion::get_code(WalletVersion::Wallet)?.clone();
    let data = WalletVersion::get_default_data(
        WalletVersion::Wallet,
        &key_pair,
        WALLET_SUBWALLET_ID_DEFAULT_TESTNET,
    )?;
    let state = StateInit::new(code, data);
    let state = WalletStateInit::from_boc(state.to_boc()?)?;
    Ok((state, key_pair.public_key.to_vec()))
}

/// Builds a deterministic valid wallet account whose connect reply reports the selected network.
fn test_account(network: &str) -> TestResult<TonAddressItemReply> {
    let mut state = TonCell::builder();
    state.write_bit(false)?;
    state.write_bit(false)?;
    state.write_bit(true)?;
    state.write_ref(TonCell::empty().to_owned())?;
    state.write_bit(true)?;
    state.write_ref(TonCell::empty().to_owned())?;
    state.write_bit(false)?;
    let state = WalletStateInit::from_boc(state.build()?.to_boc()?)?;
    let address = state.derive_address(0)?;
    Ok(TonAddressItemReply::new(
        address,
        NetworkId::try_from(network)?,
        state,
        Ed25519PublicKey::from_bytes([0_u8; 32]),
    ))
}

/// Builds deterministic wallet metadata and advertised capabilities for exact dApp assertions.
fn test_device(wallet: &WalletFixture) -> TestResult<DeviceInfo> {
    Ok(DeviceInfo::new(
        DevicePlatform::Linux,
        "wallet-engine-test".to_owned(),
        "0.1.0".to_owned(),
        2,
        vec![
            Feature::SendTransaction(SendTransactionFeature::new(
                wallet.max_messages,
                Some(wallet.extra_currency_supported),
                None,
            )?),
            Feature::SignMessage(SignMessageFeature::new(
                wallet.max_messages,
                Some(wallet.extra_currency_supported),
                None,
            )?),
        ],
    )?)
}

/// Converts the protocol `network` field into the SDK account's `chain` field.
fn expected_dapp_account(wallet: &WalletFixture) -> TestResult<DappAccount> {
    let reply = ConnectItemReply::TonAddress(wallet_account(wallet)?);
    let mut sdk_account = serde_json::to_value(reply)?;
    let object = sdk_account
        .as_object_mut()
        .ok_or_else(|| failure("serialized TON account reply is not an object"))?;
    let _ = object.remove("name");
    let network = object
        .remove("network")
        .ok_or_else(|| failure("serialized TON account reply has no network"))?;
    let _ = object.insert("chain".to_owned(), network);
    Ok(serde_json::from_value(sdk_account)?)
}

/// Converts protocol device metadata into the TypeScript actor's JSON state shape.
fn expected_dapp_device(wallet: &WalletFixture) -> TestResult<DappDevice> {
    Ok(serde_json::from_value(serde_json::to_value(test_device(
        wallet,
    )?)?)?)
}

/// Requires the selected event kinds to appear as an ordered subsequence of the actor journal.
fn assert_journal_order(journal: &[DappJournalEvent], expected: &[&str]) -> TestResult {
    let mut events = journal.iter();
    for expected_kind in expected {
        if !events.any(|event| event.kind == *expected_kind) {
            let actual = journal
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>();
            return Err(failure(format!(
                "dApp journal does not contain {expected_kind:?} in the expected order; got {actual:?}"
            )));
        }
    }
    Ok(())
}

/// Creates a lightweight boxed test error from an assertion or fixture diagnostic.
fn failure(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(io::Error::other(message.into()))
}
