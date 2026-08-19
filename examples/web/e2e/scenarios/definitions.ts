import {
  dapp,
  dappObserved,
  network,
  scenario,
  screen,
  type ScenarioDefinition,
  type TransactionConfig,
  wallet,
  walletUi,
} from "../dsl/scenario"
import type {DappActorConfig} from "../fixtures/processes"

export const TESTNET_NETWORK: string = "-3"
const FAR_FUTURE_VALID_UNTIL: number = 4_102_444_800
const FIRST_DESTINATION_ADDRESS: string = "0QAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACkT"
const SECOND_DESTINATION_ADDRESS: string = "0QAREREREREREREREREREREREREREREREREREREREREREQBc"

export const TEST_DAPP_CONFIG: DappActorConfig = {
  inNetwork: TESTNET_NETWORK,
  manifest: {
    iconUrl: "{actor_origin}/icon.png",
    name: "Wallet Engine client E2E dApp",
    url: "https://app.example",
  },
  manifestUrl: "{actor_origin}/tonconnect-manifest.json",
  universalLink: "tc://",
}

const TWO_MESSAGE_TRANSACTION: TransactionConfig = {
  fromConnectedWallet: true,
  messages: [
    {
      address: FIRST_DESTINATION_ADDRESS,
      amount: "1000000000",
    },
    {
      address: SECOND_DESTINATION_ADDRESS,
      amount: "2000000000",
    },
  ],
  network: TESTNET_NETWORK,
  validUntil: FAR_FUTURE_VALID_UNTIL,
}

const TEN_MESSAGE_TRANSACTION: TransactionConfig = {
  fromConnectedWallet: true,
  messages: Array.from({length: 10}, (_, index) => ({
    address: index % 2 === 0 ? FIRST_DESTINATION_ADDRESS : SECOND_DESTINATION_ADDRESS,
    amount: `${index + 1}00000000`,
  })),
  network: TESTNET_NETWORK,
  validUntil: FAR_FUTURE_VALID_UNTIL,
}

const FIRST_LOCALNET_TRANSACTION: TransactionConfig = {
  fromConnectedWallet: true,
  messages: [{address: SECOND_DESTINATION_ADDRESS, amount: "100000000"}],
  network: TESTNET_NETWORK,
  validUntil: FAR_FUTURE_VALID_UNTIL,
}

const SECOND_LOCALNET_TRANSACTION: TransactionConfig = {
  fromConnectedWallet: true,
  messages: [{address: SECOND_DESTINATION_ADDRESS, amount: "200000000"}],
  network: TESTNET_NETWORK,
  validUntil: FAR_FUTURE_VALID_UNTIL,
}

export const walletLifecycleScenario: ScenarioDefinition = scenario("create and restore a wallet")
  .given(wallet().open())
  .expect(walletUi().welcome())
  .expect(screen("wallet-welcome"))
  .when(wallet().create())
  .expect(walletUi().recovery())
  .expect(screen("wallet-recovery", "recovery"))
  .when(wallet().acceptRecovery())
  .expect(walletUi().dashboard())
  .expect(screen("wallet-dashboard"))
  .when(wallet().reloadDashboard())
  .expect(walletUi().dashboard())
  .expect(screen("wallet-restored-dashboard"))
  .build()

export const tonConnectScenario: ScenarioDefinition = scenario(
  "connect and reject a two-message TON Connect transaction",
)
  .given(dapp().start(TEST_DAPP_CONFIG))
  .given(wallet().open())
  .when(wallet().create())
  .when(wallet().acceptRecovery())
  .expect(walletUi().dashboard())
  .when(dapp().createConnectLink())
  .when(wallet().handleConnectLink())
  .expect(walletUi().connectApproval(TEST_DAPP_CONFIG.manifest.name))
  .expect(screen("ton-connect-approval", "dialog"))
  .when(wallet().approveConnect())
  .expect(dappObserved().connected(TESTNET_NETWORK))
  .when(wallet().reloadDashboard())
  .when(wallet().openTonConnect())
  .expect(walletUi().connectedDapp(TEST_DAPP_CONFIG.manifest.name))
  .expect(screen("ton-connect-connected", "dialog"))
  .when(wallet().closeDialog())
  .when(dapp().requestTransaction(TWO_MESSAGE_TRANSACTION))
  .expect(walletUi().transaction(TWO_MESSAGE_TRANSACTION.messages))
  .expect(screen("ton-connect-two-message-review", "dialog"))
  .when(wallet().rejectRequest())
  .expect(dappObserved().transactionRejected())
  .build()

