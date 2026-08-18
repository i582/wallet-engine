import {afterAll, beforeAll, describe, expect, test} from "bun:test"

import {
  BrowserHttpHost,
  BrowserPlatformHost,
  WalletClient,
  WalletLifecycle,
  initializeWalletEngine,
  type HttpRequest,
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
  test("injects the Toncenter API key only into its configured origin", async () => {
    let observedKey: string | null = null
    const host = new BrowserHttpHost("https://testnet.toncenter.com", {
      toncenterApiKey: "secret-value",
      fetch: mockFetch(async (_input, init) => {
        observedKey = new Headers(init?.headers).get("X-API-Key")
        return new Response(new Uint8Array([1, 2, 3]), {
          status: 200,
          headers: {"Content-Type": "application/octet-stream"},
        })
      }),
    })

    const response = await host.executeHttp(httpRequest(1))

    expect(observedKey as string | null).toBe("secret-value")
    expect(response.status).toBe(200)
    expect(response.body).toEqual([1, 2, 3])

    await expect(
      host.executeHttp({
        ...httpRequest(2),
        url: "https://toncenter.com/api/v2/getAddressInformation",
      }),
    ).rejects.toMatchObject({kind: "policyViolation"})
  })

  test("honors cancellation that arrives before fetch starts", async () => {
    let fetchCount = 0
    const host = new BrowserHttpHost("https://testnet.toncenter.com", {
      fetch: mockFetch(async () => {
        fetchCount += 1
        return new Response()
      }),
    })

    await host.cancelHttp({value: 7})
    await expect(host.executeHttp(httpRequest(7))).rejects.toMatchObject({kind: "cancelled"})
    expect(fetchCount).toBe(0)
  })

  test("aborts a response that exceeds the browser host limit", async () => {
    const host = new BrowserHttpHost("https://testnet.toncenter.com", {
      fetch: mockFetch(async () => new Response(new Uint8Array(4 * 1024 * 1024 + 1))),
    })

    await expect(host.executeHttp(httpRequest(9))).rejects.toMatchObject({
      kind: "responseTooLarge",
    })
  })
})

describe("BrowserHttpHost timeout policy", () => {
  test("aborts a request at the core-provided deadline and reports timeout", async () => {
    let observedSignal: AbortSignal | null = null
    const host = new BrowserHttpHost("https://testnet.toncenter.com", {
      fetch: mockFetch(
        (_input, init) =>
          new Promise<Response>(() => {
            observedSignal = init?.signal ?? null
          }),
      ),
    })

    await expect(host.executeHttp(httpRequest(10, {timeoutMs: 5}))).rejects.toMatchObject({
      kind: "timeout",
    })
    expect((observedSignal as AbortSignal | null)?.aborted).toBe(true)
  })

  test("rejects an invalid core-provided timeout", async () => {
    const host = new BrowserHttpHost("https://testnet.toncenter.com")

    await expect(host.executeHttp(httpRequest(11, {timeoutMs: 0}))).rejects.toMatchObject({
      kind: "policyViolation",
    })
  })

  test("applies the same deadline while reading the response body", async () => {
    const stalledBody = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new Uint8Array([1]))
      },
    })
    const host = new BrowserHttpHost("https://testnet.toncenter.com", {
      fetch: mockFetch(async () => new Response(stalledBody)),
    })

    await expect(host.executeHttp(httpRequest(12, {timeoutMs: 5}))).rejects.toMatchObject({
      kind: "timeout",
    })
  })
})

describe("high-level WASM API", () => {
  const platform = new BrowserPlatformHost({
    secrets: new MemorySecrets(),
    journal: new MemoryJournal(),
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
    const lifecycle = await WalletLifecycle.create(platform)
    lifecycles.push(lifecycle)
    const created = await lifecycle.createWallet({
      recordId: "refresh-wallet",
      network: "testnet",
    })
    const client = await WalletClient.create(walletConfig(created.descriptor), {
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
    expect(created.recoveryPhrase.phrase.split(" ")).toHaveLength(24)
    expect(created.descriptor.address).toStartWith("0Q")
    expect(created.descriptor.publicKey).toHaveLength(32)

    const account = lifecycle.tonConnectAccount(created.descriptor)
    expect(account.address).toStartWith("0:")
    expect(account.network).toBe("-3")
    expect(account.walletStateInit.length).toBeGreaterThan(16)
    expect(account.publicKey).toEqual(created.descriptor.publicKey)

    const proof = await lifecycle.signTonConnectProof({
      descriptor: created.descriptor,
      domain: "app.example",
      timestamp: 1_800_000_000,
      payload: "single-use challenge",
    })
    expect(proof.signature).toHaveLength(64)

    const revealed = await lifecycle.revealRecoveryPhrase(created.descriptor)
    expect(revealed.phrase).toEqual(created.recoveryPhrase.phrase)

    await lifecycle.deleteWallet(created.descriptor)
    await expect(lifecycle.revealRecoveryPhrase(created.descriptor)).rejects.toBeInstanceOf(Error)
  })
})

function httpRequest(id: number, overrides: Partial<HttpRequest> = {}): HttpRequest {
  return {
    id: {value: id},
    method: "get",
    url: "https://testnet.toncenter.com/api/v2/getAddressInformation",
    headers: [],
    body: [],
    timeoutMs: 15_000,
    ...overrides,
  }
}

function walletConfig(descriptor: {
  readonly recordId: string
  readonly address: string
  readonly publicKey: number[]
}): WalletClientConfig {
  return {
    recordId: descriptor.recordId,
    address: descriptor.address,
    publicKey: descriptor.publicKey,
    network: "testnet",
    sendValiditySeconds: 300,
    resolutionMarginSeconds: 60,
    providers: {
      toncenterBaseUrl: "https://testnet.toncenter.com",
      requestTimeoutMs: 15_000,
    },
  }
}
