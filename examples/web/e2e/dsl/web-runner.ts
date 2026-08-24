import {
  expect,
  type APIRequestContext,
  type Locator,
  type Page,
  type TestInfo,
} from "@playwright/test"

import {WebWalletDriver, type WebActivityObservation} from "../drivers/web-wallet-driver"
import type {TonConnectHarness} from "../fixtures/ton-connect"
import type {DappActor, DappActorState} from "../fixtures/processes"
import {formatNanogramBalance} from "../../src/lib/format"
import type {
  ActivityExpectation,
  ScenarioAction,
  ScenarioDefinition,
  TransactionConfig,
  TransactionMessageConfig,
} from "./scenario"

const EXPECTED_BALANCE: RegExp = /^10\s*GRAM$/
const DEFAULT_PROVIDER_ORIGIN: string = "https://127.0.0.1:5198"

interface WebScenarioContext {
  readonly page: Page
  readonly providerOrigin?: string
  readonly testInfo: TestInfo
  readonly tonConnect?: TonConnectHarness
}

interface RenderedTransaction {
  readonly from?: string
  readonly messages: readonly TransactionMessageConfig[]
  readonly network: string
  readonly validUntil: number
}

/** Interprets platform-independent scenario steps against the Web wallet. */
export class WebScenarioRunner {
  private readonly context: WebScenarioContext
  private readonly provider: ProviderControl
  private readonly wallet: WebWalletDriver
  private readonly rememberedActivity = new Map<string, WebActivityObservation>()
  private connectLink: string | undefined
  private lastTransaction: RenderedTransaction | undefined

  /** Creates a runner for one isolated Playwright page and its optional TON Connect fixture. */
  constructor(context: WebScenarioContext) {
    this.context = context
    this.provider = new ProviderControl(
      context.page.request,
      context.providerOrigin ?? DEFAULT_PROVIDER_ORIGIN,
    )
    this.wallet = new WebWalletDriver(context.page)
  }

  /** Executes every scenario step in declaration order and identifies a failing step. */
  async run(definition: ScenarioDefinition): Promise<void> {
    await this.provider.reset()
    await this.context.testInfo.attach("scenario.json", {
      body: Buffer.from(JSON.stringify(definition, null, 2)),
      contentType: "application/json",
    })

    for (const [index, step] of definition.steps.entries()) {
      try {
        await this.execute(step.action)
      } catch (error) {
        throw new Error(
          `Scenario "${definition.name}", ${step.phase} step ${index + 1} (${step.action.kind}) failed`,
          {cause: error},
        )
      }
    }
  }

