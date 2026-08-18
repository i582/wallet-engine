import {SessionCrypto} from "@tonconnect/protocol"
import {describe, expect, test} from "bun:test"

import {
  TonConnectWallet,
  isNewerRequestId,
  parseTonConnectLink,
  type TonConnectStorage,
} from "../src/ton-connect"
import {prepareTransaction} from "../src/ton-connect-transaction"
import type {WalletClient} from "../src/wallet-client"
import type {WalletLifecycle} from "../src/wallet-lifecycle"

const CLIENT_ID: string = "01".repeat(32)

function connectLink(): string {
  const parameters = new URLSearchParams({
    v: "2",
    id: CLIENT_ID,
    r: JSON.stringify({
      manifestUrl: "https://app.example/tonconnect-manifest.json",
      items: [{name: "ton_addr", network: "-3"}],
    }),
  })
  return `tc://?${parameters.toString()}`
}

describe("TON Connect wallet runtime", () => {
  test("parses a full v2 link without normalizing the request", () => {
    const parsed = parseTonConnectLink(connectLink())
    expect(parsed.peerClientId).toBe(CLIENT_ID)
    expect(parsed.request.items).toEqual([{name: "ton_addr", network: "-3"}])
  })

  test("rejects duplicate singleton fields and reduced links", () => {
    expect(() => parseTonConnectLink(`${connectLink()}&id=${CLIENT_ID}`)).toThrow("exactly one id")
    expect(() => parseTonConnectLink(`tc://?v=2&id=${CLIENT_ID}`)).toThrow("exactly one r")
  })

  test("orders arbitrary precision ids and canonicalizes leading zeroes", () => {
    expect(isNewerRequestId("10", "9")).toBe(true)
    expect(isNewerRequestId("001", "1")).toBe(false)
    expect(isNewerRequestId("184467440737095516160", "99999999999999999999")).toBe(true)
    expect(isNewerRequestId("-1", undefined)).toBe(false)
  })

  test("disconnect wins over an older in-flight cursor persistence", async () => {
    const walletCrypto = new SessionCrypto()
    const dappCrypto = new SessionCrypto()
    const initial = JSON.stringify({
      version: 1,
      keyPair: walletCrypto.stringifyKeypair(),
      peerClientId: dappCrypto.sessionId,
      bridgeUrl: "https://bridge.example/bridge",
      nextWalletEventId: 1,
      manifest: {
        url: "https://app.example",
        name: "Example dApp",
        iconUrl: "https://app.example/icon.png",
      },
      account: {
        address: `0:${"11".repeat(32)}`,
        network: "-3",
        walletStateInit: "state-init",
        publicKey: Array.from({length: 32}, () => 1),
      },
    })
    const storage = new DelayedTonConnectStorage(initial)
    let eventStreamSent: boolean = false
    const fetch = Object.assign(
      async (input: string | URL | Request): Promise<Response> => {
        const url = new URL(input instanceof Request ? input.url : input.toString())
        if (url.pathname.endsWith("/message")) {
          return new Response(undefined, {status: 200})
        }
        if (!eventStreamSent) {
          eventStreamSent = true
          return new Response("id: 2\nevent: message\ndata: heartbeat\n\n", {status: 200})
        }
        return new Response(undefined, {status: 200})
      },
      {preconnect: () => undefined},
    ) as typeof globalThis.fetch
    const wallet = new TonConnectWallet({
      descriptor: {
        recordId: "race-wallet",
        address: "0:wallet",
        publicKey: Array.from({length: 32}, () => 1),
        network: "testnet",
        secretRef: {value: "wallet-secret"},
      },
      walletClient: {} as WalletClient,
      lifecycle: {} as WalletLifecycle,
      bridgeUrl: "https://bridge.example/bridge",
      fetch,
      storage,
    })

    expect(await wallet.restore()).toBe(true)
    await Promise.resolve()
    await wallet.disconnect()
    await Bun.sleep(30)

    expect(storage.value).toBeUndefined()
    await wallet.close()
  })

  test("delivers a persisted response before reading more bridge requests", async () => {
    const walletCrypto = new SessionCrypto()
    const dappCrypto = new SessionCrypto()
    const storage = new MemoryTonConnectStorage(
      JSON.stringify({
        version: 1,
        keyPair: walletCrypto.stringifyKeypair(),
        peerClientId: dappCrypto.sessionId,
        bridgeUrl: "https://bridge.example/bridge",
        nextWalletEventId: 1,
        manifest: {
          url: "https://app.example",
          name: "Example dApp",
          iconUrl: "https://app.example/icon.png",
        },
        account: {
          address: `0:${"11".repeat(32)}`,
          network: "-3",
          walletStateInit: "state-init",
          publicKey: Array.from({length: 32}, () => 1),
        },
        pendingPost: {
          payload: {result: "signed-boc", id: "7"},
          topic: "sendTransaction",
        },
      }),
    )
    const requests: string[] = []
    const fetch = Object.assign(
      async (input: string | URL | Request): Promise<Response> => {
        const url = new URL(input instanceof Request ? input.url : input.toString())
        if (url.pathname.endsWith("/message")) {
          requests.push("response")
          return new Response(undefined, {status: 200})
        }
        requests.push("events")
        return new Response(undefined, {status: 200})
      },
      {preconnect: () => undefined},
    ) as typeof globalThis.fetch
    const wallet = new TonConnectWallet({
      descriptor: {
        recordId: "pending-response-wallet",
        address: "0:wallet",
        publicKey: Array.from({length: 32}, () => 1),
        network: "testnet",
        secretRef: {value: "wallet-secret"},
      },
      walletClient: {} as WalletClient,
      lifecycle: {} as WalletLifecycle,
      bridgeUrl: "https://bridge.example/bridge",
      fetch,
      storage,
    })

    expect(await wallet.restore()).toBe(true)
    await Bun.sleep(10)

    expect(requests.slice(0, 2)).toEqual(["response", "events"])
    expect(JSON.parse(storage.value ?? "null").pendingPost).toBeUndefined()
    await wallet.close()
  })
})

