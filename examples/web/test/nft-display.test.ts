import {describe, expect, test} from "bun:test"
import type {NftItem} from "@ton/wallet-engine"

import {
  nftCollectionName,
  nftDisplayName,
  nftImageUrl,
  normalizeNftMediaUrl,
} from "@/lib/nft-display"

describe("NFT display metadata", () => {
  test("uses indexed names and deterministic fallbacks", () => {
    expect(nftDisplayName(nft({name: "  Neon Pass  "}))).toBe("Neon Pass")
    expect(nftDisplayName(nft({domain: "alice.ton"}))).toBe("alice.ton")
    expect(nftDisplayName(nft({}))).toBe("NFT #42")
  })

  test("prefers the resolved collection descriptor and medium artwork fields", () => {
    const content = Object.fromEntries([
      ["collection", "On-chain collection"],
      ["collection_name", "Indexed collection"],
      ["image", "https://example.com/original.png"],
      ["_image_medium", "https://example.com/medium.png"],
    ])
    expect(nftCollectionName(nft(content))).toBe("Indexed collection")
    const item: NftItem = {
      ...nft(content),
      collection: {
        address: "0:collection",
        name: "Resolved collection",
        content: {},
      },
    }
    expect(nftCollectionName(item)).toBe("Resolved collection")
    expect(nftImageUrl(item)).toBe("https://example.com/medium.png")
  })

  test("converts IPFS media and rejects executable or malformed URLs", () => {
    expect(normalizeNftMediaUrl("ipfs://ipfs/bafy/image.png")).toBe(
      "https://ipfs.io/ipfs/bafy/image.png",
    )
    expect(normalizeNftMediaUrl("javascript:alert(1)")).toBeUndefined()
    expect(normalizeNftMediaUrl("not a URL")).toBeUndefined()
  })

  test("accepts inline NFT artwork only for whitelisted image media", () => {
    const inlineSvg = [
      "data:image/svg+xml,",
      "%3Csvg%20xmlns%3D",
      "%22http%3A%2F%2Fwww.w3.org",
      "%2F2000%2Fsvg%22%2F%3E",
    ].join("")

    expect(normalizeNftMediaUrl(inlineSvg)).toBe(inlineSvg)
    expect(normalizeNftMediaUrl("data:image/png;base64,AAAA")).toBe("data:image/png;base64,AAAA")
    expect(normalizeNftMediaUrl("data:text/html,<script>alert(1)</script>")).toBeUndefined()
    expect(normalizeNftMediaUrl("data:image/svg+xml;unknown,<svg/>")).toBeUndefined()
  })
})

function nft(content: Readonly<Record<string, string>>): NftItem {
  return {
    address: "0:item",
    index: "42",
    lastTransactionLt: "100",
    initialized: true,
    onSale: false,
    codeHash: "code",
    dataHash: "data",
    content,
  }
}
