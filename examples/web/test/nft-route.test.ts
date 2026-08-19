import {describe, expect, test} from "bun:test"

import {nftDetailHash, parseNftRoute} from "@/lib/nft-route"

describe("NFT hash routes", () => {
  test("routes the wallet and full collection pages", () => {
    expect(parseNftRoute("")).toEqual({kind: "wallet"})
    expect(parseNftRoute("#/collectibles")).toEqual({kind: "collection"})
    expect(parseNftRoute("#/unknown")).toEqual({kind: "wallet"})
  })

  test("round-trips raw NFT addresses through a detail route", () => {
    const address = `0:${"2B".repeat(32)}`

    expect(parseNftRoute(nftDetailHash(address))).toEqual({kind: "detail", address})
  })

  test("falls back to the collection for malformed detail paths", () => {
    expect(parseNftRoute("#/collectibles/")).toEqual({kind: "collection"})
    expect(parseNftRoute("#/collectibles/%E0%A4%A")).toEqual({kind: "collection"})
  })
})
