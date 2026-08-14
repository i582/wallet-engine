import {WalletLifecycle as RawWalletLifecycle} from "../../bindings/wasm/wallet_engine.js"

import {initializeWalletEngine} from "./initialize"
import type {BrowserPlatformHost} from "./platform-host"
import type {
  CreateWalletRequest,
  CreatedWallet,
  ImportWalletRequest,
  RecoveryPhrase,
  WalletDescriptor,
} from "./types"

export class WalletLifecycle {
  private readonly raw: RawWalletLifecycle
  private closed: boolean = false

  private constructor(raw: RawWalletLifecycle) {
    this.raw = raw
  }

  static async create(platformHost: BrowserPlatformHost): Promise<WalletLifecycle> {
    await initializeWalletEngine()
    return new WalletLifecycle(new RawWalletLifecycle(platformHost))
  }

  async createWallet(request: CreateWalletRequest): Promise<CreatedWallet> {
    this.assertOpen()
    return (await this.raw.createWallet(request)) as CreatedWallet
  }

  async importWallet(request: ImportWalletRequest): Promise<WalletDescriptor> {
    this.assertOpen()
    return (await this.raw.importWallet(request)) as WalletDescriptor
  }

  async revealRecoveryPhrase(descriptor: WalletDescriptor): Promise<RecoveryPhrase> {
    this.assertOpen()
    return (await this.raw.revealRecoveryPhrase(descriptor)) as RecoveryPhrase
  }

  async deleteWallet(descriptor: WalletDescriptor): Promise<void> {
    this.assertOpen()
    await this.raw.deleteWallet(descriptor)
  }

  close(): void {
    if (this.closed) {
      return
    }
    this.closed = true
    this.raw.free()
  }

  private assertOpen(): void {
    if (this.closed) {
      throw new Error("The wallet lifecycle is closed")
    }
  }
}
