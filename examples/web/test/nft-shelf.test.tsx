import {describe, expect, test} from "bun:test"
import type {NftItem, NftList, ResourceState} from "@ton/wallet-engine"
import {renderToStaticMarkup} from "react-dom/server"

import {NftShelf} from "@/components/nft-shelf"

const IDLE_RESOURCE: ResourceState = {phase: "idle"}
const READY_RESOURCE: ResourceState = {phase: "ready"}

function noAction(): Promise<void> {
  return Promise.resolve()
}

describe("NFT shelf", () => {
  test("renders the empty inventory state", () => {
    const markup: string = render(nftList([]))

    expect(markup).toContain("Collectibles")
    expect(markup).toContain("No collectibles yet")
  })

  test("renders indexed artwork, collection, and sale state", () => {
    const markup: string = render(
      nftList([
        nft(
          Object.fromEntries([
            ["collection_name", "Acton Originals"],
            ["image_url", "https://example.com/aurora.png"],
            ["name", "Aurora Relay"],
          ]),
        ),
      ]),
    )

    expect(markup).toContain("Aurora Relay")
    expect(markup).toContain("Acton Originals")
    expect(markup).toContain("https://example.com/aurora.png")
    expect(markup).toContain("On sale")
  })

  test("does not load remote artwork for reported items", () => {
    const item: NftItem = {
      ...nft(
        Object.fromEntries([
          ["image_url", "https://example.com/untrusted.png"],
          ["name", "Reported"],
        ]),
      ),
      isScam: true,
    }
    const markup: string = render(nftList([item]))

    expect(markup).toContain("Reported item")
    expect(markup).not.toContain("https://example.com/untrusted.png")
  })
})

function render(nfts: NftList): string {
  return renderToStaticMarkup(
    <NftShelf
      loadingMore={false}
      nfts={nfts}
      refreshing={false}
      onLoadMore={noAction}
      onRetry={noAction}
    />,
  )
}

function nftList(items: NftItem[]): NftList {
  return {
    hasMore: false,
    items,
    paginationResource: IDLE_RESOURCE,
    resource: READY_RESOURCE,
  }
}

function nft(content: Readonly<Record<string, string>>): NftItem {
  return {
    address: "0:item",
    codeHash: "code",
    content,
    dataHash: "data",
    index: "1",
    initialized: true,
    lastTransactionLt: "100",
    onSale: true,
  }
}