  /** Executes one serializable action with the state accumulated by earlier steps. */
  private async execute(action: ScenarioAction): Promise<void> {
    switch (action.kind) {
      case "network.localnet":
        await this.provider.activateLocalnet()
        return
      case "wallet.open":
        await this.wallet.open()
        return
      case "wallet.create":
        await this.wallet.create()
        return
      case "wallet.acceptRecovery":
        await this.wallet.acceptRecovery()
        return
      case "wallet.reloadDashboard":
        await this.wallet.reloadDashboard()
        return
      case "wallet.refresh":
        await this.wallet.refresh()
        return
      case "wallet.openTonConnect":
        await this.wallet.openTonConnectPanel()
        return
      case "wallet.closeDialog":
        await this.wallet.closeDialog()
        return
      case "wallet.handleConnectLink":
        await this.wallet.openConnection(this.requiredConnectLink())
        return
      case "wallet.approveConnect":
        await this.wallet.approveConnection()
        return
      case "wallet.approveRequest":
        await this.wallet.approveRequest()
        return
      case "wallet.rejectConnect":
        await this.wallet.rejectConnection()
        return
      case "wallet.rejectRequest":
        await this.wallet.rejectRequest()
        return
      case "dapp.start":
        await this.requiredTonConnect().startDapp(action.config)
        return
      case "dapp.createConnectLink": {
        const result: {readonly link: string} = await this.requiredDapp().command({type: "connect"})
        this.connectLink = result.link
        return
      }
      case "dapp.requestTransaction":
        this.lastTransaction = await this.renderTransaction(action.config)
        await this.requiredDapp().command({
          transaction: this.lastTransaction,
          type: "send_transaction",
        })
        return
      case "expect.ui.welcome":
        await expect(this.context.page.getByRole("heading", {name: "Create wallet"})).toBeVisible()
        await expect(this.context.page.getByRole("button", {name: "Create wallet"})).toBeEnabled()
        return
      case "expect.ui.recovery":
        await expect(
          this.context.page.getByRole("heading", {name: "Back up your wallet"}),
        ).toBeVisible()
        await expect(this.wallet.recoveryWords().getByRole("listitem")).toHaveCount(12)
        await expect(this.context.page.getByRole("button", {name: "Continue"})).toBeDisabled()
        return
      case "expect.ui.dashboard":
        await expect(this.context.page.getByText("My wallet")).toBeVisible()
        await expect(this.context.page.getByText("Testnet", {exact: true})).toBeVisible()
        await expect(this.context.page.getByText(EXPECTED_BALANCE)).toBeVisible()
        await expect(
          this.context.page.getByRole("heading", {name: "Recent activity"}),
        ).toBeVisible()
        return
      case "expect.ui.activity":
        await this.assertActivity(action.expectation)
        return
      case "expect.ui.connectApproval":
        await expect(
          this.wallet.dialog().getByRole("heading", {name: action.dappName}),
        ).toBeVisible()
        await expect(
          this.wallet.dialog().getByText("wants to connect to your wallet"),
        ).toBeVisible()
        await expect(this.wallet.dialog().getByText("View your wallet address")).toBeVisible()
        await expect(this.wallet.dialog().getByRole("button", {name: "Connect"})).toBeEnabled()
        return
      case "expect.ui.connectedDapp":
        await expect(
          this.wallet.dialog().getByRole("heading", {name: action.dappName}),
        ).toBeVisible()
        await expect(
          this.wallet.dialog().getByText("This app is connected to your wallet."),
        ).toBeVisible()
        await expect(this.wallet.dialog().getByRole("button", {name: "Disconnect"})).toBeEnabled()
        return
      case "expect.ui.tonConnectEntry":
        await expect(
          this.wallet.dialog().getByRole("heading", {name: "Paste a connection link"}),
        ).toBeVisible()
        await expect(this.wallet.dialog().getByLabel("TON Connect link")).toHaveValue("")
        await expect(this.wallet.dialog().getByRole("button", {name: "Continue"})).toBeVisible()
        return
      case "expect.ui.transaction":
        await this.assertTransactionUi(action.messages)
        return
      case "expect.dapp.connected":
        await this.assertDappConnected(action.network)
        return
      case "expect.dapp.connectionRejected":
        await this.assertConnectionRejected()
        return
      case "expect.dapp.transactionApproved":
        await this.assertTransactionApproved()
        return
      case "expect.dapp.transactionRejected":
        await this.assertTransactionRejected()
        return
      case "expect.screenshot":
        await this.assertScreenshot(action.name, action.target)
        return
      default:
        assertNever(action)
    }
  }

  /** Resolves dynamic sender and validity fields after the dApp has connected. */
  private async renderTransaction(config: TransactionConfig): Promise<RenderedTransaction> {
    const account = (await this.requiredDapp().state()).account
    if (config.fromConnectedWallet === true && account === null) {
      throw new Error("The dApp has no connected wallet account")
    }
    return {
      ...(config.fromConnectedWallet === true ? {from: account?.address} : {}),
      messages: config.messages,
      network: config.network,
      validUntil: config.validUntil,
    }
  }

  /** Checks every transaction message rendered in the approval dialog. */
  private async assertTransactionUi(messages: readonly TransactionMessageConfig[]): Promise<void> {
    const dialog: Locator = this.wallet.dialog()
    const dappName: string = this.requiredDapp().config.manifest.name
    try {
      await expect(dialog.getByRole("heading", {name: `${dappName} wants to send`})).toBeVisible()
    } catch (cause) {
      const state: DappActorState = await this.requiredDapp().state()
      throw new Error(`Transaction dialog did not open; dApp state: ${JSON.stringify(state)}`, {
        cause,
      })
    }
    for (const [index, message] of messages.entries()) {
      await expect(dialog.getByText(`Message ${index + 1} of ${messages.length}`)).toBeVisible()
      await expect(
        dialog.getByText(`${formatNanogramBalance(message.amount)} GRAM`, {exact: true}),
      ).toBeVisible()
    }
    await expect(dialog.getByText("To", {exact: true})).toHaveCount(messages.length)
    await expect(dialog.getByText("StateInit", {exact: true})).toHaveCount(messages.length)
    const summary: Locator = dialog.getByText("Message BOC", {exact: true})
    const confirm: Locator = dialog.getByRole("button", {name: "Confirm"})
    await expect(summary).toBeVisible()
    await expect(confirm).toBeEnabled()
    await confirm.scrollIntoViewIfNeeded()
    await expect(summary).toBeInViewport()
    await expect(confirm).toBeInViewport()
  }

