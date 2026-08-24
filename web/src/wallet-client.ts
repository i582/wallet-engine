import {WalletClient as RawWalletClient} from "../../bindings/wasm/wallet_engine.js"

import {BrowserHttpHost, type BrowserHttpHostOptions} from "./http-host"
import {initializeWalletEngine} from "./initialize"
import type {BrowserPlatformHost} from "./platform-host"
import type {
  NftTransferPreviewRequest,
  NftTransferRequest,
  SendPreview,
  SendPreviewRequest,
  SendRequest,
  SendResult,
  SignMessageRequest,
  SignMessagePreview,
  SignMessageResult,
} from "./send-types"
import type {
  CreateEncryptedCommentRequest,
  DecryptCommentRequest,
  WalletClientConfig,
  WalletSnapshot,
  WalletUpdate,
} from "./types"

export interface CreateClientOptions extends BrowserHttpHostOptions {
  readonly platformHost: BrowserPlatformHost
}

export class WalletClient {
  private readonly raw: RawWalletClient
  private closed: boolean = false

  private constructor(raw: RawWalletClient) {
    this.raw = raw
  }

  static async create(
    config: WalletClientConfig,
    options: CreateClientOptions,
  ): Promise<WalletClient> {
    await initializeWalletEngine()
    const httpHost = new BrowserHttpHost(config.providers.toncenterBaseUrl, options)
    return new WalletClient(new RawWalletClient(config, httpHost, options.platformHost))
  }

  snapshot(): WalletSnapshot {
    this.assertOpen()
    return this.raw.snapshot() as WalletSnapshot
  }

  async waitForChange(afterRevision: number): Promise<WalletSnapshot> {
    this.assertOpen()
    return (await this.raw.waitForChange(BigInt(afterRevision))) as WalletSnapshot
  }

  async refresh(): Promise<WalletUpdate> {
    this.assertOpen()
    return (await this.raw.refresh()) as WalletUpdate
  }

  async resolvePending(): Promise<WalletSnapshot["send"]> {
    this.assertOpen()
    return (await this.raw.resolvePending()) as WalletSnapshot["send"]
  }

  /** Resolves the standard TON DNS wallet record for a `.ton` name. */
  async resolveDns(name: string): Promise<string | null> {
    this.assertOpen()
    return (await this.raw.resolveDns(name)) as string | null
  }

  async cancelRefresh(): Promise<void> {
    this.assertOpen()
    await this.raw.cancelRefresh()
  }

  async loadMoreActivity(): Promise<WalletUpdate> {
    this.assertOpen()
    return (await this.raw.loadMoreActivity()) as WalletUpdate
  }

  async cancelLoadMoreActivity(): Promise<void> {
    this.assertOpen()
    await this.raw.cancelLoadMoreActivity()
  }

  async refreshNfts(): Promise<WalletUpdate> {
    this.assertOpen()
    return (await this.raw.refreshNfts()) as WalletUpdate
  }

  async cancelRefreshNfts(): Promise<void> {
    this.assertOpen()
    await this.raw.cancelRefreshNfts()
  }

  async loadMoreNfts(): Promise<WalletUpdate> {
    this.assertOpen()
    return (await this.raw.loadMoreNfts()) as WalletUpdate
  }

  async cancelLoadMoreNfts(): Promise<void> {
    this.assertOpen()
    await this.raw.cancelLoadMoreNfts()
  }

  async send(request: SendRequest): Promise<SendResult> {
    this.assertOpen()
    return (await this.raw.send(request)) as SendResult
  }

  /** Returns a Base64 BOC suitable for SendMessageBody.rawPayload. */
  async createEncryptedComment(request: CreateEncryptedCommentRequest): Promise<string> {
    this.assertOpen()
    return (await this.raw.createEncryptedComment(request)) as string
  }

  /** Explicitly authorizes and decrypts one TON encrypted-comment BOC. */
  async decryptComment(request: DecryptCommentRequest): Promise<string> {
    this.assertOpen()
    return (await this.raw.decryptComment(request)) as string
  }

  /** Revalidates ownership and signs/submits one TEP-62 NFT transfer. */
  async sendNftTransfer(request: NftTransferRequest): Promise<SendResult> {
    this.assertOpen()
    return (await this.raw.sendNftTransfer(request)) as SendResult
  }

  /** Signs a complete internal Wallet V5 message and leaves submission to the caller. */
  async signMessage(request: SignMessageRequest): Promise<SignMessageResult> {
    this.assertOpen()
    return (await this.raw.signMessage(request)) as SignMessageResult
  }

  async previewSend(request: SendPreviewRequest): Promise<SendPreview> {
    this.assertOpen()
    return (await this.raw.previewSend(request)) as SendPreview
  }

  /** Loads fresh NFT state and requires a complete successful emulated transfer action. */
  async previewNftTransfer(request: NftTransferPreviewRequest): Promise<SendPreview> {
    this.assertOpen()
    return (await this.raw.previewNftTransfer(request)) as SendPreview
  }

  /** Previews the exact expiration, payload, and StateInit in a TON Connect request. */
  async previewTonConnect(request: SendRequest): Promise<SendPreview> {
    this.assertOpen()
    return (await this.raw.previewTonConnect(request)) as SendPreview
  }

  /** Validates a sign-only request without reporting a wallet-paid network fee. */
  async previewSignMessage(request: SendPreviewRequest): Promise<SignMessagePreview> {
    this.assertOpen()
    return (await this.raw.previewSignMessage(request)) as SignMessagePreview
  }

  async cancelSendPreview(): Promise<void> {
    this.assertOpen()
    await this.raw.cancelSendPreview()
  }

  async cancelSend(): Promise<void> {
    this.assertOpen()
    await this.raw.cancelSend()
  }

  async close(): Promise<void> {
    if (this.closed) {
      return
    }
    this.closed = true
    await this.raw.shutdown()
    this.raw.free()
  }

  private assertOpen(): void {
    if (this.closed) {
      throw new Error("The wallet client is closed")
    }
  }
}
