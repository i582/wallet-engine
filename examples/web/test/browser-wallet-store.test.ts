import {describe, expect, test} from "bun:test"
import "fake-indexeddb/auto"

import {
  IndexedDbJournalStore,
  type JournalCompareExchange,
  type WalletDescriptor,
} from "@ton/wallet-engine"

import {BrowserWalletStore} from "@/lib/browser-wallet-store"

describe("BrowserWalletStore", () => {
  test("restores the descriptor and protected secret from a new store instance", async () => {
    const descriptor: WalletDescriptor = {
      recordId: "example-record",
      address: "0:example",
      publicKey: Array.from({length: 32}, () => 0),
      network: "testnet",
      secretRef: {value: "example-secret"},
    }
    const first: BrowserWalletStore = new BrowserWalletStore()
    await first.store({
      secretRef: descriptor.secretRef,
      bytes: [1, 2, 3, 4],
      requireUserPresence: false,
    })
    await first.saveWallet(descriptor)

    const restored: BrowserWalletStore = new BrowserWalletStore()
    expect(await restored.loadWallet()).toEqual(descriptor)
    expect(
      await restored.read({
        secretRef: descriptor.secretRef,
        reason: "revealRecoveryPhrase",
        prompt: "Test prompt",
      }),
    ).toEqual(new Uint8Array([1, 2, 3, 4]))

    await restored.delete(descriptor.secretRef)
    await restored.clearWallet()
    expect(await restored.loadWallet()).toBeUndefined()
  })

  test("keeps the send journal independent from wallet storage", async () => {
    const journal: IndexedDbJournalStore = new IndexedDbJournalStore(
      "wallet-engine-example-journal-test",
    )
    const mutation: JournalCompareExchange = {
      key: {
        recordId: "example-record",
        slot: "outgoing-transfer",
      },
      expectedVersion: undefined,
      replacement: {
        version: 1,
        payload: [5, 6],
      },
    }

    expect((await journal.compareExchange(mutation)).applied).toBe(true)
    expect(await journal.load(mutation.key)).toEqual(mutation.replacement)
  })
})
