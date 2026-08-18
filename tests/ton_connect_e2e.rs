#[allow(
    dead_code,
    unused_imports,
    unused_results,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::iter_over_hash_type,
    clippy::panic,
    clippy::pedantic,
    reason = "the shared integration-test support module contains fixtures for other test binaries"
)]
#[path = "support/ton_connect_support.rs"]
mod support;

use support::ton_connect_scenario::{
    DappConfig, DappManifestConfig, DappTransactionConfig, DappTransactionMessage, bridge,
    connect_link_created, dapp, dapp_connected, dapp_connects, dapp_disconnected, dapp_disconnects,
    dapp_received_transaction_account_mismatch, dapp_received_transaction_bad_request,
    dapp_received_transaction_rejection, dapp_received_transaction_success,
    dapp_received_transaction_unknown_app, dapp_rejected_transaction_for_extra_currency,
    dapp_rejected_transaction_for_message_limit, dapp_rejected_transaction_wrong_network,
    dapp_rejected_wrong_network, dapp_sends_transaction, deployment_target, manifest_available,
    scenario, source_wallet_account, wallet, wallet_answers_disconnect, wallet_approves_connect,
    wallet_approves_transaction, wallet_approves_transaction_messages,
    wallet_executes_transaction_on_localnet, wallet_rejects_expired_transaction,
    wallet_rejects_transaction, wallet_rejects_transaction_for_account_mismatch,
    wallet_rejects_transaction_from_unknown_app,
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
        .given(bridge().official().in_memory())
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
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(MAINNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .then(manifest_available())
        .when(wallet_approves_connect())
        .then(dapp_rejected_wrong_network())
        .run()
}

/// Verifies that omitting `in_network` leaves `ton_addr.network` unrestricted.
#[test]
fn connects_without_a_network_restriction() -> TestResult {
    const DAPP_CONFIG: DappConfig = DappConfig::new(
        "{actor_origin}/tonconnect-manifest.json",
        DappManifestConfig::new(
            "{actor_origin}",
            "Wallet Engine unrestricted TON Connect dApp",
            "{actor_origin}/icon.png",
        ),
    )
    .universal_link("tc://");

    scenario("connect without a network restriction")
        .given(bridge().official().in_memory())
        .given(dapp().config(DAPP_CONFIG))
        .given(wallet().network(MAINNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .then(manifest_available())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .run()
}

/// Verifies an exact encrypted `sendTransaction` request and successful wallet response.
#[test]
fn sends_a_transaction_through_the_connected_session() -> TestResult {
    scenario("send transaction through connected session")
        .given(bridge().official().in_memory())
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

/// Verifies that a plain transfer preserves amount and omits all optional message fields.
#[test]
fn sends_a_plain_transfer_without_optional_fields() -> TestResult {
    const TRANSACTION_MESSAGE: DappTransactionMessage =
        DappTransactionMessage::new("{wallet_address}", "1");
    const TRANSACTION_CONFIG: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, TRANSACTION_MESSAGE).valid_for_seconds(120);

    scenario("send a plain transfer without optional fields")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TRANSACTION_CONFIG))
        .when(wallet_approves_transaction(TRANSACTION_MESSAGE))
        .then(dapp_received_transaction_success())
        .run()
}

/// Verifies exact `stateInit` casing and content across SDK and protocol representations.
#[test]
fn sends_a_transaction_with_state_init() -> TestResult {
    const EMPTY_CELL_BOC: &str = "te6ccgEBAQEAAgAAAA==";
    const TRANSACTION_MESSAGE: DappTransactionMessage =
        DappTransactionMessage::new("{wallet_address}", "10000000").state_init(EMPTY_CELL_BOC);
    const TRANSACTION_CONFIG: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, TRANSACTION_MESSAGE).valid_for_seconds(120);

    scenario("send a transaction with state init")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TRANSACTION_CONFIG))
        .when(wallet_approves_transaction(TRANSACTION_MESSAGE))
        .then(dapp_received_transaction_success())
        .run()
}

