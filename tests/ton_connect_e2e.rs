#[path = "support/ton_connect_scenario.rs"]
mod ton_connect_scenario;

use ton_connect_scenario::{
    DappConfig, DappManifestConfig, DappTransactionConfig, DappTransactionMessage, bridge,
    connect_link_created, dapp, dapp_connected, dapp_connects, dapp_disconnected, dapp_disconnects,
    dapp_received_transaction_rejection, dapp_received_transaction_success,
    dapp_rejected_wrong_network, dapp_sends_transaction, manifest_available, scenario, wallet,
    wallet_answers_disconnect, wallet_approves_connect, wallet_approves_transaction,
    wallet_rejects_transaction,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const TESTNET_NETWORK: &str = "-3";
const MAINNET_NETWORK: &str = "-239";

const TEST_DAPP_CONFIG: DappConfig = DappConfig::new(
    "{actor_origin}/tonconnect-manifest.json",
    DappManifestConfig::new(
        "{actor_origin}",
        "Wallet Engine TON Connect Test dApp",
        "{actor_origin}/icon.png",
    ),
)
.universal_link("tc://")
.in_network(TESTNET_NETWORK);

const TEST_TRANSACTION_MESSAGE: DappTransactionMessage =
    DappTransactionMessage::new("{wallet_address}", "10000000").payload("te6ccgEBAQEAAgAAAA==");

const TEST_TRANSACTION_CONFIG: DappTransactionConfig =
    DappTransactionConfig::new(TESTNET_NETWORK, TEST_TRANSACTION_MESSAGE).valid_for_seconds(120);

/// Verifies connect and dApp-initiated disconnect through the official bridge.
#[test]
fn connects_through_the_official_bridge() -> TestResult {
    scenario("connect through the official bridge")
        .given(bridge().official().memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .then(manifest_available())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_disconnects())
        .when(wallet_answers_disconnect())
        .then(dapp_disconnected())
        .run()
}

/// Verifies that the dApp SDK rejects a wallet account from a different network.
#[test]
fn rejects_a_wallet_connected_to_another_network() -> TestResult {
    scenario("reject wallet connected to another network")
        .given(bridge().official().memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(MAINNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .then(manifest_available())
        .when(wallet_approves_connect())
        .then(dapp_rejected_wrong_network())
        .run()
}

/// Verifies an exact encrypted `sendTransaction` request and successful wallet response.
#[test]
fn sends_a_transaction_through_the_connected_session() -> TestResult {
    scenario("send transaction through connected session")
        .given(bridge().official().memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TEST_TRANSACTION_CONFIG))
        .when(wallet_approves_transaction(TEST_TRANSACTION_MESSAGE))
        .then(dapp_received_transaction_success())
        .run()
}

/// Verifies that wallet error 300 rejects the dApp's pending transaction promise.
#[test]
fn rejects_a_transaction_in_the_wallet() -> TestResult {
    scenario("reject transaction in wallet")
        .given(bridge().official().memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TEST_TRANSACTION_CONFIG))
        .when(wallet_rejects_transaction(TEST_TRANSACTION_MESSAGE))
        .then(dapp_received_transaction_rejection())
        .run()
}
