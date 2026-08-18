import {Base64, SessionCrypto, hexToByteArray} from "@tonconnect/protocol"
import {describe, expect, test} from "bun:test"

import {
  TonConnectWallet,
  isNewerRequestId,
  parseTonConnectLink,
  type TonConnectStorage,
} from "../src/ton-connect"
import {deviceInfo} from "../src/ton-connect-protocol"
import {prepareTransaction} from "../src/ton-connect-transaction"
import type {WalletClient} from "../src/wallet-client"
import type {WalletLifecycle} from "../src/wallet-lifecycle"

const CLIENT_ID: string = "01".repeat(32)
const WALLET_IDENTITY = {appName: "tonkeeper", appVersion: "0.1.0"} as const

function connectLink(clientId: string = CLIENT_ID): string {
  const parameters = new URLSearchParams({
    v: "2",
    id: clientId,
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
      identity: WALLET_IDENTITY,
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
      identity: WALLET_IDENTITY,
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

  test("emulates the exact request and forwards an approved force override", async () => {
    const dappCrypto = new SessionCrypto()
    const order: string[] = []
    const previewRequests: unknown[] = []
    const sendRequests: unknown[] = []
    let walletClientId: string | undefined
    let requestSent: boolean = false
    const fetch = Object.assign(
      async (input: string | URL | Request): Promise<Response> => {
        const url = new URL(input instanceof Request ? input.url : input.toString())
        if (url.hostname === "app.example") {
          return Response.json({
            url: "https://app.example",
            name: "Example dApp",
            iconUrl: "https://app.example/icon.png",
          })
        }
        if (url.pathname.endsWith("/message")) {
          walletClientId = url.searchParams.get("client_id") ?? undefined
          return new Response(undefined, {status: 200})
        }
        if (!requestSent && walletClientId) {
          requestSent = true
          const request = {
            method: "sendTransaction",
            id: "7",
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
                    stateInit: "te6ccgEBAQEAAgAAAA==",
                  },
                ],
              }),
            ],
          }
          const encrypted = dappCrypto.encrypt(
            JSON.stringify(request),
            hexToByteArray(walletClientId),
          )
          const envelope = JSON.stringify({
            from: dappCrypto.sessionId,
            message: Base64.encode(encrypted),
          })
          return new Response(`id: 1\nevent: message\ndata: ${envelope}\n\n`, {
            status: 200,
            headers: {"content-type": "text/event-stream"},
          })
        }
        return new Response(undefined, {status: 200})
      },
      {preconnect: () => undefined},
    ) as typeof globalThis.fetch
    const walletClient = {
      previewTonConnect: async (request: unknown) => {
        order.push("preview")
        previewRequests.push(request)
        return {
          message: {
            destination: "EQDestination",
            amount: {kind: "exact", nanograms: "1000000"},
            body: {kind: "rawPayload", boc: "te6ccgEBAQEAAgAAAA=="},
            stateInit: "te6ccgEBAQEAAgAAAA==",
          },
          validUntil: 1_900_000_000,
          messageBocBase64: "te6ccgEBAQEAAgAAAA==",
          emulation: {
            mcBlockSeqno: 1,
            walletFeesNanograms: "11",
            traceFeesNanograms: "18",
            transactionCount: 2,
            actions: [],
            traceSucceeded: true,
            isIncomplete: false,
          },
        }
      },
      send: async (request: unknown) => {
        order.push("send")
        sendRequests.push(request)
        return {
          operationId: "ton-connect-operation",
          messageHash: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
          signedBoc: "te6ccgEBAQEAAgAAAA==",
          phase: "submitted",
        }
      },
    } as unknown as WalletClient
    const lifecycle = {
      tonConnectAccount: () => ({
        address: `0:${"11".repeat(32)}`,
        network: "-3",
        walletStateInit: "state-init",
        publicKey: Array.from({length: 32}, () => 1),
      }),
    } as unknown as WalletLifecycle
    const wallet = new TonConnectWallet({
      descriptor: {
        recordId: "preview-wallet",
        address: "0QWallet",
        publicKey: Array.from({length: 32}, () => 1),
        network: "testnet",
        secretRef: {value: "wallet-secret"},
      },
      walletClient,
      lifecycle,
      identity: WALLET_IDENTITY,
      bridgeUrl: "https://bridge.example/bridge",
      fetch,
      storage: new MemoryTonConnectStorage(undefined),
    })
    let transactionPreview: unknown
    wallet.onEvent(event => {
      if (event.kind !== "interaction") {
        return
      }
      if (event.interaction.kind === "connect") {
        wallet.respond(event.interaction.id, true)
      } else {
        order.push("interaction")
        transactionPreview = event.interaction.preview
        wallet.respond(event.interaction.id, true, true)
      }
    })

    await wallet.start(connectLink(dappCrypto.sessionId))
    await Bun.sleep(20)

    expect(order).toEqual(["preview", "interaction", "send"])
    expect(previewRequests).toEqual([
      expect.objectContaining({
        intent: {
          expiration: {kind: "exact", unixTimestamp: 1_900_000_000},
          message: expect.objectContaining({
            body: {kind: "rawPayload", boc: "te6ccgEBAQEAAgAAAA=="},
            stateInit: "te6ccgEBAQEAAgAAAA==",
          }),
        },
      }),
    ])
    expect(transactionPreview).toEqual(
      expect.objectContaining({
        validUntil: 1_900_000_000,
        emulation: expect.objectContaining({transactionCount: 2}),
      }),
    )
    expect(sendRequests).toEqual([
      expect.objectContaining({
        force: true,
        intent: expect.objectContaining({
          expiration: {kind: "exact", unixTimestamp: 1_900_000_000},
        }),
      }),
    ])
    await wallet.close()
  })
})

describe("TON Connect transaction mapping", () => {
  test("uses the canonical browser DeviceInfo names", () => {
    expect(deviceInfo(WALLET_IDENTITY)).toEqual({
      platform: "browser",
      appName: "tonkeeper",
      appVersion: "0.1.0",
      maxProtocolVersion: 2,
      features: [{name: "SendTransaction", maxMessages: 1, extraCurrencySupported: false}],
    })
  })

  test("rejects an empty wallet identity", () => {
    expect(() => deviceInfo({appName: "", appVersion: "0.1.0"})).toThrow(
      "wallet identity is invalid",
    )
    expect(() => deviceInfo({appName: "tonkeeper", appVersion: " "})).toThrow(
      "wallet identity is invalid",
    )
  })

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
      expect(prepared.sendRequest.intent.expiration).toEqual({
        kind: "exact",
        unixTimestamp: 1_900_000_000,
      })
      expect(prepared.sendRequest.intent.message.body).toEqual({
        kind: "rawPayload",
        boc: "te6ccgEBAQEAAgAAAA==",
      })
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

  test("rejects SDK-only camelCase transaction names on the wire", () => {
    const base = {
      network: "-3",
      messages: [{address: "EQDestination", amount: "1000000"}],
    }
    const payloads = [
      {...base, validUntil: 1_900_000_000},
      {
        ...base,
        messages: [{...base.messages[0], extraCurrency: {1: "5"}}],
      },
    ]
    for (const payload of payloads) {
      const prepared = prepareTransaction({
        request: {
          method: "sendTransaction",
          id: "1",
          params: [JSON.stringify(payload)],
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
      expect(prepared).toMatchObject({ok: false, code: 1})
    }
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

  constructor(value: string | undefined) {
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