  /** Checks account, config, protocol version, and capabilities observed by the dApp SDK. */
  private async assertDappConnected(network: string): Promise<void> {
    const actor: DappActor = this.requiredDapp()
    const state: DappActorState = await actor.waitFor(
      candidate => candidate.status === "connected",
      "connection",
    )
    expect(state.config).toEqual(actor.config)
    expect(state.error).toBeNull()
    expect(state.account).toEqual({
      address: expect.any(String),
      chain: network,
      publicKey: expect.any(String),
      walletStateInit: expect.any(String),
    })
    expect(state.device).toEqual({
      appName: "tonkeeper",
      appVersion: "0.1.0",
      features: [
        {extraCurrencySupported: false, maxMessages: 255, name: "SendTransaction"},
        {extraCurrencySupported: false, maxMessages: 255, name: "SignMessage"},
      ],
      maxProtocolVersion: 2,
      platform: "browser",
    })
    expect(journalTypes(state)).toEqual(
      expect.arrayContaining(["connect_link_created", "wallet_connected"]),
    )
  }

  /** Checks the terminal SDK error and empty account state after connection rejection. */
  private async assertConnectionRejected(): Promise<void> {
    const actor: DappActor = this.requiredDapp()
    const state: DappActorState = await actor.waitFor(
      candidate => candidate.status === "error",
      "connection rejection",
    )
    expect(state.config).toEqual(actor.config)
    expect(state.account).toBeNull()
    expect(state.device).toBeNull()
    expect(state.error?.name).toBe("UserRejectsError")
    expect(state.error?.message).toContain("User rejects the action in the wallet.")
    expect(state.error?.message).toContain("User declined the connection")
    expect(journalTypes(state)).toEqual(
      expect.arrayContaining(["connect_link_created", "connector_error"]),
    )
  }

  /** Checks the exact request and protocol error observed by the dApp after rejection. */
  private async assertTransactionRejected(): Promise<void> {
    const transaction: RenderedTransaction = this.requiredLastTransaction()
    const state: DappActorState = await this.requiredDapp().waitFor(
      candidate => candidate.transaction.status === "error",
      "transaction rejection",
    )
    expect(state.transaction.request).toEqual(transaction)
    expect(state.transaction.result).toBeNull()
    expect(state.transaction.error?.name).toBe("UserRejectsError")
    expect(state.transaction.error?.message).toContain("User rejects the action in the wallet.")
    expect(state.transaction.error?.message).toContain("User declined the request")
    expect(journalTypes(state)).toEqual(
      expect.arrayContaining(["transaction_requested", "transaction_sent", "transaction_failed"]),
    )
  }

  /** Checks the exact request and non-empty protocol result returned after approval. */
  private async assertTransactionApproved(): Promise<void> {
    const transaction: RenderedTransaction = this.requiredLastTransaction()
    const state: DappActorState = await this.requiredDapp().waitFor(
      candidate => candidate.transaction.status === "success",
      "transaction approval",
    )
    expect(state.transaction.request).toEqual(transaction)
    expect(state.transaction.result).not.toBeNull()
    expect(state.transaction.error).toBeNull()
    expect(journalTypes(state)).toEqual(
      expect.arrayContaining([
        "transaction_requested",
        "transaction_sent",
        "transaction_succeeded",
      ]),
    )
  }

