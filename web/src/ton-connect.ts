import {Base64, SessionCrypto} from "@tonconnect/protocol"

import {
  decryptBridgeRequest,
  type DecryptedBridgeRequest,
  postBridgeMessage,
  readBridgeEvents,
} from "./ton-connect-bridge"
import {
  accountReply,
  canonicalRequestId,
  DEFAULT_BRIDGE_URL,
  delay,
  deviceInfo,
  enforceConnectRequest,
  errorMessage,
  isNewerRequestId,
  loadManifest,
  normalizeBridgeUrl,
  parsePersistedSession,
  parseTonConnectLink,
  RECONNECT_DELAY_MS,
  requireValue,
  STORAGE_VERSION,
} from "./ton-connect-protocol"
import {prepareTransaction, type PreparedTransaction} from "./ton-connect-transaction"
import type {
  AppManifest,
  AppRequest,
  ParsedConnectLink,
  PendingApproval,
  PendingBridgePost,
  PersistedSession,
  SseEvent,
  TonConnectAccountInfo,
  TonConnectInteraction,
  TonConnectStorage,
  TonConnectWalletEvent,
  TonConnectWalletOptions,
} from "./ton-connect-types"
import type {SendPreview, SendResult} from "./send-types"
import type {WalletDescriptor} from "./types"
import type {WalletClient} from "./wallet-client"
import type {WalletLifecycle} from "./wallet-lifecycle"

export {isNewerRequestId, parseTonConnectLink} from "./ton-connect-protocol"
export type {
  TonConnectInteraction,
  TonConnectStorage,
  TonConnectWalletEvent,
  TonConnectWalletOptions,
} from "./ton-connect-types"

/** Wallet-side TON Connect HTTP bridge runtime for browser applications. */
export class TonConnectWallet {
  private readonly descriptor: WalletDescriptor
  private readonly walletClient: WalletClient
  private readonly lifecycle: WalletLifecycle
  private readonly bridgeUrl: string
  private readonly fetch: typeof globalThis.fetch
  private readonly storage: TonConnectStorage
  private readonly storageKey: string
  private readonly listeners: Set<(event: TonConnectWalletEvent) => void> = new Set()
  private readonly approvals: Map<string, PendingApproval> = new Map()
  private abortController: AbortController = new AbortController()
  private crypto?: SessionCrypto
  private peerClientId?: string
  private manifest?: AppManifest
  private account?: TonConnectAccountInfo
  private lastRequestId?: string
  private lastEventId?: string
  private nextWalletEventId: number = 0
  private pendingPost?: PendingBridgePost
  private listenTask?: Promise<void>
  private running: boolean = false

  constructor(options: TonConnectWalletOptions) {
    this.descriptor = options.descriptor
    this.walletClient = options.walletClient
    this.lifecycle = options.lifecycle
    this.bridgeUrl = normalizeBridgeUrl(options.bridgeUrl ?? DEFAULT_BRIDGE_URL)
    this.fetch = options.fetch ?? globalThis.fetch.bind(globalThis)
    this.storage = options.storage
    this.storageKey = `wallet-engine:ton-connect:${this.descriptor.recordId}`
  }

