import {WalletClient as RawWalletClient} from "../../bindings/wasm/wallet_engine.js"

import {BrowserHttpHost, type BrowserHttpHostOptions} from "./http-host"
import {initializeWalletEngine} from "./initialize"
import type {BrowserPlatformHost} from "./platform-host"
import type {
  SendRequest,
  SendResult,
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
    const httpHost = new BrowserHttpHost(options)
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

  async send(request: SendRequest): Promise<SendResult> {
    this.assertOpen()
    return (await this.raw.send(request)) as SendResult
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
