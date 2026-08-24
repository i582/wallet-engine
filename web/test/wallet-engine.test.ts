import {afterAll, beforeAll, describe, expect, test} from "bun:test"

import {
  BrowserHttpHost,
  BrowserPlatformHost,
  WalletClient,
  WalletLifecycle,
  convertTonAddress,
  initializeWalletEngine,
  isValidTonAddress,
  mnemonicWordlist,
  parseTonAddress,
  type HttpRequest,
  type NftTransferPreviewRequest,
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

  test("parses, validates, and converts TON address formats", async () => {
    const raw = "0:ca6e321c7cce9ecedf0a8ca2492ec8592494aa5fb5ce0387dff96ef6af982a3e"
    const friendly = "0QDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPleK"

    expect(await parseTonAddress(friendly)).toEqual({
      raw,
      workchain: 0,
      format: {kind: "userFriendly", bounceable: false, testnet: true},
    })
    expect(await convertTonAddress(friendly, {kind: "raw"})).toBe(raw)
    expect(
      await convertTonAddress(raw, {
        kind: "userFriendly",
        bounceable: true,
        testnet: false,
      }),
    ).toBe("EQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPrHF")
    expect(await isValidTonAddress(friendly)).toBe(true)
    expect(await isValidTonAddress("not-an-address")).toBe(false)
  })

  test("exports the complete BIP-39 wordlist", async () => {
    const words = await mnemonicWordlist()

    expect(words).toHaveLength(2048)
    expect(words[0]).toBe("abandon")
    expect(words.at(-1)).toBe("zoo")
  })

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

  test("preserves NFT and collection metadata at the WASM boundary", async () => {
    const lifecycle = await WalletLifecycle.create(platform)
    lifecycles.push(lifecycle)
    const created = await lifecycle.createWallet({
      recordId: "inline-nft-wallet",
      network: "testnet",
    })
    const itemAddress = `0:${"2B".repeat(32)}`
    const collectionAddress = `0:${"3C".repeat(32)}`
    const inlineSvg = "data:image/svg+xml,%3Csvg%2F%3E"
    const client = await WalletClient.create(walletConfig(created.descriptor), {
      platformHost: platform,
      fetch: mockFetch(async input => {
        expect(String(input)).toContain("/api/v3/nft/items?")
        return Response.json({
          nft_items: [
            {
              address: itemAddress,
              code_hash: "code",
              collection: {
                address: collectionAddress,
                collection_content: {
                  description: "A collection from the chain.",
                  image: "ipfs://collection/image.png",
                },
              },
              content: {
                description: "Few have witnessed such magnificence.",
                image: inlineSvg,
                name: "Shadow Reaper",
              },
              data_hash: "data",
              index: "0",
              init: true,
              last_transaction_lt: "90751083000003",
              on_sale: false,
              owner_address: created.descriptor.address,
              real_owner: created.descriptor.address,
            },
          ],
          metadata: {
            [collectionAddress]: {
              token_info: [{name: "Nightfall", type: "nft_collections"}],
            },
          },
        })
      }),
    })
    clients.push(client)

    const update = await client.refreshNfts()
    const [item] = update.snapshot.nfts.items
    const resolvedCollectionAddress = await convertTonAddress(collectionAddress, {
      kind: "userFriendly",
      bounceable: false,
      testnet: true,
    })

    expect(update.outcome).toBe("completed")
    expect(item?.content.name).toBe("Shadow Reaper")
    expect(item?.content.description).toBe("Few have witnessed such magnificence.")
    expect(item?.content.image).toBe(inlineSvg)
    expect(item?.collectionAddress).toBe(resolvedCollectionAddress)
    expect(item?.collection?.address).toBe(resolvedCollectionAddress)
    expect(item?.collection?.name).toBe("Nightfall")
    expect(item?.collection?.description).toBe("A collection from the chain.")
    expect(item?.collection?.image).toBe("ipfs://collection/image.png")
  })

  test("accepts the camel-case exact expiration field at the WASM boundary", async () => {
    const lifecycle = await WalletLifecycle.create(platform)
    lifecycles.push(lifecycle)
    const created = await lifecycle.createWallet({
      recordId: "exact-expiration-wallet",
      network: "testnet",
    })
    const client = await WalletClient.create(walletConfig(created.descriptor), {
      platformHost: platform,
      fetch: mockFetch(async () => {
        throw new TypeError("offline in test")
      }),
    })
    clients.push(client)

    let diagnostic: string = ""
    try {
      await client.previewTonConnect({
        operationId: "exact-expiration-preview",
        intent: {
          expiration: {kind: "exact", unixTimestamp: 1_900_000_000},
          messages: [
            {
              destination: created.descriptor.address,
              amount: {kind: "exact", nanograms: "1"},
              body: {kind: "empty"},
            },
          ],
        },
      })
    } catch (cause) {
      diagnostic = cause instanceof Error ? cause.message : String(cause)
    }

    expect(diagnostic).not.toContain("missing field `unix_timestamp`")
  })

  test("requires both exact NFT funding values at the WASM boundary", async () => {
    const lifecycle = await WalletLifecycle.create(platform)
    lifecycles.push(lifecycle)
    const created = await lifecycle.createWallet({
      recordId: "nft-funding-wallet",
      network: "testnet",
    })
    let fetchCount: number = 0
    const client = await WalletClient.create(walletConfig(created.descriptor), {
      platformHost: platform,
      fetch: mockFetch(async () => {
        fetchCount += 1
        throw new TypeError("request must not reach HTTP")
      }),
    })
    clients.push(client)

    const malformed = {
      operationId: "nft-funding-preview",
      intent: {
        nftAddress: created.descriptor.address,
        recipient: created.descriptor.address,
        funding: {kind: "exact", attachedNanograms: "50000000"},
        payload: {kind: "empty"},
        expiration: {kind: "engineDefault"},
      },
    } as unknown as NftTransferPreviewRequest

    let diagnostic: string = ""
    try {
      await client.previewNftTransfer(malformed)
    } catch (cause) {
      diagnostic = cause instanceof Error ? cause.message : String(cause)
    }

    expect(diagnostic).toContain("forwardNanograms")
    expect(fetchCount).toBe(0)
  })

  test("creates, reveals, and deletes a wallet through the platform host", async () => {
    const lifecycle = await WalletLifecycle.create(platform)
    lifecycles.push(lifecycle)

    const created = await lifecycle.createWallet({
      recordId: "browser-lifecycle-wallet",
      network: "testnet",
    })
    expect(created.recoveryPhrase.phrase.split(" ")).toHaveLength(12)
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

  test("creates and decrypts TON encrypted comments through the WASM boundary", async () => {
    const secrets = new RecordingSecrets()
    const encryptedPlatform = new BrowserPlatformHost({
      secrets,
      journal: new MemoryJournal(),
    })
    const lifecycle = await WalletLifecycle.create(encryptedPlatform)
    lifecycles.push(lifecycle)
    const created = await lifecycle.createWallet({
      recordId: "encrypted-comment-wallet",
      network: "testnet",
    })
    const peerPublicKey = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c"
    const client = await WalletClient.create(
      {
        ...walletConfig(created.descriptor),
        localSecretRef: created.descriptor.secretRef,
      },
      {
        platformHost: encryptedPlatform,
        fetch: mockFetch(async () =>
          Response.json({result: {stack: [["num", `0x${peerPublicKey}`]]}}),
        ),
      },
    )
    clients.push(client)

    const body = await client.createEncryptedComment({
      recipient: `0:${"22".repeat(32)}`,
      comment: "private hello",
    })
    const comment = await client.decryptComment({
      sender: created.descriptor.address,
      body,
    })

    expect(comment).toBe("private hello")
    expect(secrets.reasons).toEqual(["encryptComment", "decryptComment"])
  })
})

class RecordingSecrets extends MemorySecrets {
  readonly reasons: string[] = []

  override async read(request: Parameters<MemorySecrets["read"]>[0]): Promise<Uint8Array> {
    this.reasons.push(request.reason)
    return super.read(request)
  }
}

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
