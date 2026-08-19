import type {DappActorConfig} from "../fixtures/processes"

export type ScenarioPhase = "given" | "then" | "when"
export type ScreenshotTarget = "dialog" | "page" | "recovery"
export type ActivityDirection = "received" | "sent"

export interface ActivityExpectation {
  readonly amounts?: readonly string[]
  readonly count: number
  readonly directions?: readonly ActivityDirection[]
  readonly extends?: string
  readonly rememberAs?: string
  readonly sameAs?: string
}

export interface TransactionMessageConfig {
  readonly address: string
  readonly amount: string
  readonly payload?: string
  readonly stateInit?: string
}

export interface TransactionConfig {
  readonly fromConnectedWallet?: boolean
  readonly messages: readonly TransactionMessageConfig[]
  readonly network: string
  readonly validUntil: number
}

export type ScenarioAction =
  | {readonly kind: "network.localnet"}
  | {readonly kind: "wallet.open"}
  | {readonly kind: "wallet.create"}
  | {readonly kind: "wallet.acceptRecovery"}
  | {readonly kind: "wallet.reloadDashboard"}
  | {readonly kind: "wallet.refresh"}
  | {readonly kind: "wallet.openTonConnect"}
  | {readonly kind: "wallet.closeDialog"}
  | {readonly kind: "wallet.handleConnectLink"}
  | {readonly kind: "wallet.approveConnect"}
  | {readonly kind: "wallet.approveRequest"}
  | {readonly kind: "wallet.rejectConnect"}
  | {readonly kind: "wallet.rejectRequest"}
  | {readonly config: DappActorConfig; readonly kind: "dapp.start"}
  | {readonly kind: "dapp.createConnectLink"}
  | {readonly config: TransactionConfig; readonly kind: "dapp.requestTransaction"}
  | {readonly kind: "expect.ui.welcome"}
  | {readonly kind: "expect.ui.recovery"}
  | {readonly kind: "expect.ui.dashboard"}
  | {readonly expectation: ActivityExpectation; readonly kind: "expect.ui.activity"}
  | {readonly dappName: string; readonly kind: "expect.ui.connectApproval"}
  | {readonly dappName: string; readonly kind: "expect.ui.connectedDapp"}
  | {readonly kind: "expect.ui.tonConnectEntry"}
  | {readonly messages: readonly TransactionMessageConfig[]; readonly kind: "expect.ui.transaction"}
  | {readonly kind: "expect.dapp.connected"; readonly network: string}
  | {readonly kind: "expect.dapp.connectionRejected"}
  | {readonly kind: "expect.dapp.transactionApproved"}
  | {readonly kind: "expect.dapp.transactionRejected"}
  | {readonly kind: "expect.screenshot"; readonly name: string; readonly target: ScreenshotTarget}

export interface ScenarioStep {
  readonly action: ScenarioAction
  readonly phase: ScenarioPhase
}

export interface ScenarioDefinition {
  readonly name: string
  readonly steps: readonly ScenarioStep[]
}

/** Builds a serializable client scenario without binding it to Playwright. */
export class ScenarioBuilder {
  private readonly name: string
  private readonly steps: ScenarioStep[] = []

  /** Creates an empty definition with the name reported by every platform runner. */
  constructor(name: string) {
    this.name = name
  }

  /** Adds state that must exist before user interaction starts. */
  given(action: ScenarioAction): this {
    this.steps.push({action, phase: "given"})
    return this
  }

  /** Adds one user or dApp action to the scenario. */
  when(action: ScenarioAction): this {
    this.steps.push({action, phase: "when"})
    return this
  }

  /** Adds one externally observable expectation. */
  expect(action: ScenarioAction): this {
    this.steps.push({action, phase: "then"})
    return this
  }

  /** Returns the plain definition consumed by platform-specific runners. */
  build(): ScenarioDefinition {
    return {name: this.name, steps: [...this.steps]}
  }
}

/** Starts a named scenario definition. */
export function scenario(name: string): ScenarioBuilder {
  return new ScenarioBuilder(name)
}