  /** Checks row count, order, values, uniqueness, and relationships to remembered history. */
  private async assertActivity(expectation: ActivityExpectation): Promise<void> {
    const observation: WebActivityObservation = await this.wallet.observeActivity(expectation.count)
    expect(new Set(observation.ids).size).toBe(observation.ids.length)
    expect(observation.ids.every(id => id.length > 0)).toBe(true)
    if (expectation.amounts !== undefined) {
      expect(observation.amounts).toEqual(expectation.amounts)
    }
    if (expectation.directions !== undefined) {
      expect(observation.directions).toEqual(expectation.directions)
    }
    if (expectation.sameAs !== undefined) {
      expect(observation.ids).toEqual(this.requiredActivity(expectation.sameAs).ids)
    }
    if (expectation.extends !== undefined) {
      const previous: readonly string[] = this.requiredActivity(expectation.extends).ids
      expect(observation.ids.length).toBeGreaterThan(previous.length)
      expect(observation.ids.slice(-previous.length)).toEqual(previous)
    }
    if (expectation.rememberAs !== undefined) {
      this.rememberedActivity.set(expectation.rememberAs, observation)
    }
  }

  /** Compares a stable page or dialog region with its committed visual baseline. */
  private async assertScreenshot(
    name: string,
    target: "dialog" | "page" | "recovery",
  ): Promise<void> {
    if (this.context.testInfo.project.metadata.compareScreenshots !== true) {
      return
    }
    if (target === "dialog") {
      await expect(this.wallet.dialog()).toHaveScreenshot(`${name}.png`)
      return
    }
    const masks: Locator[] =
      target === "recovery" ? [this.wallet.recoveryWords()] : [this.wallet.walletAddress()]
    await expect(this.context.page).toHaveScreenshot(`${name}.png`, {mask: masks})
  }

  /** Returns the configured TON Connect fixture or reports a malformed scenario. */
  private requiredTonConnect(): TonConnectHarness {
    if (this.context.tonConnect === undefined) {
      throw new Error("This scenario requires the TON Connect fixture")
    }
    return this.context.tonConnect
  }

  /** Returns the running dApp actor or reports a missing setup step. */
  private requiredDapp(): DappActor {
    return this.requiredTonConnect().dapp()
  }

  /** Returns the connect link created by the preceding dApp action. */
  private requiredConnectLink(): string {
    if (this.connectLink === undefined) {
      throw new Error("The dApp has not created a TON Connect link")
    }
    return this.connectLink
  }

  /** Returns the rendered request retained for dApp-side equality checks. */
  private requiredLastTransaction(): RenderedTransaction {
    if (this.lastTransaction === undefined) {
      throw new Error("The dApp has not requested a transaction")
    }
    return this.lastTransaction
  }

  /** Returns a preceding activity observation used by an ordering or stability assertion. */
  private requiredActivity(name: string): WebActivityObservation {
    const observation: WebActivityObservation | undefined = this.rememberedActivity.get(name)
    if (observation === undefined) {
      throw new Error(`No activity observation was remembered as ${name}`)
    }
    return observation
  }
}

/** Selects the deterministic or real provider backend used by one Web scenario. */
class ProviderControl {
  private readonly origin: string
  private readonly request: APIRequestContext

  /** Uses the Playwright request context so the local TLS certificate remains trusted. */
  constructor(request: APIRequestContext, origin: string) {
    this.request = request
    this.origin = origin
  }

  /** Restores the scripted backend before each isolated scenario starts. */
  async reset(): Promise<void> {
    await this.select("scripted")
  }

  /** Starts a fresh Acton localnet and routes subsequent wallet requests to it. */
  async activateLocalnet(): Promise<void> {
    await this.select("localnet")
  }

  /** Sends one provider-mode command and rejects an unsuccessful harness response. */
  private async select(mode: "localnet" | "scripted"): Promise<void> {
    const response = await this.request.post(`${this.origin}/e2e/provider`, {data: {mode}})
    if (!response.ok()) {
      throw new Error(
        `Provider mode ${mode} failed with HTTP ${response.status()}: ${await response.text()}`,
      )
    }
  }
}

/** Returns journal event names without discarding their original order. */
function journalTypes(state: DappActorState): string[] {
  return state.journal.map(event => event.type)
}

/** Fails compilation when a newly added DSL action has no Web interpretation. */
function assertNever(value: never): never {
  throw new Error(`Unsupported scenario action: ${JSON.stringify(value)}`)
}