export const rejectedTonConnectScenario: ScenarioDefinition = scenario(
  "reject a TON Connect connection",
)
  .given(dapp().start(TEST_DAPP_CONFIG))
  .given(wallet().open())
  .when(wallet().create())
  .when(wallet().acceptRecovery())
  .expect(walletUi().dashboard())
  .when(dapp().createConnectLink())
  .when(wallet().handleConnectLink())
  .expect(walletUi().connectApproval(TEST_DAPP_CONFIG.manifest.name))
  .when(wallet().rejectConnect())
  .expect(dappObserved().connectionRejected())
  .when(wallet().openTonConnect())
  .expect(walletUi().tonConnectEntry())
  .expect(screen("ton-connect-rejected", "dialog"))
  .when(wallet().closeDialog())
  .build()

export const tenMessageTonConnectScenario: ScenarioDefinition = scenario(
  "connect and reject a ten-message TON Connect transaction",
)
  .given(dapp().start(TEST_DAPP_CONFIG))
  .given(wallet().open())
  .when(wallet().create())
  .when(wallet().acceptRecovery())
  .expect(walletUi().dashboard())
  .when(dapp().createConnectLink())
  .when(wallet().handleConnectLink())
  .expect(walletUi().connectApproval(TEST_DAPP_CONFIG.manifest.name))
  .when(wallet().approveConnect())
  .expect(dappObserved().connected(TESTNET_NETWORK))
  .when(dapp().requestTransaction(TEN_MESSAGE_TRANSACTION))
  .expect(walletUi().transaction(TEN_MESSAGE_TRANSACTION.messages))
  .expect(screen("ton-connect-ten-message-review", "dialog"))
  .when(wallet().rejectRequest())
  .expect(dappObserved().transactionRejected())
  .build()

export const localnetActivityScenario: ScenarioDefinition = scenario(
  "refresh transaction history from Acton localnet",
)
  .given(network().localnet())
  .given(dapp().start(TEST_DAPP_CONFIG))
  .given(wallet().open())
  .when(wallet().create())
  .when(wallet().acceptRecovery())
  .expect(walletUi().dashboard())
  .expect(
    walletUi().activity({
      amounts: ["10000000000"],
      count: 1,
      directions: ["received"],
      rememberAs: "funded",
    }),
  )
  .when(dapp().createConnectLink())
  .when(wallet().handleConnectLink())
  .when(wallet().approveConnect())
  .expect(dappObserved().connected(TESTNET_NETWORK))
  .when(dapp().requestTransaction(FIRST_LOCALNET_TRANSACTION))
  .expect(walletUi().transaction(FIRST_LOCALNET_TRANSACTION.messages))
  .when(wallet().approveRequest())
  .expect(dappObserved().transactionApproved())
  .expect(walletUi().activity({count: 1, sameAs: "funded"}))
  .when(wallet().refresh())
  .expect(
    walletUi().activity({
      amounts: ["100000000", "10000000000"],
      count: 2,
      directions: ["sent", "received"],
      extends: "funded",
      rememberAs: "first-send",
    }),
  )
  .when(wallet().refresh())
  .expect(walletUi().activity({count: 2, sameAs: "first-send"}))
  .when(dapp().requestTransaction(SECOND_LOCALNET_TRANSACTION))
  .expect(walletUi().transaction(SECOND_LOCALNET_TRANSACTION.messages))
  .when(wallet().approveRequest())
  .expect(dappObserved().transactionApproved())
  .expect(walletUi().activity({count: 2, sameAs: "first-send"}))
  .when(wallet().refresh())
  .expect(
    walletUi().activity({
      amounts: ["200000000", "100000000", "10000000000"],
      count: 3,
      directions: ["sent", "sent", "received"],
      extends: "first-send",
      rememberAs: "second-send",
    }),
  )
  .expect(screen("localnet-activity"))
  .when(wallet().reloadDashboard())
  .expect(walletUi().activity({count: 3, sameAs: "second-send"}))
  .build()

export const clientScenarios: Readonly<Record<string, ScenarioDefinition>> = {
  "localnet-activity": localnetActivityScenario,
  "ton-connect": tonConnectScenario,
  "ton-connect-rejected": rejectedTonConnectScenario,
  "ton-connect-ten-messages": tenMessageTonConnectScenario,
  "wallet-lifecycle": walletLifecycleScenario,
}