describe("TON Connect transaction mapping", () => {
  test("binds the wallet, validity, payload, and deterministic operation id", () => {
    const prepared = prepareTransaction({
      request: {
        method: "sendTransaction",
        id: "0007",
        params: [
          JSON.stringify({
            network: "-3",
            from: `0:${"11".repeat(32)}`,
            valid_until: 1_900_000_000,
            messages: [
              {
                address: "EQDestination",
                amount: "1000000",
                payload: "te6ccgEBAQEAAgAAAA==",
              },
            ],
          }),
        ],
      },
      account: {
        address: `0:${"11".repeat(32)}`,
        network: "-3",
        walletStateInit: "state-init",
        publicKey: Array.from({length: 32}, () => 1),
      },
      descriptor: {
        recordId: "wallet",
        address: "0QWallet",
        publicKey: Array.from({length: 32}, () => 1),
        network: "testnet",
        secretRef: {value: "wallet-secret"},
      },
      sessionId: "session",
      dappName: "Example dApp",
    })

    expect(prepared.ok).toBe(true)
    if (prepared.ok) {
      expect(prepared.sendRequest.operationId).toBe("ton-connect:session:7")
      expect(prepared.sendRequest.validUntil).toBe(1_900_000_000)
      expect(prepared.sendRequest.payload).toBe("te6ccgEBAQEAAgAAAA==")
    }
  })

  test("rejects multi-message transactions before approval", () => {
    const prepared = prepareTransaction({
      request: {
        method: "sendTransaction",
        id: "1",
        params: [JSON.stringify({messages: [{}, {}]})],
      },
      account: {address: "0:account", network: "-3", walletStateInit: "x", publicKey: []},
      descriptor: {
        recordId: "wallet",
        address: "0QWallet",
        publicKey: [],
        network: "testnet",
        secretRef: {value: "wallet-secret"},
      },
      sessionId: "session",
      dappName: "Example dApp",
    })
    expect(prepared).toMatchObject({ok: false, code: 400})
  })
})

class DelayedTonConnectStorage implements TonConnectStorage {
  value: string | undefined
  private saveCount: number = 0

  constructor(value: string) {
    this.value = value
  }

  async load(): Promise<string | undefined> {
    return this.value
  }

  async save(_key: string, value: string): Promise<void> {
    this.saveCount += 1
    if (this.saveCount === 1) {
      await Bun.sleep(20)
    }
    this.value = value
  }

  async remove(): Promise<void> {
    this.value = undefined
  }
}

class MemoryTonConnectStorage implements TonConnectStorage {
  value: string | undefined

  constructor(value: string) {
    this.value = value
  }

  async load(): Promise<string | undefined> {
    return this.value
  }

  async save(_key: string, value: string): Promise<void> {
    this.value = value
  }

  async remove(): Promise<void> {
    this.value = undefined
  }
}