/// Verifies that a TON Connect deploy message is signed, executed, and observable on localnet.
#[test]
fn deploys_an_account_through_ton_connect_on_localnet() -> TestResult {
    const DEPLOY_MESSAGE: DappTransactionMessage =
        DappTransactionMessage::new("{deployment_address}", "1000000000")
            .state_init("{deployment_state_init}");
    const TRANSACTION_CONFIG: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, DEPLOY_MESSAGE).valid_for_seconds(120);

    scenario("deploy an account through TON Connect on localnet")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(
            wallet()
                .network(TESTNET_NETWORK)
                .on_localnet()
                .with_balance_nanograms("10000000000"),
        )
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .then(deployment_target().absent())
        .when(dapp_sends_transaction(TRANSACTION_CONFIG))
        .when(wallet_executes_transaction_on_localnet(DEPLOY_MESSAGE))
        .then(dapp_received_transaction_success())
        .then(
            deployment_target()
                .active()
                .balance_between("900000000", "1000000000")
                .seqno(0),
        )
        .then(source_wallet_account().active().seqno(1))
        .run()
}

/// Verifies wallet sequence numbers across a deploy followed by a second TON Connect send.
#[test]
fn sends_a_second_transaction_after_deploying_an_account_on_localnet() -> TestResult {
    const DEPLOY_MESSAGE: DappTransactionMessage =
        DappTransactionMessage::new("{deployment_address}", "1000000000")
            .state_init("{deployment_state_init}");
    const SECOND_MESSAGE: DappTransactionMessage =
        DappTransactionMessage::new("{deployment_address}", "100000000");
    const DEPLOY_TRANSACTION: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, DEPLOY_MESSAGE).valid_for_seconds(120);
    const SECOND_TRANSACTION: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, SECOND_MESSAGE).valid_for_seconds(120);

    scenario("send a second transaction after deploying an account on localnet")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(
            wallet()
                .network(TESTNET_NETWORK)
                .on_localnet()
                .with_balance_nanograms("10000000000"),
        )
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .then(deployment_target().absent())
        .when(dapp_sends_transaction(DEPLOY_TRANSACTION))
        .when(wallet_executes_transaction_on_localnet(DEPLOY_MESSAGE))
        .then(dapp_received_transaction_success())
        .then(deployment_target().active().seqno(0))
        .then(source_wallet_account().active().seqno(1))
        .when(dapp_sends_transaction(SECOND_TRANSACTION))
        .when(wallet_executes_transaction_on_localnet(SECOND_MESSAGE))
        .then(dapp_received_transaction_success())
        .then(deployment_target().active().seqno(0))
        .then(source_wallet_account().active().seqno(2))
        .run()
}

/// Verifies that rejecting a TON Connect deploy leaves its deterministic target absent.
#[test]
fn keeps_the_deployment_target_absent_when_the_wallet_rejects_the_send() -> TestResult {
    const DEPLOY_MESSAGE: DappTransactionMessage =
        DappTransactionMessage::new("{deployment_address}", "1000000000")
            .state_init("{deployment_state_init}");
    const TRANSACTION_CONFIG: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, DEPLOY_MESSAGE).valid_for_seconds(120);

    scenario("keep the deployment target absent after wallet rejection")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(
            wallet()
                .network(TESTNET_NETWORK)
                .on_localnet()
                .with_balance_nanograms("10000000000"),
        )
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .then(deployment_target().absent())
        .when(dapp_sends_transaction(TRANSACTION_CONFIG))
        .when(wallet_rejects_transaction(DEPLOY_MESSAGE))
        .then(dapp_received_transaction_rejection())
        .then(deployment_target().absent())
        .run()
}

/// Verifies a two-message request when the wallet advertises a matching batch limit.
#[test]
fn sends_two_messages_when_the_wallet_supports_them() -> TestResult {
    const FIRST_MESSAGE: DappTransactionMessage =
        DappTransactionMessage::new("{wallet_address}", "10000000").payload("te6ccgEBAQEAAgAAAA==");
    const SECOND_MESSAGE: DappTransactionMessage =
        DappTransactionMessage::new("{wallet_address}", "20000000");
    const TRANSACTION_CONFIG: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, FIRST_MESSAGE)
            .valid_for_seconds(120)
            .and_message(SECOND_MESSAGE);

    scenario("send two messages when the wallet supports them")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK).max_messages(2))
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TRANSACTION_CONFIG))
        .when(wallet_approves_transaction_messages(
            FIRST_MESSAGE,
            SECOND_MESSAGE,
        ))
        .then(dapp_received_transaction_success())
        .run()
}

/// Verifies an extra-currency transfer when the wallet explicitly advertises support.
#[test]
fn sends_extra_currency_when_the_wallet_supports_it() -> TestResult {
    const TRANSACTION_MESSAGE: DappTransactionMessage =
        DappTransactionMessage::new("{wallet_address}", "10000000").extra_currency(1, "5");
    const TRANSACTION_CONFIG: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, TRANSACTION_MESSAGE).valid_for_seconds(120);

    scenario("send extra currency when the wallet supports it")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(
            wallet()
                .network(TESTNET_NETWORK)
                .extra_currency_supported(true),
        )
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TRANSACTION_CONFIG))
        .when(wallet_approves_transaction(TRANSACTION_MESSAGE))
        .then(dapp_received_transaction_success())
        .run()
}

