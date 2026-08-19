import {ArrowClockwise, ArrowLeft, ImageSquare, WarningCircle} from "@phosphor-icons/react"
import type {NftList} from "@ton/wallet-engine"
import {type ReactElement, useId} from "react"

import {NftCard} from "@/components/nft-card"
import {Button} from "@/components/ui/button"
import {Skeleton} from "@/components/ui/skeleton"

export interface CollectiblesPageProps {
  readonly nfts: NftList
  readonly refreshing: boolean
  readonly loadingMore: boolean
  readonly onRefresh: () => Promise<void>
  readonly onLoadMore: () => Promise<void>
}

/** Full wallet inventory with responsive grid and independent pagination. */
export function CollectiblesPage({
  nfts,
  refreshing,
  loadingMore,
  onRefresh,
  onLoadMore,
}: CollectiblesPageProps): ReactElement {
  const titleId: string = useId()
  const initialLoading: boolean =
    nfts.items.length === 0 && (refreshing || nfts.resource.phase === "loading")
  const initialFailure: boolean = nfts.items.length === 0 && nfts.resource.phase === "failed"

  return (
    <main className="mx-auto flex h-[100dvh] w-full max-w-lg flex-col overflow-hidden bg-background px-5 pb-6 pt-4 sm:pb-8 sm:pt-5">
      <header className="flex min-h-11 items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <Button
            asChild
            aria-label="Back to wallet"
            className="-ml-2 size-9"
            size="icon"
            variant="ghost"
          >
            <a href="#/">
              <ArrowLeft aria-hidden="true" size={19} />
            </a>
          </Button>
          <div className="min-w-0">
            <h1 className="truncate text-lg font-semibold tracking-[-0.025em]" id={titleId}>
              Collectibles
            </h1>
            <p className="text-xs text-muted-foreground">
              {nfts.items.length} loaded {nfts.items.length === 1 ? "item" : "items"}
            </p>
          </div>
        </div>
        <Button
          aria-label="Refresh collectibles"
          className="size-9"
          disabled={refreshing}
          size="icon"
          variant="ghost"
          onClick={onRefresh}
        >
          <ArrowClockwise
            aria-hidden="true"
            className={refreshing ? "animate-spin" : undefined}
            size={18}
          />
        </Button>
      </header>

      <section aria-labelledby={titleId} className="mt-6 min-h-0 flex-1 overflow-y-auto pb-2">
        <p className="max-w-sm text-sm leading-6 text-muted-foreground">
          Every NFT currently owned by this wallet, ordered by its latest transaction.
        </p>

        {initialLoading ? <CollectiblesSkeleton /> : null}
        {initialFailure ? <CollectionFailure onRetry={onRefresh} /> : null}
        {!(initialLoading || initialFailure) && nfts.items.length === 0 ? (
          <EmptyCollection />
        ) : null}

        {nfts.items.length > 0 ? (
          <>
            {nfts.resource.phase === "failed" ? (
              <p className="mt-5 flex items-center gap-1.5 text-xs text-muted-foreground">
                <WarningCircle aria-hidden="true" size={14} /> Showing the last loaded collection.
              </p>
            ) : null}
            <div className="mt-5 grid grid-cols-2 gap-x-4 gap-y-6">
              {nfts.items.map(item => (
                <NftCard item={item} key={item.address} layout="grid" />
              ))}
            </div>
          </>
        ) : null}

        <PaginationButton loadingMore={loadingMore} nfts={nfts} onLoadMore={onLoadMore} />
      </section>
    </main>
  )
}

function PaginationButton({
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
      className="mt-7 w-full"
      disabled={loadingMore || nfts.paginationResource.phase === "loading"}
      variant="outline"
      onClick={onLoadMore}
    >
      {label}
    </Button>
  )
}

function CollectiblesSkeleton(): ReactElement {
  return (
    <div aria-label="Loading collectibles" className="mt-5 grid grid-cols-2 gap-4" role="status">
      {[0, 1, 2, 3].map(index => (
        <div key={index}>
          <Skeleton className="aspect-square w-full rounded-2xl" />
          <Skeleton className="mt-2.5 h-4 w-24" />
          <Skeleton className="mt-2 h-3 w-16" />
        </div>
      ))}
    </div>
  )
}

function CollectionFailure({onRetry}: {readonly onRetry: () => Promise<void>}): ReactElement {
  return (
    <div className="mt-6 rounded-2xl border border-border bg-card p-5">
      <WarningCircle aria-hidden="true" className="text-muted-foreground" size={24} />
      <p className="mt-4 text-sm font-semibold">Collectibles unavailable</p>
      <p className="mt-1 text-sm text-muted-foreground">
        The provider did not return this wallet’s inventory.
      </p>
      <Button className="mt-4" size="sm" variant="outline" onClick={onRetry}>
        Retry
      </Button>
    </div>
  )
}

function EmptyCollection(): ReactElement {
  return (
    <div className="mt-10 flex flex-col items-center text-center">
      <span className="flex size-14 items-center justify-center rounded-2xl bg-secondary text-muted-foreground">
        <ImageSquare aria-hidden="true" size={26} />
      </span>
      <p className="mt-4 text-sm font-semibold">No collectibles yet</p>
      <p className="mt-1 max-w-64 text-sm leading-5 text-muted-foreground">
        NFTs owned by this address will appear here automatically.
      </p>
    </div>
  )
}
