import {afterAll, beforeAll, describe, expect, test} from "bun:test"
import {createHash, createHmac, createPrivateKey, createPublicKey, pbkdf2Sync} from "node:crypto"

import {
  BrowserHttpHost,
  BrowserPlatformHost,
  WalletClient,
  WalletLifecycle,
  convertTonAddress,
  detectMnemonicSchemes,
  initializeWalletEngine,
  isValidTonAddress,
  mnemonicWordlist,
  parseTonAddress,
  parseTonTransferLink,
  type CreatedWallet,
  type HttpRequest,
  type NftTransferPreviewRequest,
  type PreparedKeyRotation,
  type WalletClientConfig,
  type WalletDescriptor,
  type WalletStatuslessHost,
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

  test("detects the scheme of entered recovery words", async () => {
    const rotation12 =
      "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    const ton24 =
      "dose ice enrich trigger test dove century still betray gas diet dune " +
      "use other base gym mad law immense village world example praise game"
    const bip3924 =
      "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon " +
      "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art"

    expect(await detectMnemonicSchemes(rotation12.split(" "))).toEqual(["rotation"])
    expect(await detectMnemonicSchemes(ton24.split(" "))).toEqual(["ton"])
    expect(await detectMnemonicSchemes(bip3924.split(" "))).toEqual(["bip39"])
    expect(await detectMnemonicSchemes(["not", "a", "mnemonic"])).toEqual([])
  })

  test("parses strict TON transfer links without form-decoding the query", async () => {
    const recipient = "0:0000000000000000000000000000000000000000000000000000000000000000"
    const parsed = await parseTonTransferLink(
      `ton://transfer/${recipient}?amount=1000000000&text=hello+TON&exp=18446744073709551615`,
    )

    expect(parsed).toEqual({
      recipient,
      asset: {kind: "gram"},
      amount: "1000000000",
      payload: {kind: "text", text: "hello+TON"},
      expiration: {kind: "exact", unixTimestamp: 18446744073709551615n},
    })
    await expect(
      parseTonTransferLink(`ton://transfer/${recipient}?bin=te6ccgEBAQEAAgAAAA==`),
    ).resolves.toMatchObject({payload: {kind: "boc"}})
    await expect(parseTonTransferLink(`ton://transfer/${recipient}?Text=ignored`)).rejects.toThrow(
      "unsupported",
    )
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

  test("routes provider requests through a JavaScript status-less host", async () => {
    let executeCount = 0
    const lifecycle = await WalletLifecycle.create(platform)
    lifecycles.push(lifecycle)
    const created = await lifecycle.createWallet({
      recordId: "statusless-wallet",
      network: "testnet",
    })
    const statuslessHost: WalletStatuslessHost = {
      executeStatusless: async () => {
        executeCount += 1
        throw Object.assign(new Error("relay unavailable"), {
          kind: "connectionLost",
          diagnostic: "relay unavailable",
        })
      },
      cancelStatusless: () => Promise.resolve(),
    }
    const client = await WalletClient.createStatusless(walletConfig(created.descriptor), {
      platformHost: platform,
      statuslessHost,
    })
    clients.push(client)

    const update = await client.refresh()

    expect(executeCount).toBe(2)
    expect(update.outcome).toBe("failed")
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
    expect(proof.publicKey).toEqual(created.descriptor.publicKey)

    let submittedBoc: string | undefined
    const client = await WalletClient.create(
      {
        ...walletConfig(created.descriptor),
        localSecretRef: created.descriptor.secretRef,
      },
      {
        platformHost: platform,
        fetch: mockFetch(async (input, init) => {
          if (String(input).includes("getAddressInformation")) {
            return Response.json({
              ok: true,
              result: {balance: "5000000000", state: "active", sync_utime: 1_800_000_000},
            })
          }
          const body = new TextDecoder().decode(init?.body as Uint8Array)
          const request = JSON.parse(body) as {method: string; params: Record<string, unknown>}
          if (request.method === "runGetMethod") {
            expect(request.params.method).toBe("seqno")
            return Response.json({ok: true, result: {stack: [["num", "0x2a"]]}})
          }
          expect(request.method).toBe("sendBoc")
          submittedBoc = request.params.boc as string
          return Response.json({ok: true, result: {"@type": "ok"}})
        }),
      },
    )
    clients.push(client)

    const preparedTransfer = await client.prepareTransfer({
      operationId: "browser-prepared-transfer",
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
    expect(preparedTransfer).toMatchObject({
      operationId: "browser-prepared-transfer",
      seqno: 42,
      validUntil: 1_900_000_000,
    })
    expect(preparedTransfer.externalBoc.length).toBeGreaterThan(16)
    expect(preparedTransfer.internalBoc.length).toBeGreaterThan(16)
    expect(preparedTransfer.externalBoc).not.toBe(preparedTransfer.internalBoc)
    expect(submittedBoc).toBeUndefined()

    const rotation = await client.prepareKeyRotation({
      validUntil: 1_900_000_000,
      messageKind: "external",
    })
    expect(rotation.replacementRecoveryPhrase.phrase.split(" ")).toHaveLength(24)
    expect(rotation.newPublicKey).toHaveLength(32)
    expect(rotation.signedBoc.length).toBeGreaterThan(16)
    expect(rotation).toMatchObject({
      seqno: 42,
      validUntil: 1_900_000_000,
      messageKind: "external",
    })

    const sendResult = await client.sendBoc({
      operationId: "browser-key-rotation-send",
      force: false,
      signedBoc: rotation.signedBoc,
      seqno: rotation.seqno,
      validUntil: rotation.validUntil,
    })
    expect(sendResult.phase).toBe("submitted")
    expect(sendResult.signedBoc).toBe(rotation.signedBoc)
    expect(submittedBoc).toBe(rotation.signedBoc)

    const revealed = await lifecycle.revealRecoveryPhrase(created.descriptor)
    expect(revealed.phrase).toEqual(created.recoveryPhrase.phrase)

    await lifecycle.deleteWallet(created.descriptor)
    await expect(lifecycle.revealRecoveryPhrase(created.descriptor)).rejects.toBeInstanceOf(Error)
  })

  test("resolves a .ton wallet record with the network-default root", async () => {
    const walletAddress = "EQCD39VS5jcptHL8vMjEXrzGaRcCVYto7HUn4bpAOg8xqB2N"
    const methods: string[] = []
    const lifecycle = await WalletLifecycle.create(platform)
    lifecycles.push(lifecycle)
    const created = await lifecycle.createWallet({
      recordId: "dns-wallet",
      network: "testnet",
    })
    const client = await WalletClient.create(walletConfig(created.descriptor), {
      platformHost: platform,
      fetch: mockFetch(async (_input, init) => {
        const body = new TextDecoder().decode(init?.body as Uint8Array)
        const request = JSON.parse(body) as {method: string; params: Record<string, unknown>}
        methods.push(request.method)
        expect(request.method).toBe("dnsResolve")
        expect(request.params.name).toBe("foundation.ton")
        expect(request.params.address).toBe(
          "-1:efe71d13860afaa6aeaeaf636f9168487f80f1031b0bf8d939ae49d3ea7f7da0",
        )
        return Response.json({
          ok: true,
          result: {
            "@type": "dns.resolved",
            entries: [
              {
                category: "6NRAUIc9uoZap8Fwq0zOZNkIOaNNz9bPcdFOAgVEOxs=",
                entry: {
                  "@type": "dns.entryDataSmcAddress",
                  smc_address: {
                    "@type": "accountAddress",
                    account_address: walletAddress,
                  },
                },
              },
            ],
          },
        })
      }),
    })
    clients.push(client)

    const resolved = await client.resolveDns("Foundation.TON")

    expect(resolved).toStartWith("0Q")
    expect(await convertTonAddress(resolved ?? "", {kind: "raw"})).toBe(
      await convertTonAddress(walletAddress, {kind: "raw"}),
    )
    expect(methods).toEqual(["dnsResolve"])
  })

  test.each(["omitted", "null", "supplied"] as const)(
    "creates and decrypts TON encrypted comments through the WASM boundary (%s recipient key)",
    async keySource => {
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
      let fetchCount = 0
      const client = await WalletClient.create(
        {
          ...walletConfig(created.descriptor),
          localSecretRef: created.descriptor.secretRef,
        },
        {
          platformHost: encryptedPlatform,
          fetch: mockFetch(async input => {
            fetchCount += 1
            return String(input).includes("getAddressInformation")
              ? accountResponse("active")
              : Response.json({result: {stack: [["num", `0x${peerPublicKey}`]]}})
          }),
        },
      )
      clients.push(client)

      const body = await client.createEncryptedComment({
        recipient: keySource === "supplied" ? created.descriptor.address : `0:${"22".repeat(32)}`,
        comment: "private hello",
        ...(keySource === "omitted"
          ? {}
          : {
              recipientPublicKey: keySource === "null" ? null : created.descriptor.publicKey,
            }),
      })
      const comment = await client.decryptComment({
        sender: created.descriptor.address,
        body,
      })

      expect(comment).toBe("private hello")
      // The lookup reads the account state, then calls `get_public_key`.
      expect(fetchCount).toBe(keySource === "supplied" ? 0 : 2)
      expect(secrets.reasons).toEqual(["encryptComment", "decryptComment"])
    },
  )

  test("rejects a mismatched encrypted-comment key before HTTP or secret access", async () => {
    const secrets = new RecordingSecrets()
    const encryptedPlatform = new BrowserPlatformHost({
      secrets,
      journal: new MemoryJournal(),
    })
    const lifecycle = await WalletLifecycle.create(encryptedPlatform)
    lifecycles.push(lifecycle)
    const created = await lifecycle.createWallet({
      recordId: "encrypted-comment-mismatched-key",
      network: "testnet",
    })
    let fetchCount = 0
    const client = await WalletClient.create(
      {
        ...walletConfig(created.descriptor),
        localSecretRef: created.descriptor.secretRef,
      },
      {
        platformHost: encryptedPlatform,
        fetch: mockFetch(async () => {
          fetchCount += 1
          return new Response()
        }),
      },
    )
    clients.push(client)

    await expect(
      client.createEncryptedComment({
        recipient: created.descriptor.address,
        comment: "private hello",
        recipientPublicKey: Array.from(
          Buffer.from("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c", "hex"),
        ),
      }),
    ).rejects.toBeInstanceOf(Error)
    expect(fetchCount).toBe(0)
    expect(secrets.reasons).toEqual([])
  })

  describe("encrypted-comment recipient resolution", () => {
    const peerPublicKey = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c"

    async function resolvingClient(state: string) {
      const secrets = new RecordingSecrets()
      const platform = new BrowserPlatformHost({secrets, journal: new MemoryJournal()})
      const lifecycle = await WalletLifecycle.create(platform)
      lifecycles.push(lifecycle)
      const created = await lifecycle.createWallet({
        recordId: `encrypted-comment-resolution-${state}`,
        network: "testnet",
      })
      const urls: string[] = []
      const client = await WalletClient.create(walletConfig(created.descriptor), {
        platformHost: platform,
        fetch: mockFetch(async input => {
          urls.push(String(input))
          return String(input).includes("getAddressInformation")
            ? accountResponse(state)
            : Response.json({
                ok: true,
                result: {exit_code: 0, stack: [["num", `0x${peerPublicKey}`]]},
              })
        }),
      })
      clients.push(client)
      return {client, created, secrets, urls}
    }

    test("reads an active recipient's key without any secret access", async () => {
      const {client, secrets, urls} = await resolvingClient("active")
      const key = await client.resolveEncryptedCommentRecipient({recipient: `0:${"22".repeat(32)}`})
      expect(Buffer.from(key).toString("hex")).toBe(peerPublicKey)
      expect(urls).toHaveLength(2)
      expect(secrets.reasons).toEqual([])
    })

    test("reports an undeployed recipient as unable to receive encrypted comments", async () => {
      const {client, secrets, urls} = await resolvingClient("uninitialized")
      await expect(
        client.resolveEncryptedCommentRecipient({recipient: `0:${"22".repeat(32)}`}),
      ).rejects.toThrow("encrypted comment is unavailable")
      expect(urls).toHaveLength(1)
      expect(secrets.reasons).toEqual([])
    })

    test("verifies a supplied key locally", async () => {
      const {client, created, urls} = await resolvingClient("active")
      const key = await client.resolveEncryptedCommentRecipient({
        recipient: created.descriptor.address,
        recipientPublicKey: created.descriptor.publicKey,
      })
      expect(key).toEqual(created.descriptor.publicKey)
      expect(urls).toEqual([])
    })
  })

  describe("encrypted comments to a rotated wallet", () => {
    // Words 1-12 of a recovery phrase keep the anchor key and the address; every rotation
    // replaces words 13-24, the signing key. This wallet rotated twice, so its phrase no
    // longer holds the key the first rotation installed.
    interface RotatedWallet {
      readonly platform: BrowserPlatformHost
      readonly secrets: RecordingSecrets
      readonly initial: CreatedWallet
      readonly firstRotation: PreparedKeyRotation
      readonly secondRotation: PreparedKeyRotation
      /** The wallet as imported from its phrase after the second rotation. */
      readonly descriptor: WalletDescriptor
      /** The replaced signing key encrypted with the new one, as each rotation publishes it. */
      readonly encryptedOldKeys: {readonly first: Buffer; readonly second: Buffer}
      /** Toncenter v3 `change_wallet_key` actions of both rotations, newest first. */
      readonly actions: unknown[]
      readonly sender: string
      readonly comments: {
        readonly toAnchor: string
        readonly toReplaced: string
        readonly toCurrent: string
        readonly toStranger: string
      }
    }

    const rotationRequest = {validUntil: 1_900_000_000, messageKind: "external"} as const
    const strangerPublicKey = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c"
    const setupAnswers: ChainAnswers = {publicKeyHex: "", actions: []}
    let wallet: RotatedWallet

    beforeAll(async () => {
      wallet = await rotatedWallet()
    })

    async function rotatedWallet(): Promise<RotatedWallet> {
      const secrets = new RecordingSecrets()
      const platform = new BrowserPlatformHost({secrets, journal: new MemoryJournal()})
      const lifecycle = await WalletLifecycle.create(platform)
      lifecycles.push(lifecycle)

      const initial = await lifecycle.createWallet({
        recordId: "rotated-comment-wallet",
        network: "testnet",
      })
      const firstRotation = await rotateKey(platform, initial.descriptor)
      const replaced = await lifecycle.importWallet({
        recordId: "rotated-comment-wallet-1",
        network: "testnet",
        recoveryWords: firstRotation.replacementRecoveryPhrase.phrase.split(" "),
      })
      const secondRotation = await rotateKey(platform, replaced)
      const descriptor = await lifecycle.importWallet({
        recordId: "rotated-comment-wallet-2",
        network: "testnet",
        recoveryWords: secondRotation.replacementRecoveryPhrase.phrase.split(" "),
      })

      const anchorSeed = signingSeed(initial.recoveryPhrase.phrase)
      const replacedSeed = signingSeed(firstRotation.replacementRecoveryPhrase.phrase)
      const currentSeed = signingSeed(secondRotation.replacementRecoveryPhrase.phrase)
      const encryptedOldKeys = {
        first: encryptOldPrivateKey(anchorSeed, replacedSeed),
        second: encryptOldPrivateKey(replacedSeed, currentSeed),
      }
      const rawAddress = await convertTonAddress(initial.descriptor.address, {kind: "raw"})

      // Another wallet encrypts to whichever key the recipient's `get_public_key` reports.
      const senderWallet = await lifecycle.createWallet({
        recordId: "rotated-comment-sender",
        network: "testnet",
      })
      const sender = await connect(platform, senderWallet.descriptor)
      const recipient = initial.descriptor.address
      return {
        platform,
        secrets,
        initial,
        firstRotation,
        secondRotation,
        descriptor,
        encryptedOldKeys,
        actions: [
          changeWalletKeyAction(rawAddress, secondRotation.newPublicKey, encryptedOldKeys.second),
          changeWalletKeyAction(rawAddress, firstRotation.newPublicKey, encryptedOldKeys.first),
        ],
        sender: senderWallet.descriptor.address,
        comments: {
          toAnchor: await encryptTo(sender, recipient, initial.descriptor.publicKey, "anchor"),
          toReplaced: await encryptTo(sender, recipient, firstRotation.newPublicKey, "replaced"),
          toCurrent: await encryptTo(sender, recipient, secondRotation.newPublicKey, "current"),
          toStranger: await encryptTo(
            sender,
            recipient,
            Array.from(Buffer.from(strangerPublicKey, "hex")),
            "stranger",
          ),
        },
      }
    }

    async function connect(
      platform: BrowserPlatformHost,
      descriptor: WalletDescriptor,
    ): Promise<WalletClient> {
      const client = await WalletClient.create(
        {...walletConfig(descriptor), localSecretRef: descriptor.secretRef},
        {platformHost: platform, fetch: toncenterFetch([], setupAnswers)},
      )
      clients.push(client)
      return client
    }

    async function rotateKey(
      platform: BrowserPlatformHost,
      descriptor: WalletDescriptor,
    ): Promise<PreparedKeyRotation> {
      return await (await connect(platform, descriptor)).prepareKeyRotation(rotationRequest)
    }

    async function encryptTo(
      sender: WalletClient,
      recipient: string,
      publicKey: readonly number[],
      comment: string,
    ): Promise<string> {
      setupAnswers.publicKeyHex = Buffer.from(publicKey).toString("hex")
      return await sender.createEncryptedComment({recipient, comment})
    }

    async function currentClient(actions: unknown[]) {
      const urls: string[] = []
      const chain: ChainAnswers = {publicKeyHex: "", actions}
      const client = await WalletClient.create(
        {...walletConfig(wallet.descriptor), localSecretRef: wallet.descriptor.secretRef},
        {platformHost: wallet.platform, fetch: toncenterFetch(urls, chain)},
      )
      clients.push(client)
      return {client, urls, chain, secretReads: wallet.secrets.reasons.length}
    }

    test("publishes each replaced key encrypted as the history fixture reports it", () => {
      const {initial, firstRotation, secondRotation, descriptor, encryptedOldKeys} = wallet

      expect(descriptor.address).toBe(initial.descriptor.address)
      expect(descriptor.publicKey).toEqual(initial.descriptor.publicKey)
      // The seeds derived here, independently of the engine, match the engine's public keys.
      expect(ed25519PublicKey(signingSeed(initial.recoveryPhrase.phrase))).toEqual(
        initial.descriptor.publicKey,
      )
      expect(ed25519PublicKey(signingSeed(firstRotation.replacementRecoveryPhrase.phrase))).toEqual(
        firstRotation.newPublicKey,
      )
      expect(
        ed25519PublicKey(signingSeed(secondRotation.replacementRecoveryPhrase.phrase)),
      ).toEqual(secondRotation.newPublicKey)
      // Each signed rotation request carries the value in a 256-bit cell, whose data a BOC
      // stores verbatim; the contract logs the same value for Toncenter to report.
      expect(Buffer.from(firstRotation.signedBoc, "base64").includes(encryptedOldKeys.first)).toBe(
        true,
      )
      expect(
        Buffer.from(secondRotation.signedBoc, "base64").includes(encryptedOldKeys.second),
      ).toBe(true)
    })

    test("decrypts comments to the current and anchor keys without HTTP", async () => {
      const {client, urls, secretReads} = await currentClient(wallet.actions)

      await expect(
        client.decryptComment({sender: wallet.sender, body: wallet.comments.toCurrent}),
      ).resolves.toBe("current")
      await expect(
        client.decryptComment({sender: wallet.sender, body: wallet.comments.toAnchor}),
      ).resolves.toBe("anchor")

      expect(urls).toEqual([])
      expect(wallet.secrets.reasons.slice(secretReads)).toEqual([
        "decryptComment",
        "decryptComment",
      ])
    })

    test("recovers a replaced signing key from the Toncenter key-change history", async () => {
      const {client, urls, secretReads} = await currentClient(wallet.actions)
      const request = {sender: wallet.sender, body: wallet.comments.toReplaced}

      await expect(client.decryptComment(request)).resolves.toBe("replaced")

      expect(urls).toHaveLength(1)
      const url = new URL(urls[0] ?? "")
      expect(`${url.origin}${url.pathname}`).toBe("https://testnet.toncenter.com/api/v3/actions")
      expect(Object.fromEntries(url.searchParams)).toEqual({
        account: wallet.descriptor.address,
        action_type: "change_wallet_key",
        limit: "100",
        offset: "0",
        sort: "desc",
      })

      // The history holds no secret, so the client reuses it while it reaches the current key.
      await expect(client.decryptComment(request)).resolves.toBe("replaced")
      expect(urls).toHaveLength(1)
      expect(wallet.secrets.reasons.slice(secretReads)).toEqual([
        "decryptComment",
        "decryptComment",
      ])
    })

    test("separates a lagging key-change history from a comment no key decrypts", async () => {
      const {client, urls, chain} = await currentClient([])
      const request = {sender: wallet.sender, body: wallet.comments.toReplaced}

      // The indexer has not reported the rotation that installed the current key yet.
      await expect(client.decryptComment(request)).rejects.toThrow(
        "encrypted-comment recipient lookup failed",
      )
      chain.actions = wallet.actions
      await expect(client.decryptComment(request)).resolves.toBe("replaced")
      await expect(
        client.decryptComment({sender: wallet.sender, body: wallet.comments.toStranger}),
      ).rejects.toThrow("encrypted comment is unavailable")

      expect(urls).toHaveLength(2)
    })
  })
})

function accountResponse(state: string): Response {
  return Response.json({
    ok: true,
    result: {balance: "1000000000", state, sync_utime: 1_800_000_000},
  })
}

/** Answers of `toncenterFetch`; tests may change them between calls. */
interface ChainAnswers {
  /** Hex key every `get_public_key` call returns. */
  publicKeyHex: string
  /** Actions every `/api/v3/actions` page returns. */
  actions: unknown[]
}

/** Serves an active wallet with seqno 42 and records every requested URL. */
function toncenterFetch(urls: string[], chain: ChainAnswers): typeof globalThis.fetch {
  return mockFetch(async (input, init) => {
    const url = String(input)
    urls.push(url)
    if (url.includes("/api/v3/actions?")) {
      return Response.json({actions: chain.actions, address_book: {}, metadata: {}})
    }
    if (url.includes("getAddressInformation")) {
      return accountResponse("active")
    }
    const body = new TextDecoder().decode(init?.body as Uint8Array)
    const request = JSON.parse(body) as {method: string; params: {method: string}}
    if (request.method !== "runGetMethod") {
      throw new TypeError(`unexpected Toncenter call ${request.method}`)
    }
    const value = request.params.method === "seqno" ? "0x2a" : `0x${chain.publicKeyHex}`
    return Response.json({ok: true, result: {exit_code: 0, stack: [["num", value]]}})
  })
}

/** One successful rotation of `wallet` in the Toncenter v3 `/actions` format. */
function changeWalletKeyAction(
  wallet: string,
  newPublicKey: readonly number[],
  encryptedOldPrivateKey: Uint8Array,
): unknown {
  return {
    type: "change_wallet_key",
    success: true,
    details: {
      source: null,
      destination: wallet.toUpperCase(),
      new_public_key: Buffer.from(newPublicKey).toString("hex"),
      rotation_signature: null,
      encrypted_old_private_key: Buffer.from(encryptedOldPrivateKey).toString("hex"),
    },
  }
}

/**
 * Ed25519 seed of the signing half of a recovery phrase: words 13-24, or the only 12 words
 * before the first rotation. Like the engine, it derives a passphraseless BIP-39 seed and then
 * the SLIP-0010 key on m/44'/607'/0'.
 */
function signingSeed(phrase: string): Buffer {
  const half = phrase.split(" ").slice(-12).join(" ")
  let node = createHmac("sha512", "ed25519 seed")
    .update(pbkdf2Sync(half, "mnemonic", 2048, 64, "sha512"))
    .digest()
  for (const index of [44, 607, 0]) {
    const hardened = Buffer.alloc(4)
    hardened.writeUInt32BE(index + 2 ** 31)
    node = createHmac("sha512", node.subarray(32))
      .update(Buffer.concat([Buffer.alloc(1), node.subarray(0, 32), hardened]))
      .digest()
  }
  return node.subarray(0, 32)
}

function ed25519PublicKey(seed: Uint8Array): number[] {
  const pkcs8Prefix = Buffer.from("302e020100300506032b657004220420", "hex")
  const privateKey = createPrivateKey({
    key: Buffer.concat([pkcs8Prefix, seed]),
    format: "der",
    type: "pkcs8",
  })
  const spki = createPublicKey(privateKey).export({format: "der", type: "spki"})
  return Array.from(spki.subarray(-32))
}

/** `sha256(newSeed ‖ "keyChangeSaltV1") XOR oldSeed`, which a rotation publishes. */
function encryptOldPrivateKey(oldSeed: Uint8Array, newSeed: Uint8Array): Buffer {
  const mask = createHash("sha256").update(newSeed).update("keyChangeSaltV1").digest()
  return Buffer.from(mask.map((byte, index) => byte ^ (oldSeed[index] ?? 0)))
}

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
