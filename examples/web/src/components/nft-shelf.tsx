import {CaretRight, ImageSquare, WarningCircle} from "@phosphor-icons/react"
import type {NftList} from "@ton/wallet-engine"
import {type ReactElement, useId} from "react"

import {NftCard} from "@/components/nft-card"
import {Button} from "@/components/ui/button"
import {Skeleton} from "@/components/ui/skeleton"

export interface NftShelfProps {
  readonly nfts: NftList
  readonly refreshing: boolean
  readonly loadingMore: boolean
  readonly onRetry: () => Promise<void>
  readonly onLoadMore: () => Promise<void>
}

/** Displays the wallet's collectible inventory without blocking activity loading. */
export function NftShelf({
  nfts,
  refreshing,
  loadingMore,
  onRetry,
  onLoadMore,
}: NftShelfProps): ReactElement {
  const titleId: string = useId()
  const initialLoading: boolean =
    nfts.items.length === 0 && (refreshing || nfts.resource.phase === "loading")
  const initialFailure: boolean = nfts.resource.phase === "failed" && nfts.items.length === 0

  return (
    <section aria-labelledby={titleId}>
      <div className="flex items-baseline justify-between gap-4">
        <div className="flex items-baseline gap-2.5">
          <h2 className="text-xl font-semibold tracking-[-0.03em]" id={titleId}>
            Collectibles
          </h2>
          {nfts.items.length > 0 ? (
            <span className="text-xs tabular-nums text-muted-foreground">
              {nfts.items.length} {nfts.items.length === 1 ? "item" : "items"}
            </span>
          ) : null}
        </div>
        <Button asChild className="-mr-2" size="sm" variant="ghost">
          <a href="#/collectibles">
            View all <CaretRight aria-hidden="true" size={14} />
          </a>
        </Button>
      </div>

      <NftShelfContent
        initialFailure={initialFailure}
        initialLoading={initialLoading}
        nfts={nfts}
        onRetry={onRetry}
      />
      <NftPagination loadingMore={loadingMore} nfts={nfts} onLoadMore={onLoadMore} />
    </section>
  )
}

function NftShelfContent({
  initialFailure,
  initialLoading,
  nfts,
  onRetry,
}: {
  readonly initialFailure: boolean
  readonly initialLoading: boolean
  readonly nfts: NftList
  readonly onRetry: () => Promise<void>
}): ReactElement {
  if (initialLoading) {
    return <NftSkeletons />
  }
  if (initialFailure) {
    return <NftLoadError onRetry={onRetry} />
  }
  if (nfts.items.length === 0) {
    return <EmptyNfts />
  }
  return (
    <>
      {nfts.resource.phase === "failed" ? (
        <p className="mt-3 flex items-center gap-1.5 text-xs text-muted-foreground">
          <WarningCircle aria-hidden="true" size={14} /> Showing the last loaded collection.
        </p>
      ) : null}
      <div className="-mx-5 mt-4 flex snap-x snap-mandatory gap-3 overflow-x-auto px-5 pb-2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {nfts.items.map(item => (
          <NftCard item={item} key={item.address} layout="shelf" />
        ))}
      </div>
    </>
  )
}

function NftPagination({
  loadingMore,
  nfts,
  onLoadMore,
}: {
  readonly loadingMore: boolean
  readonly nfts: NftList
  readonly onLoadMore: () => Promise<void>
}): ReactElement | null {
  if (!(nfts.hasMore || nfts.paginationResource.phase === "failed")) {
    return null
  }
  let label = "Load more collectibles"
  if (loadingMore) {
    label = "Loading collectibles"
  } else if (nfts.paginationResource.phase === "failed") {
    label = "Retry loading collectibles"
  }
  return (
    <Button
      className="mt-3 w-full"
      disabled={loadingMore || nfts.paginationResource.phase === "loading"}
      size="sm"
      variant="outline"
      onClick={onLoadMore}
    >
      {label}
    </Button>
  )
}

function NftSkeletons(): ReactElement {
  return (
    <div
      aria-label="Loading collectibles"
      className="-mx-5 mt-4 flex gap-3 overflow-hidden px-5"
      role="status"
    >
      {[0, 1, 2].map(index => (
        <div className="w-[10.5rem] shrink-0" key={index}>
          <Skeleton className="aspect-square w-full rounded-2xl" />
          <Skeleton className="mt-2.5 h-4 w-28" />
          <Skeleton className="mt-2 h-3 w-20" />
        </div>
      ))}
    </div>
  )
}

function NftLoadError({onRetry}: {readonly onRetry: () => Promise<void>}): ReactElement {
  return (
    <div className="mt-4 flex items-center justify-between gap-4 rounded-2xl border border-border bg-card px-4 py-3.5">
      <div className="flex min-w-0 items-center gap-3">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-full bg-secondary text-muted-foreground">
          <WarningCircle aria-hidden="true" size={18} />
        </span>
        <div className="min-w-0">
          <p className="text-sm font-medium">Collectibles unavailable</p>
          <p className="mt-0.5 truncate text-xs text-muted-foreground">
            The rest of your wallet is ready.
          </p>
        </div>
      </div>
      <Button size="sm" variant="ghost" onClick={onRetry}>
        Retry
      </Button>
    </div>
  )
}

function EmptyNfts(): ReactElement {
  return (
    <div className="mt-4 flex items-center gap-3 rounded-2xl border border-dashed border-border bg-card/60 px-4 py-4">
      <span className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-secondary text-muted-foreground">
        <ImageSquare aria-hidden="true" size={20} />
      </span>
      <div>
        <p className="text-sm font-medium">No collectibles yet</p>
        <p className="mt-0.5 text-xs text-muted-foreground">
          NFTs owned by this address appear here.
        </p>
      </div>
    </div>
  )
}
