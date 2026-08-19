import {test as base, expect, type TestInfo} from "@playwright/test"

import {DappActor, type DappActorConfig, OfficialBridge} from "./processes"
import type {WalletRuntimeConfig} from "../../src/lib/runtime-config"

interface TonConnectTestFixtures {
  readonly tonConnect: TonConnectHarness
}

interface TonConnectWorkerFixtures {
  readonly bridge: OfficialBridge
}

/** Owns the bridge and dApp processes used by one browser scenario. */
export class TonConnectHarness {
  private readonly bridge: OfficialBridge
  private dappActor: DappActor | undefined

  /** Creates a per-test dApp harness backed by the worker's bridge. */
  constructor(bridge: OfficialBridge) {
    this.bridge = bridge
  }

  /** Starts the scenario's single dApp with its complete manifest configuration. */
  async startDapp(config: DappActorConfig): Promise<DappActor> {
    if (this.dappActor !== undefined) {
      throw new Error("The scenario already has a TON Connect dApp")
    }
    this.dappActor = await DappActor.start(this.bridge.url, config)
    return this.dappActor
  }

  /** Returns the actor after a dApp fixture step has started it. */
  dapp(): DappActor {
    if (this.dappActor === undefined) {
      throw new Error("The scenario has not started a TON Connect dApp")
    }
    return this.dappActor
  }

  /** Returns bridge output, dApp output, and the final protocol state for diagnostics. */
  async diagnostics(): Promise<string> {
    const dappState: string =
      this.dappActor === undefined
        ? "not started"
        : await this.dappActor
            .state()
            .then(state => JSON.stringify(state, null, 2))
            .catch(error => `unavailable: ${String(error)}`)
    return [
      "=== official bridge ===",
      this.bridge.logs(),
      "=== dApp actor ===",
      this.dappActor?.logs() ?? "not started",
      "=== dApp state ===",
      dappState,
    ].join("\n")
  }

  /** Stops every per-test process owned by this harness. */
  async stop(): Promise<void> {
    await this.dappActor?.stop()
  }
}

export const test = base.extend<TonConnectTestFixtures, TonConnectWorkerFixtures>({
  bridge: [
    // biome-ignore lint/correctness/noEmptyPattern: Playwright requires fixture callbacks to destructure their first argument.
    async ({}, use) => {
      const bridge: OfficialBridge = await OfficialBridge.start()
      try {
        await use(bridge)
      } finally {
        await bridge.stop()
      }
    },
    {scope: "worker"},
  ],
  tonConnect: async ({bridge, page}, use, testInfo) => {
    const runtimeConfig: WalletRuntimeConfig = {tonConnectBridgeUrl: bridge.url}
    await page.addInitScript(config => {
      globalThis.walletEngineConfig = config
    }, runtimeConfig)
    const harness = new TonConnectHarness(bridge)
    try {
      await use(harness)
    } finally {
      if (testFailed(testInfo)) {
        await testInfo.attach("ton-connect-fixtures.log", {
          body: Buffer.from(await harness.diagnostics()),
          contentType: "text/plain",
        })
      }
      await harness.stop()
    }
  },
})

/** Reports whether a fixture should preserve its diagnostics for this test result. */
function testFailed(testInfo: TestInfo): boolean {
  return testInfo.status !== undefined && testInfo.status !== testInfo.expectedStatus
}

export {expect}
