import {describe, expect, test} from "bun:test"
import type {NftItem, NftList, ResourceState} from "@ton/wallet-engine"
import {renderToStaticMarkup} from "react-dom/server"

import {CollectiblesPage} from "@/components/collectibles-page"
import {NftDetailPage} from "@/components/nft-detail-page"
import {NftSendForm} from "@/components/nft-send-form"

const READY_RESOURCE: ResourceState = {phase: "ready"}

describe("collectible pages", () => {
  test("renders all owned NFTs as links in the collection grid", () => {
    const item: NftItem = nft()
    const markup: string = renderToStaticMarkup(
      <CollectiblesPage
        loadingMore={false}
        nfts={nftList(item)}
        refreshing={false}
        onLoadMore={noAction}
        onRefresh={noAction}
      />,
    )

    expect(markup).toContain("1 loaded item")
    expect(markup).toContain("Shadow Reaper")
    expect(markup).toContain("#/collectibles/0%3A")
  })

  test("renders inline artwork and metadata on the NFT detail page", () => {
    const item: NftItem = nft()
    const markup: string = renderToStaticMarkup(
      <NftDetailPage
        canForceRetry={false}
        hasMore={false}
        item={item}
        loadingMore={false}
        onCancelPreview={noAction}
        onLoadMore={noAction}
        onPreviewTransfer={unexpectedAction}
        onSendTransfer={unexpectedAction}
      />,
    )

    expect(markup).toContain("data:image/svg+xml,%3Csvg%2F%3E")
    expect(markup).toContain("Few have witnessed such magnificence.")
    expect(markup).toContain("Last transaction LT")
    expect(markup).toContain("Send collectible")
  })

  test("starts NFT sending with a new-owner form and irreversible-action warning", () => {
    const markup: string = renderToStaticMarkup(
      <NftSendForm
        canForceRetry={false}
        item={nft()}
        onCancelPreview={noAction}
        onClose={() => undefined}
        onPreview={unexpectedAction}
        onSend={unexpectedAction}
      />,
    )

    expect(markup).toContain("Send collectible")
    expect(markup).toContain("New owner")
    expect(markup).toContain("cannot be undone")
    expect(markup).not.toContain("Send anyway")
  })
})

function noAction(): Promise<void> {
  return Promise.resolve()
}

function unexpectedAction(): Promise<never> {
  return Promise.reject(new Error("render-only callback must not run"))
}

function nftList(item: NftItem): NftList {
  return {
    hasMore: false,
    items: [item],
    paginationResource: READY_RESOURCE,
    resource: READY_RESOURCE,
  }
}

function nft(): NftItem {
  return {
    address: `0:${"2B".repeat(32)}`,
    codeHash: "code",
    content: {
      description: "Few have witnessed such magnificence.",
      image: "data:image/svg+xml,%3Csvg%2F%3E",
      name: "Shadow Reaper",
    },
    dataHash: "data",
    index: "0",
    initialized: true,
    lastTransactionLt: "90751083000003",
    onSale: false,
  }
}
