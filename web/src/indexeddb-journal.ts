import {type DBSchema, type IDBPDatabase, openDB} from "idb"

import type {
  JournalCompareExchange,
  JournalCompareExchangeResult,
  JournalKey,
  JournalRecord,
} from "./types"
import type {JournalStoreHost} from "./platform-host"

const DATABASE_VERSION = 1
const STORE_NAME = "journal"

interface JournalDatabase extends DBSchema {
  readonly journal: {
    readonly key: string
    readonly value: JournalRecord
  }
}

export class IndexedDbJournalStore implements JournalStoreHost {
  private readonly databaseName: string
  private database?: Promise<IDBPDatabase<JournalDatabase>>

  constructor(databaseName = "wallet-engine") {
    this.databaseName = databaseName
  }

  async load(key: JournalKey): Promise<JournalRecord | undefined> {
    const database = await this.open()
    const value = await database.get(STORE_NAME, storageKey(key))
    return value ? cloneRecord(value) : undefined
  }

  async compareExchange(mutation: JournalCompareExchange): Promise<JournalCompareExchangeResult> {
    const database = await this.open()
    const transaction = database.transaction(STORE_NAME, "readwrite")
    const key = storageKey(mutation.key)
    const current = await transaction.store.get(key)
    if (current?.version !== mutation.expectedVersion) {
      await transaction.done
      return {applied: false, current: current ? cloneRecord(current) : undefined}
    }
    await transaction.store.put(cloneRecord(mutation.replacement), key)
    await transaction.done
    return {applied: true, current: cloneRecord(mutation.replacement)}
  }

  private open(): Promise<IDBPDatabase<JournalDatabase>> {
    this.database ??= openDB<JournalDatabase>(this.databaseName, DATABASE_VERSION, {
      upgrade(database) {
        if (!database.objectStoreNames.contains(STORE_NAME)) {
          database.createObjectStore(STORE_NAME)
        }
      },
    })
    return this.database
  }
}

function storageKey(key: JournalKey): string {
  return `${key.walletId}\u0000${key.slot}`
}

function cloneRecord(record: JournalRecord): JournalRecord {
  return {version: record.version, payload: [...record.payload]}
}