/** Defines the real blockchain environment required by a client scenario. */
export function network(): {
  localnet: () => ScenarioAction
} {
  return {
    localnet: () => ({kind: "network.localnet"}),
  }
}

/** Defines wallet lifecycle and approval actions. */
export function wallet(): {
  acceptRecovery: () => ScenarioAction
  approveConnect: () => ScenarioAction
  approveRequest: () => ScenarioAction
  closeDialog: () => ScenarioAction
  create: () => ScenarioAction
  handleConnectLink: () => ScenarioAction
  open: () => ScenarioAction
  openTonConnect: () => ScenarioAction
  rejectConnect: () => ScenarioAction
  rejectRequest: () => ScenarioAction
  refresh: () => ScenarioAction
  reloadDashboard: () => ScenarioAction
} {
  return {
    acceptRecovery: () => ({kind: "wallet.acceptRecovery"}),
    approveConnect: () => ({kind: "wallet.approveConnect"}),
    approveRequest: () => ({kind: "wallet.approveRequest"}),
    closeDialog: () => ({kind: "wallet.closeDialog"}),
    create: () => ({kind: "wallet.create"}),
    handleConnectLink: () => ({kind: "wallet.handleConnectLink"}),
    open: () => ({kind: "wallet.open"}),
    openTonConnect: () => ({kind: "wallet.openTonConnect"}),
    rejectConnect: () => ({kind: "wallet.rejectConnect"}),
    rejectRequest: () => ({kind: "wallet.rejectRequest"}),
    refresh: () => ({kind: "wallet.refresh"}),
    reloadDashboard: () => ({kind: "wallet.reloadDashboard"}),
  }
}

/** Defines dApp setup, connection, and RPC actions. */
export function dapp(): {
  createConnectLink: () => ScenarioAction
  requestTransaction: (config: TransactionConfig) => ScenarioAction
  start: (config: DappActorConfig) => ScenarioAction
} {
  return {
    createConnectLink: () => ({kind: "dapp.createConnectLink"}),
    requestTransaction: config => ({config, kind: "dapp.requestTransaction"}),
    start: config => ({config, kind: "dapp.start"}),
  }
}

/** Defines semantic assertions against the rendered wallet UI. */
export function walletUi(): {
  activity: (expectation: ActivityExpectation) => ScenarioAction
  connectApproval: (dappName: string) => ScenarioAction
  connectedDapp: (dappName: string) => ScenarioAction
  dashboard: () => ScenarioAction
  recovery: () => ScenarioAction
  tonConnectEntry: () => ScenarioAction
  transaction: (messages: readonly TransactionMessageConfig[]) => ScenarioAction
  welcome: () => ScenarioAction
} {
  return {
    activity: expectation => ({expectation, kind: "expect.ui.activity"}),
    connectApproval: dappName => ({dappName, kind: "expect.ui.connectApproval"}),
    connectedDapp: dappName => ({dappName, kind: "expect.ui.connectedDapp"}),
    dashboard: () => ({kind: "expect.ui.dashboard"}),
    recovery: () => ({kind: "expect.ui.recovery"}),
    tonConnectEntry: () => ({kind: "expect.ui.tonConnectEntry"}),
    transaction: messages => ({kind: "expect.ui.transaction", messages}),
    welcome: () => ({kind: "expect.ui.welcome"}),
  }
}

/** Defines assertions against state observed by the official dApp SDK. */
export function dappObserved(): {
  connected: (network: string) => ScenarioAction
  connectionRejected: () => ScenarioAction
  transactionApproved: () => ScenarioAction
  transactionRejected: () => ScenarioAction
} {
  return {
    connected: network => ({kind: "expect.dapp.connected", network}),
    connectionRejected: () => ({kind: "expect.dapp.connectionRejected"}),
    transactionApproved: () => ({kind: "expect.dapp.transactionApproved"}),
    transactionRejected: () => ({kind: "expect.dapp.transactionRejected"}),
  }
}

/** Defines a visual checkpoint for a page or security-sensitive dialog. */
export function screen(name: string, target: ScreenshotTarget = "page"): ScenarioAction {
  return {kind: "expect.screenshot", name, target}
}
