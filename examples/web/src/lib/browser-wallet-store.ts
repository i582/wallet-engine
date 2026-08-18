import type {
  ProtectedSecretRead,
  ProtectedSecretRef,
  ProtectedSecretStore,
  ProtectedSecretStoreHost,
  TonConnectStorage,
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
  readonly tonConnect: {
    readonly key: string
    readonly value: string
  }
}

/**
 * Persists the example wallet across reloads.
 * This database is not a substitute for an encrypted browser vault.
 */
export class BrowserWalletStore implements ProtectedSecretStoreHost, TonConnectStorage {
  private readonly database: Promise<IDBPDatabase<WalletExampleDatabase>>

  constructor() {
    this.database = openDB<WalletExampleDatabase>(DATABASE_NAME, 2, {
      upgrade(database: IDBPDatabase<WalletExampleDatabase>, oldVersion: number): void {
        if (oldVersion < 1) {
          database.createObjectStore("secrets")
          database.createObjectStore("state")
        }
        if (oldVersion < 2) {
          database.createObjectStore("tonConnect")
        }
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

  async load(key: string): Promise<string | undefined> {
    return await (await this.database).get("tonConnect", key)
  }

  async save(key: string, value: string): Promise<void> {
    await (await this.database).put("tonConnect", value, key)
  }

  async remove(key: string): Promise<void> {
    await (await this.database).delete("tonConnect", key)
  }
}

function secretError(kind: string, diagnostic: string): Error {
  return Object.assign(new Error(diagnostic), {kind, diagnostic})
}
