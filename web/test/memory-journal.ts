import type {
  JournalCompareExchange,
  JournalCompareExchangeResult,
  JournalKey,
  JournalRecord,
  JournalStoreHost,
} from "../src"

export class MemoryJournal implements JournalStoreHost {
  private readonly records: Map<string, JournalRecord> = new Map()

  async load(key: JournalKey): Promise<JournalRecord | undefined> {
    return this.records.get(journalKey(key))
  }

  async compareExchange(mutation: JournalCompareExchange): Promise<JournalCompareExchangeResult> {
    const key = journalKey(mutation.key)
    const current = this.records.get(key)
    if (current?.version !== mutation.expectedVersion) {
      return {applied: false, current}
    }
    this.records.set(key, mutation.replacement)
    return {applied: true, current: mutation.replacement}
  }
}

function journalKey(key: JournalKey): string {
  return `${key.recordId}:${key.slot}`
}