/// Verifies that wallet error 300 rejects the dApp's pending transaction promise.
#[test]
fn rejects_a_transaction_in_the_wallet() -> TestResult {
    scenario("reject transaction in wallet")
        .given(bridge().official().in_memory())
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

/// Verifies that the SDK blocks a transaction for a network other than the connected account.
#[test]
fn rejects_a_transaction_for_another_network_before_bridge() -> TestResult {
    const TRANSACTION_CONFIG: DappTransactionConfig =
        DappTransactionConfig::new(MAINNET_NETWORK, TEST_TRANSACTION_MESSAGE)
            .valid_for_seconds(120);

    scenario("reject transaction for another network before bridge")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TRANSACTION_CONFIG))
        .then(dapp_rejected_transaction_wrong_network())
        .run()
}

/// Verifies that the SDK enforces the wallet's advertised one-message limit before transport.
#[test]
fn rejects_too_many_transaction_messages_before_bridge() -> TestResult {
    const TRANSACTION_CONFIG: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, TEST_TRANSACTION_MESSAGE)
            .valid_for_seconds(120)
            .and_message(TEST_TRANSACTION_MESSAGE);

    scenario("reject too many transaction messages before bridge")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TRANSACTION_CONFIG))
        .then(dapp_rejected_transaction_for_message_limit())
        .run()
}

/// Verifies that the SDK rejects extra currencies not advertised by the connected wallet.
#[test]
fn rejects_unadvertised_extra_currency_before_bridge() -> TestResult {
    const TRANSACTION_MESSAGE: DappTransactionMessage =
        DappTransactionMessage::new("{wallet_address}", "10000000").extra_currency(1, "5");
    const TRANSACTION_CONFIG: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, TRANSACTION_MESSAGE).valid_for_seconds(120);

    scenario("reject unadvertised extra currency before bridge")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TRANSACTION_CONFIG))
        .then(dapp_rejected_transaction_for_extra_currency())
        .run()
}

/// Verifies wallet-side expiry validation and SDK mapping of protocol error 1.
#[test]
fn rejects_an_expired_transaction_in_the_wallet() -> TestResult {
    const TRANSACTION_CONFIG: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, TEST_TRANSACTION_MESSAGE)
            .expired_seconds_ago(30);

    scenario("reject expired transaction in wallet")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TRANSACTION_CONFIG))
        .when(wallet_rejects_expired_transaction(TEST_TRANSACTION_MESSAGE))
        .then(dapp_received_transaction_bad_request())
        .run()
}

/// Verifies wallet-side `from` validation and SDK mapping of protocol error 1.
#[test]
fn rejects_a_transaction_for_another_sender_in_the_wallet() -> TestResult {
    const OTHER_ACCOUNT: &str =
        "0:0000000000000000000000000000000000000000000000000000000000000000";
    const TRANSACTION_CONFIG: DappTransactionConfig =
        DappTransactionConfig::new(TESTNET_NETWORK, TEST_TRANSACTION_MESSAGE)
            .valid_for_seconds(120)
            .from(OTHER_ACCOUNT);

    scenario("reject a transaction for another sender in the wallet")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TRANSACTION_CONFIG))
        .when(wallet_rejects_transaction_for_account_mismatch(
            TEST_TRANSACTION_MESSAGE,
        ))
        .then(dapp_received_transaction_account_mismatch())
        .run()
}

/// Verifies SDK mapping of protocol error 100 for an unknown or revoked dApp session.
#[test]
fn reports_an_unknown_app_transaction_error() -> TestResult {
    scenario("report an unknown app transaction error")
        .given(bridge().official().in_memory())
        .given(dapp().config(TEST_DAPP_CONFIG))
        .given(wallet().network(TESTNET_NETWORK))
        .when(dapp_connects())
        .then(connect_link_created())
        .when(wallet_approves_connect())
        .then(dapp_connected())
        .when(dapp_sends_transaction(TEST_TRANSACTION_CONFIG))
        .when(wallet_rejects_transaction_from_unknown_app(
            TEST_TRANSACTION_MESSAGE,
        ))
        .then(dapp_received_transaction_unknown_app())
        .run()
}
