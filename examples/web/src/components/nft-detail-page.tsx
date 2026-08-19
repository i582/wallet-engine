import {ArrowLeft, PaperPlaneTilt, ShoppingBagOpen, WarningCircle} from "@phosphor-icons/react"
import type {NftItem, SendPreview, SendResult} from "@ton/wallet-engine"
import {type ReactElement, type ReactNode, useState} from "react"

import {NftArtwork} from "@/components/nft-card"
import {NftSendForm} from "@/components/nft-send-form"
import {Button} from "@/components/ui/button"
import {compactAddress} from "@/lib/format"
import {nftCollectionName, nftDisplayName} from "@/lib/nft-display"

export interface NftDetailPageProps {
  readonly item?: NftItem
  readonly hasMore: boolean
  readonly loadingMore: boolean
  readonly canForceRetry: boolean
  readonly onLoadMore: () => Promise<void>
  readonly onCancelPreview: () => Promise<void>
  readonly onPreviewTransfer: (
    operationId: string,
    nftAddress: string,
    recipient: string,
  ) => Promise<SendPreview>
  readonly onSendTransfer: (
    operationId: string,
    nftAddress: string,
    recipient: string,
    force: boolean,
  ) => Promise<SendResult>
}

/** Dedicated page for one loaded collectible and its indexed ownership metadata. */
export function NftDetailPage({
  item,
  hasMore,
  loadingMore,
  canForceRetry,
  onLoadMore,
  onCancelPreview,
  onPreviewTransfer,
  onSendTransfer,
}: NftDetailPageProps): ReactElement {
  const [sending, setSending] = useState<boolean>(false)
  if (!item) {
    return (
      <NftPageFrame title="Collectible">
        <div className="mt-24 flex flex-col items-center text-center">
          <WarningCircle aria-hidden="true" className="text-muted-foreground" size={30} />
          <p className="mt-4 text-sm font-semibold">Collectible is not loaded</p>
          <p className="mt-1 max-w-72 text-sm leading-5 text-muted-foreground">
            Open it from the collection, or load older items and try again.
          </p>
          {hasMore ? (
            <Button className="mt-5" disabled={loadingMore} variant="outline" onClick={onLoadMore}>
              {loadingMore ? "Loading collectibles" : "Load older collectibles"}
            </Button>
          ) : null}
        </div>
      </NftPageFrame>
    )
  }

  const name: string = nftDisplayName(item)
  const collection: string =
    nftCollectionName(item) ??
    (item.collectionAddress ? compactAddress(item.collectionAddress) : "Independent item")
  const description: string | undefined = item.content.description?.trim() || undefined

  return (
    <NftPageFrame title={name}>
      <div className="group relative aspect-square overflow-hidden rounded-[1.75rem] border border-border bg-secondary">
        <NftArtwork item={item} name={name} />
        {item.onSale ? (
          <span className="absolute left-4 top-4 flex items-center gap-1.5 rounded-full bg-background/90 px-3 py-1.5 text-xs font-semibold shadow-sm backdrop-blur">
            <ShoppingBagOpen aria-hidden="true" size={15} /> On sale
          </span>
        ) : null}
      </div>

      <div className="px-1 pb-3 pt-6">
        <p className="text-sm font-medium text-primary">{collection}</p>
        <h1 className="mt-1 text-3xl font-semibold tracking-[-0.045em]">{name}</h1>
        {description ? (
          <p className="mt-4 text-sm leading-6 text-muted-foreground">{description}</p>
        ) : null}

        <Button
          className="mt-6 w-full"
          disabled={!item.initialized || item.onSale}
          type="button"
          onClick={() => setSending(true)}
        >
          <PaperPlaneTilt aria-hidden="true" size={18} />
          {item.onSale ? "Unavailable while on sale" : "Send collectible"}
        </Button>

        <dl className="mt-8 divide-y divide-border border-y border-border">
          <MetadataRow label="NFT address" value={item.address} />
          {item.collectionAddress ? (
            <MetadataRow label="Collection" value={item.collectionAddress} />
          ) : null}
          {item.ownerAddress ? <MetadataRow label="Owner" value={item.ownerAddress} /> : null}
          <MetadataRow label="Index" value={item.index} />
          <MetadataRow label="Last transaction LT" value={item.lastTransactionLt} />
          <MetadataRow label="Initialized" value={item.initialized ? "Yes" : "No"} />
        </dl>
      </div>
      {sending ? (
        <NftSendForm
          canForceRetry={canForceRetry}
          item={item}
          onCancelPreview={onCancelPreview}
          onClose={() => setSending(false)}
          onPreview={onPreviewTransfer}
          onSend={onSendTransfer}
        />
      ) : null}
    </NftPageFrame>
  )
}

function NftPageFrame({
  children,
  title,
}: {
  readonly children: ReactNode
  readonly title: string
}): ReactElement {
  return (
    <main className="mx-auto flex h-[100dvh] w-full max-w-lg flex-col overflow-hidden bg-background px-5 pb-6 pt-4 sm:pb-8 sm:pt-5">
      <header className="flex min-h-11 items-center gap-2">
        <Button
          asChild
          aria-label="Back to collectibles"
          className="-ml-2 size-9"
          size="icon"
          variant="ghost"
        >
          <a href="#/collectibles">
            <ArrowLeft aria-hidden="true" size={19} />
          </a>
        </Button>
        <p className="truncate text-sm font-semibold">{title}</p>
      </header>
      <section className="mt-5 min-h-0 flex-1 overflow-y-auto pb-2">{children}</section>
    </main>
  )
}

function MetadataRow({
  label,
  value,
}: {
  readonly label: string
  readonly value: string
}): ReactElement {
  return (
    <div className="grid grid-cols-[7.5rem_minmax(0,1fr)] gap-4 py-3.5 text-sm">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="break-all text-right font-medium">{value}</dd>
    </div>
  )
}
