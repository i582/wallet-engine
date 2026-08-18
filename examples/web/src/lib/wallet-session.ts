import {
  BrowserPlatformHost,
  IndexedDbJournalStore,
  type RecoveryPhrase,
  type SendPreview,
  type SendResult,
  TonConnectWallet,
  type TonConnectWalletEvent,
  type WalletDescriptor,
  WalletClient,
  WalletLifecycle,
  type WalletSnapshot,
} from "@ton/wallet-engine"

import {BrowserWalletStore} from "@/lib/browser-wallet-store"

const TESTNET_BASE_URL: string = "https://testnet.toncenter.com"
const JOURNAL_DATABASE_NAME: string = "wallet-engine-example-journal"

export interface CreatedWalletSession {
  readonly session: WalletSession
  readonly recoveryPhrase: RecoveryPhrase
}

export class WalletSession {
  readonly descriptor: WalletDescriptor

  private readonly client: WalletClient
  private readonly lifecycle: WalletLifecycle
  private readonly store: BrowserWalletStore
  private readonly tonConnect: TonConnectWallet
  private closed: boolean = false

  private constructor(
    descriptor: WalletDescriptor,
    client: WalletClient,
    lifecycle: WalletLifecycle,
    store: BrowserWalletStore,
  ) {
    this.descriptor = descriptor
    this.client = client
    this.lifecycle = lifecycle
    this.store = store
    this.tonConnect = new TonConnectWallet({
      descriptor,
      walletClient: client,
      lifecycle,
      identity: {
        appName: "tonkeeper",
        appVersion: "0.1.0",
      },
      storage: store,
    })
  }

  static async create(): Promise<CreatedWalletSession> {
    const apiKey: string | undefined = optionalApiKey()
    const store: BrowserWalletStore = new BrowserWalletStore()
    const platformHost: BrowserPlatformHost = new BrowserPlatformHost({
      secrets: store,
      journal: new IndexedDbJournalStore(JOURNAL_DATABASE_NAME),
    })
    const lifecycle: WalletLifecycle = await WalletLifecycle.create(platformHost)
    const created = await lifecycle.createWallet({
      recordId: crypto.randomUUID(),
      network: "testnet",
    })

    try {
      const client: WalletClient = await createClient(created.descriptor, platformHost, apiKey)
      await store.saveWallet(created.descriptor)
      return {
        session: new WalletSession(created.descriptor, client, lifecycle, store),
        recoveryPhrase: created.recoveryPhrase,
      }
    } catch (error) {
      await lifecycle.deleteWallet(created.descriptor)
      lifecycle.close()
      throw error
    }
  }

  static async restore(): Promise<WalletSession | undefined> {
    const apiKey: string | undefined = optionalApiKey()
    const store: BrowserWalletStore = new BrowserWalletStore()
    const descriptor: WalletDescriptor | undefined = await store.loadWallet()
    if (!descriptor) {
      return undefined
    }
    const platformHost: BrowserPlatformHost = new BrowserPlatformHost({
      secrets: store,
      journal: new IndexedDbJournalStore(JOURNAL_DATABASE_NAME),
    })
    const lifecycle: WalletLifecycle = await WalletLifecycle.create(platformHost)
    try {
      const client: WalletClient = await createClient(descriptor, platformHost, apiKey)
      return new WalletSession(descriptor, client, lifecycle, store)
    } catch (error) {
      lifecycle.close()
      throw error
    }
  }

  snapshot(): WalletSnapshot {
    this.assertOpen()
    return this.client.snapshot()
  }

  async refresh(): Promise<WalletSnapshot> {
    this.assertOpen()
    return (await this.client.refresh()).snapshot
  }

  async loadMoreActivity(): Promise<WalletSnapshot> {
    this.assertOpen()
    return (await this.client.loadMoreActivity()).snapshot
  }

  async previewSend(destination: string, amountNanograms: string): Promise<SendPreview> {
    this.assertOpen()
    return await this.client.previewSend({
      intent: {
        expiration: {kind: "engineDefault"},
        message: {
          destination,
          amount: {kind: "exact", nanograms: amountNanograms},
          body: {kind: "empty"},
        },
      },
    })
  }

  async cancelSendPreview(): Promise<void> {
    this.assertOpen()
    await this.client.cancelSendPreview()
  }

  async send(destination: string, amountNanograms: string): Promise<SendResult> {
    this.assertOpen()
    try {
      return await this.client.send({
        operationId: crypto.randomUUID(),
        intent: {
          expiration: {kind: "engineDefault"},
          message: {
            destination,
            amount: {kind: "exact", nanograms: amountNanograms},
            body: {kind: "empty"},
          },
        },
      })
    } catch (cause) {
      const diagnostic: string | undefined = this.client.snapshot().send.errorMessage
      if (diagnostic) {
        throw new Error(diagnostic, {cause})
      }
      throw cause
    }
  }

  onTonConnectEvent(listener: (event: TonConnectWalletEvent) => void): () => void {
    this.assertOpen()
    return this.tonConnect.onEvent(listener)
  }

  async startTonConnect(link: string): Promise<void> {
    this.assertOpen()
    await this.tonConnect.start(link)
  }

  async restoreTonConnect(): Promise<boolean> {
    this.assertOpen()
    return await this.tonConnect.restore()
  }

  respondTonConnect(interactionId: string, approved: boolean): void {
    this.assertOpen()
    this.tonConnect.respond(interactionId, approved)
  }

  async disconnectTonConnect(): Promise<void> {
    this.assertOpen()
    await this.tonConnect.disconnect()
  }

  async forget(): Promise<void> {
    if (this.closed) {
      return
    }
    await this.tonConnect.disconnect()
    await this.client.close()
    await this.lifecycle.deleteWallet(this.descriptor)
    await this.store.clearWallet()
    this.lifecycle.close()
    this.closed = true
  }

  async close(): Promise<void> {
    if (this.closed) {
      return
    }
    await this.tonConnect.close()
    await this.client.close()
    this.lifecycle.close()
    this.closed = true
  }

  private assertOpen(): void {
    if (this.closed) {
      throw new Error("The wallet session is closed")
    }
  }
}

async function createClient(
  descriptor: WalletDescriptor,
  platformHost: BrowserPlatformHost,
  apiKey: string | undefined,
): Promise<WalletClient> {
  return await WalletClient.create(
    {
      recordId: descriptor.recordId,
      address: descriptor.address,
      publicKey: descriptor.publicKey,
      localSecretRef: descriptor.secretRef,
      network: descriptor.network,
      // This is application policy, not a hidden engine default. The engine
      // adds it to Toncenter's fresh synchronization timestamp.
      sendValiditySeconds: 300,
      resolutionMarginSeconds: 60,
      providers: {
        toncenterBaseUrl: TESTNET_BASE_URL,
        requestTimeoutMs: 15_000,
      },
    },
    {
      platformHost,
      toncenterApiKey: apiKey,
    },
  )
}

function optionalApiKey(): string | undefined {
  const value: string = import.meta.env.VITE_TONCENTER_API_KEY?.trim() ?? ""
  return value || undefined
}
