import type {
  JournalCompareExchange,
  JournalCompareExchangeResult,
  JournalKey,
  JournalRecord,
  ProtectedSecretRead,
  ProtectedSecretRef,
  ProtectedSecretStore,
} from "./types"

export interface ProtectedSecretStoreHost {
  readonly read: (request: ProtectedSecretRead) => Promise<Uint8Array>
  readonly store: (request: ProtectedSecretStore) => Promise<void>
  readonly delete: (secretRef: ProtectedSecretRef) => Promise<void>
}

export interface JournalStoreHost {
  readonly load: (key: JournalKey) => Promise<JournalRecord | undefined>
  readonly compareExchange: (
    mutation: JournalCompareExchange,
  ) => Promise<JournalCompareExchangeResult>
}

export interface BrowserPlatformHostOptions {
  readonly secrets: ProtectedSecretStoreHost
  readonly journal: JournalStoreHost
}

export class BrowserPlatformHost {
  private readonly secrets: ProtectedSecretStoreHost
  private readonly journal: JournalStoreHost

  constructor(options: BrowserPlatformHostOptions) {
    this.secrets = options.secrets
    this.journal = options.journal
  }

  async readProtectedSecret(request: ProtectedSecretRead): Promise<number[]> {
    return [...(await this.secrets.read(request))]
  }

  storeProtectedSecret(request: ProtectedSecretStore): Promise<void> {
    return this.secrets.store(request)
  }

  deleteProtectedSecret(secretRef: ProtectedSecretRef): Promise<void> {
    return this.secrets.delete(secretRef)
  }

  loadJournal(key: JournalKey): Promise<JournalRecord | undefined> {
    return this.journal.load(key)
  }

  compareExchangeJournal(mutation: JournalCompareExchange): Promise<JournalCompareExchangeResult> {
    return this.journal.compareExchange(mutation)
  }
}
