import {afterAll, beforeAll, describe, expect, test} from "bun:test"

import {
  BrowserHttpHost,
  BrowserPlatformHost,
  WalletClient,
  WalletLifecycle,
  initializeWalletEngine,
  type HttpCall,
  type WalletClientConfig,
} from "../src"
import {MemoryJournal} from "./memory-journal"
import {MemorySecrets} from "./memory-secrets"

const wasmPath = new URL("../../bindings/wasm/wallet_engine_bg.wasm", import.meta.url)

function mockFetch(
  implementation: (
    ...args: Parameters<typeof globalThis.fetch>
  ) => ReturnType<typeof globalThis.fetch>,
): typeof globalThis.fetch {
  return Object.assign(implementation, {preconnect: () => undefined})
}

beforeAll(async () => {
  const bytes = await Bun.file(wasmPath).arrayBuffer()
  await initializeWalletEngine(bytes)
})

describe("BrowserHttpHost", () => {
  test("injects a credential only into its exact origin", async () => {
    let observedKey: string | null = null
    const host = new BrowserHttpHost({
      credentialProvider: ({value}) => (value === "test-key" ? "secret-value" : undefined),
      fetch: mockFetch(async (_input, init) => {
        observedKey = new Headers(init?.headers).get("X-API-Key")
        return new Response(new Uint8Array([1, 2, 3]), {
          status: 200,
          headers: {"Content-Type": "application/octet-stream"},
        })
      }),
    })

    const response = await host.executeHttp(
      httpCall(1, {
        credential: {value: "test-key"},
        credentialOrigin: "https://testnet.toncenter.com:443",
      }),
    )

    expect(observedKey as string | null).toBe("secret-value")
    expect(response.status).toBe(200)
    expect(response.body).toEqual([1, 2, 3])
  })

  test("honors cancellation that arrives before fetch starts", async () => {
    let fetchCount = 0
    const host = new BrowserHttpHost({
      fetch: mockFetch(async () => {
        fetchCount += 1
        return new Response()
      }),
    })

    await host.cancelHttp({value: 7})
    await expect(host.executeHttp(httpCall(7))).rejects.toMatchObject({kind: "cancelled"})
    expect(fetchCount).toBe(0)
  })

  test("aborts a response that exceeds the Rust limit", async () => {
    const host = new BrowserHttpHost({
      fetch: mockFetch(async () => new Response(new Uint8Array(32))),
    })
    const call: HttpCall = {...httpCall(9), maxResponseBodyBytes: 8}

    await expect(host.executeHttp(call)).rejects.toMatchObject({
      kind: "responseTooLarge",
    })
  })
})

describe("high-level WASM API", () => {
  const platform = new BrowserPlatformHost({
    secrets: new MemorySecrets(),
    journal: new MemoryJournal(),
    now: () => 1_800_000_000,
  })
  const clients: WalletClient[] = []
  const lifecycles: WalletLifecycle[] = []

  afterAll(async () => {
    await Promise.all(clients.map(client => client.close()))
    for (const lifecycle of lifecycles) {
      lifecycle.close()
    }
  })

  test("Rust awaits JavaScript HTTP callbacks during refresh", async () => {
    let fetchCount = 0
    const client = await WalletClient.create(walletConfig("refresh-wallet"), {
      platformHost: platform,
      fetch: mockFetch(async () => {
        fetchCount += 1
        throw new TypeError("offline in test")
      }),
    })
    clients.push(client)

    const update = await client.refresh()

    expect(fetchCount).toBe(2)
    expect(update.outcome).toBe("failed")
    expect(update.snapshot.accountResource.phase).toBe("failed")
    expect(update.snapshot.accountResource.error?.hostKind).toBe("connectionLost")
  })

  test("creates, reveals, and deletes a wallet through the platform host", async () => {
    const lifecycle = await WalletLifecycle.create(platform)
    lifecycles.push(lifecycle)

    const created = await lifecycle.createWallet({
      recordId: "browser-lifecycle-wallet",
      network: "testnet",
    })
    expect(created.recoveryPhrase.words).toHaveLength(24)
    expect(created.descriptor.address).toStartWith("0Q")

    const revealed = await lifecycle.revealRecoveryPhrase(created.descriptor)
    expect(revealed.words).toEqual(created.recoveryPhrase.words)

    await lifecycle.deleteWallet(created.descriptor)
    await expect(lifecycle.revealRecoveryPhrase(created.descriptor)).rejects.toBeInstanceOf(Error)
  })
})

function httpCall(id: number, overrides: Partial<HttpCall> = {}): HttpCall {
  return {
    id: {value: id},
    method: "get",
    url: "https://testnet.toncenter.com/api/v2/getAddressInformation",
    headers: [],
    body: [],
    maxResponseHeaderBytes: 64 * 1024,
    maxResponseBodyBytes: 4 * 1024 * 1024,
    ...overrides,
  }
}

function walletConfig(recordId: string): WalletClientConfig {
  return {
    recordId,
    address: "0QAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACkT",
    network: "testnet",
    providers: {
      toncenterBaseUrl: "https://testnet.toncenter.com/api/v2",
    },
  }
}
