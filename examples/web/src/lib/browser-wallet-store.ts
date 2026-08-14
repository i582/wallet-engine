import type {
  ProtectedSecretRead,
  ProtectedSecretRef,
  ProtectedSecretStore,
  ProtectedSecretStoreHost,
  WalletDescriptor,
} from "@ton/wallet-engine"
import {type DBSchema, type IDBPDatabase, openDB} from "idb"

const DATABASE_NAME: string = "wallet-engine-example"
const WALLET_KEY: string = "current-wallet"

interface WalletExampleDatabase extends DBSchema {
  readonly secrets: {
    readonly key: string
    readonly value: Uint8Array
  }
  readonly state: {
    readonly key: string
    readonly value: WalletDescriptor
  }
}

/**
 * Persists the example wallet across reloads.
 * This database is not a substitute for an encrypted browser vault.
 */
export class BrowserWalletStore implements ProtectedSecretStoreHost {
  private readonly database: Promise<IDBPDatabase<WalletExampleDatabase>>

  constructor() {
    this.database = openDB<WalletExampleDatabase>(DATABASE_NAME, 1, {
      upgrade(database: IDBPDatabase<WalletExampleDatabase>): void {
        database.createObjectStore("secrets")
        database.createObjectStore("state")
      },
    })
  }

  async read(request: ProtectedSecretRead): Promise<Uint8Array> {
    const value: Uint8Array | undefined = await (await this.database).get(
      "secrets",
      request.secretRef.value,
    )
    if (!value) {
      throw secretError("notFound", "The recovery phrase is not available in this browser")
    }
    return value.slice()
  }

  async store(request: ProtectedSecretStore): Promise<void> {
    await (await this.database).put(
      "secrets",
      new Uint8Array(request.bytes),
      request.secretRef.value,
    )
  }

  async delete(secretRef: ProtectedSecretRef): Promise<void> {
    await (await this.database).delete("secrets", secretRef.value)
  }

  async loadWallet(): Promise<WalletDescriptor | undefined> {
    return await (await this.database).get("state", WALLET_KEY)
  }

  async saveWallet(descriptor: WalletDescriptor): Promise<void> {
    await (await this.database).put("state", descriptor, WALLET_KEY)
  }

  async clearWallet(): Promise<void> {
    await (await this.database).delete("state", WALLET_KEY)
  }
}

function secretError(kind: string, diagnostic: string): Error {
  return Object.assign(new Error(diagnostic), {kind, diagnostic})
}