  /** Subscribes to approvals, lifecycle changes, and failures. */
  onEvent(listener: (event: TonConnectWalletEvent) => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  /** Opens and approves a new TON Connect link. */
  async start(linkValue: string): Promise<void> {
    if (this.running) {
      throw new Error("A TON Connect session is already active")
    }
    this.resetAbortController()
    this.running = true
    try {
      const link: ParsedConnectLink = parseTonConnectLink(linkValue)
      const account: TonConnectAccountInfo = this.lifecycle.tonConnectAccount(this.descriptor)
      enforceConnectRequest(link.request, account.network)
      const manifest: AppManifest = await loadManifest(
        this.fetch,
        link.request.manifestUrl,
        this.abortController.signal,
      )
      const domain: string = new URL(manifest.url).hostname
      const approved: boolean = await this.requestApproval({
        kind: "connect",
        id: crypto.randomUUID(),
        dappName: manifest.name,
        origin: manifest.url,
        iconUrl: manifest.iconUrl,
        account: account.address,
        proofPayload: link.request.items.find(item => item.name === "ton_proof")?.payload,
      })
      const session: SessionCrypto = new SessionCrypto()
      this.crypto = session
      this.peerClientId = link.peerClientId
      this.manifest = manifest
      this.account = account
      this.lastRequestId = undefined
      this.lastEventId = undefined
      this.nextWalletEventId = 0

      if (!approved) {
        await this.postEncrypted(
          {
            event: "connect_error",
            id: this.allocateWalletEventId(),
            payload: {code: 300, message: "User declined the connection"},
          },
          undefined,
          link.traceId,
        )
        await this.clearSession()
        return
      }

      const items: unknown[] = []
      for (const item of link.request.items) {
        if (item.name === "ton_addr") {
          items.push(accountReply(account))
        } else if (item.name === "ton_proof" && typeof item.payload === "string") {
          const timestamp: number = Math.floor(Date.now() / 1000)
          const signed = await this.lifecycle.signTonConnectProof({
            descriptor: this.descriptor,
            domain,
            timestamp,
            payload: item.payload,
          })
          items.push({
            name: "ton_proof",
            proof: {
              timestamp: String(timestamp),
              domain: {lengthBytes: new TextEncoder().encode(domain).byteLength, value: domain},
              payload: item.payload,
              signature: Base64.encode(new Uint8Array(signed.signature)),
            },
          })
        } else {
          items.push({name: item.name, error: {code: 400, message: "Method is not supported"}})
        }
      }
      await this.postEncrypted(
        {
          event: "connect",
          id: this.allocateWalletEventId(),
          payload: {
            items,
            device: deviceInfo(),
          },
        },
        undefined,
        link.traceId,
      )
      this.emit({kind: "connected", dappName: manifest.name, account: account.address})
      this.listenTask = this.listen()
    } catch (cause) {
      this.running = false
      this.emit({kind: "error", message: errorMessage(cause)})
      throw cause
    }
  }

  /** Restores the last connected dApp session and resumes its SSE stream. */
  async restore(): Promise<boolean> {
    if (this.running) {
      return true
    }
    const encoded: string | undefined = await this.storage.load(this.storageKey)
    if (!encoded) {
      return false
    }
    const persisted: PersistedSession = parsePersistedSession(encoded)
    if (persisted.bridgeUrl !== this.bridgeUrl) {
      await this.storage.remove(this.storageKey)
      return false
    }
    this.resetAbortController()
    this.crypto = new SessionCrypto(persisted.keyPair)
    this.peerClientId = persisted.peerClientId
    this.manifest = persisted.manifest
    this.account = persisted.account
    this.lastRequestId = persisted.lastRequestId
    this.lastEventId = persisted.lastEventId
    this.nextWalletEventId = persisted.nextWalletEventId
    this.pendingPost = persisted.pendingPost
    this.running = true
    this.emit({
      kind: "connected",
      dappName: persisted.manifest.name,
      account: persisted.account.address,
    })
    this.listenTask = this.listen()
    return true
  }

  /** Resolves a pending connect or transaction interaction. */
  respond(interactionId: string, approved: boolean): void {
    const pending: PendingApproval | undefined = this.approvals.get(interactionId)
    if (!pending) {
      return
    }
    this.approvals.delete(interactionId)
    pending.resolve(approved)
  }

  /** Sends a wallet-initiated disconnect and removes persisted session keys. */
  async disconnect(): Promise<void> {
    if (!this.running || !this.crypto || !this.peerClientId) {
      return
    }
    await this.postEncrypted({event: "disconnect", id: this.allocateWalletEventId(), payload: {}})
    this.abortController.abort()
    await this.listenTask?.catch(() => undefined)
    this.listenTask = undefined
    await this.clearSession()
    this.emit({kind: "disconnected"})
  }

  /** Stops transport work without revoking a resumable session. */
  async close(): Promise<void> {
    this.abortController.abort()
    for (const pending of this.approvals.values()) {
      pending.resolve(false)
    }
    this.approvals.clear()
    await this.listenTask?.catch(() => undefined)
    this.listenTask = undefined
    this.running = false
  }

  private async listen(): Promise<void> {
    const cryptoSession: SessionCrypto = requireValue(this.crypto, "session crypto")
    const peerClientId: string = requireValue(this.peerClientId, "peer client id")
    while (!this.abortController.signal.aborted && this.running) {
      try {
        // A wallet response is persisted before the bridge request starts. Retry it before
        // accepting more dApp requests so a temporary POST failure cannot strand the response.
        if (this.pendingPost !== undefined) {
          await this.flushPendingPost()
        }
        await readBridgeEvents({
          bridgeUrl: this.bridgeUrl,
          clientId: cryptoSession.sessionId,
          lastEventId: this.lastEventId,
          fetch: this.fetch,
          signal: this.abortController.signal,
          onEvent: event => this.handleSseEvent(event, cryptoSession, peerClientId),
        })
      } catch (cause) {
        if (this.abortController.signal.aborted || !this.running) {
          return
        }
        this.emit({kind: "error", message: errorMessage(cause)})
      }
      await delay(RECONNECT_DELAY_MS, this.abortController.signal)
    }
  }

  private async handleSseEvent(
    event: SseEvent,
    cryptoSession: SessionCrypto,
    peerClientId: string,
  ): Promise<void> {
    if (event.id !== undefined) {
      this.lastEventId = event.id
      await this.persistSession()
    }
    const decrypted: DecryptedBridgeRequest | undefined = decryptBridgeRequest(
      event,
      cryptoSession,
      peerClientId,
    )
    if (!decrypted) {
      return
    }
    const {request, traceId} = decrypted
    if (!isNewerRequestId(request.id, this.lastRequestId)) {
      return
    }
    this.lastRequestId = canonicalRequestId(request.id)
    await this.persistSession()
    await this.handleRequest(request, traceId)
  }

  private async handleRequest(request: AppRequest, traceId?: string): Promise<void> {
    if (request.method === "disconnect" && request.params.length === 0) {
      await this.postEncrypted({result: {}, id: request.id}, "disconnect", traceId)
      await this.clearSession()
      this.emit({kind: "disconnected"})
      return
    }
    if (request.method !== "sendTransaction") {
      await this.postRpcError({
        id: request.id,
        code: 400,
        message: "Method is not supported",
        topic: request.method,
        traceId,
      })
      return
    }
    const account: TonConnectAccountInfo = requireValue(this.account, "connected account")
    const prepared: PreparedTransaction = prepareTransaction({
      request,
      account,
      descriptor: this.descriptor,
      sessionId: requireValue(this.crypto, "session crypto").sessionId,
      dappName: requireValue(this.manifest, "dApp manifest").name,
    })
    if (!prepared.ok) {
      await this.postRpcError({
        id: request.id,
        code: prepared.code,
        message: prepared.message,
        topic: request.method,
        traceId,
      })
      return
    }
    let preview: SendPreview
    try {
      preview = await this.walletClient.previewTonConnect(prepared.sendRequest)
    } catch (cause) {
      const diagnostic: string = errorMessage(cause)
      this.emit({kind: "error", message: `Transaction preview failed: ${diagnostic}`})
      await this.postRpcError({
        id: request.id,
        code: 0,
        message: `Transaction preview failed: ${diagnostic}`,
        topic: request.method,
        traceId,
      })
      return
    }
    const approved: boolean = await this.requestApproval({...prepared.interaction, preview})
    if (!approved) {
      await this.postRpcError({
        id: request.id,
        code: 300,
        message: "User declined the transaction",
        topic: request.method,
        traceId,
      })
      return
    }
    let result: SendResult
    try {
      result = await this.walletClient.send(prepared.sendRequest)
      if (!(["submitted", "submissionUnknown", "confirmed"] as string[]).includes(result.phase)) {
        throw new Error(`Transaction finished with ${result.phase}`)
      }
    } catch (cause) {
      const diagnostic: string = errorMessage(cause)
      this.emit({kind: "error", message: `Transaction failed: ${diagnostic}`})
      await this.postRpcError({
        id: request.id,
        code: 0,
        message: "Transaction submission failed",
        topic: request.method,
        traceId,
      })
      return
    }

    // Do not include the bridge POST in the submission catch above. At this point the wallet
    // may already have submitted the message; replacing its durable success response with an
    // RPC error would lie to the dApp and could encourage a duplicate payment.
    await this.postEncrypted({result: result.signedBoc, id: request.id}, request.method, traceId)
    this.emit({kind: "transactionFinished", message: `Transaction: ${result.phase}`})
  }

  private async postRpcError(options: {
    readonly id: string
    readonly code: number
    readonly message: string
    readonly topic: string
    readonly traceId?: string
  }): Promise<void> {
    await this.postEncrypted(
      {error: {code: options.code, message: options.message}, id: options.id},
      options.topic,
      options.traceId,
    )
  }

  private async postEncrypted(payload: unknown, topic?: string, traceId?: string): Promise<void> {
    this.pendingPost = {payload, topic, traceId}
    await this.persistSession()
    await this.flushPendingPost()
  }

  private async flushPendingPost(): Promise<void> {
    const pending: PendingBridgePost = requireValue(this.pendingPost, "pending bridge response")
    const cryptoSession: SessionCrypto = requireValue(this.crypto, "session crypto")
    const peerClientId: string = requireValue(this.peerClientId, "peer client id")
    await postBridgeMessage({
      bridgeUrl: this.bridgeUrl,
      crypto: cryptoSession,
      peerClientId,
      payload: pending.payload,
      topic: pending.topic,
      traceId: pending.traceId,
      fetch: this.fetch,
      signal: this.abortController.signal,
    })
    this.pendingPost = undefined
    await this.persistSession()
  }

  private requestApproval(interaction: TonConnectInteraction): Promise<boolean> {
    return new Promise(resolve => {
      this.approvals.set(interaction.id, {resolve})
      this.emit({kind: "interaction", interaction})
    })
  }

  private allocateWalletEventId(): number {
    const id: number = this.nextWalletEventId
    if (!Number.isSafeInteger(id) || id < 0) {
      throw new Error("TON Connect wallet event id is exhausted")
    }
    this.nextWalletEventId += 1
    return id
  }

  private async persistSession(): Promise<void> {
    if (!this.crypto || !this.peerClientId || !this.manifest || !this.account) {
      return
    }
    const persisted: PersistedSession = {
      version: STORAGE_VERSION,
      keyPair: this.crypto.stringifyKeypair(),
      peerClientId: this.peerClientId,
      bridgeUrl: this.bridgeUrl,
      lastRequestId: this.lastRequestId,
      lastEventId: this.lastEventId,
      nextWalletEventId: this.nextWalletEventId,
      manifest: this.manifest,
      account: this.account,
      pendingPost: this.pendingPost,
    }
    await this.storage.save(this.storageKey, JSON.stringify(persisted))
  }

  private async clearSession(): Promise<void> {
    this.abortController.abort()
    this.running = false
    this.crypto = undefined
    this.peerClientId = undefined
    this.manifest = undefined
    this.account = undefined
    this.lastRequestId = undefined
    this.lastEventId = undefined
    this.pendingPost = undefined
    await this.storage.remove(this.storageKey)
  }

  private resetAbortController(): void {
    this.abortController.abort()
    this.abortController = new AbortController()
  }

  private emit(event: TonConnectWalletEvent): void {
    for (const listener of this.listeners) {
      listener(event)
    }
  }
}
