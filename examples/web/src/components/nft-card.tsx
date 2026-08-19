import {EyeSlash, ImageSquare, ShieldWarning} from "@phosphor-icons/react"
import type {NftItem} from "@ton/wallet-engine"
import {type ReactElement, useState} from "react"

import {compactAddress} from "@/lib/format"
import {nftCollectionName, nftDisplayName, nftImageUrl} from "@/lib/nft-display"
import {nftDetailHash} from "@/lib/nft-route"

export interface NftCardProps {
  readonly item: NftItem
  readonly layout?: "grid" | "shelf"
}

/** Links one collectible preview to its dedicated detail page. */
export function NftCard({item, layout = "shelf"}: NftCardProps): ReactElement {
  const name: string = nftDisplayName(item)
  const collection: string =
    nftCollectionName(item) ??
    (item.collectionAddress ? compactAddress(item.collectionAddress) : "Independent item")
  const widthClass: string = layout === "shelf" ? "w-[10.5rem] shrink-0 snap-start" : "min-w-0"

  return (
    <a
      aria-label={`Open ${name}`}
      className={`group block text-left outline-none focus-visible:rounded-2xl focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background ${widthClass}`}
      data-nft-address={item.address}
      data-nft-index={item.index}
      href={nftDetailHash(item.address)}
    >
      <div className="relative aspect-square overflow-hidden rounded-2xl border border-border/80 bg-secondary">
        <NftArtwork item={item} name={name} />
        {item.onSale ? (
          <span className="absolute left-2 top-2 rounded-full bg-background/90 px-2 py-1 text-[10px] font-semibold text-foreground shadow-sm backdrop-blur">
            On sale
          </span>
        ) : null}
      </div>
      <p className="mt-2.5 truncate text-sm font-semibold tracking-[-0.01em]" title={name}>
        {name}
      </p>
      <p className="mt-0.5 truncate text-xs text-muted-foreground" title={collection}>
        {collection}
      </p>
    </a>
  )
}

export function NftArtwork({
  item,
  name = nftDisplayName(item),
}: {
  readonly item: NftItem
  readonly name?: string
}): ReactElement {
  const [failed, setFailed] = useState<boolean>(false)
  const imageUrl: string | undefined = nftImageUrl(item)

  if (item.isScam === true) {
    return (
      <ArtworkFallback
        icon={<ShieldWarning aria-hidden="true" size={28} />}
        label="Reported item"
        tone="danger"
      />
    )
  }
  if (item.isNsfw === true) {
    return (
      <ArtworkFallback
        icon={<EyeSlash aria-hidden="true" size={28} />}
        label="Sensitive media"
        tone="muted"
      />
    )
  }
  if (!imageUrl || failed) {
    return (
      <ArtworkFallback
        icon={<ImageSquare aria-hidden="true" size={30} />}
        label="Artwork unavailable"
        tone="default"
      />
    )
  }

  return (
    // biome-ignore lint: Vite has no image optimizer; onError only observes a failed media load.
    <img
      alt={`${name} artwork`}
      className="size-full object-cover transition-transform duration-500 ease-out group-hover:scale-[1.035]"
      height={640}
      loading="lazy"
      referrerPolicy="no-referrer"
      src={imageUrl}
      width={640}
      onError={() => setFailed(true)}
    />
  )
}

function ArtworkFallback({
  icon,
  label,
  tone,
}: {
  readonly icon: ReactElement
  readonly label: string
  readonly tone: "default" | "danger" | "muted"
}): ReactElement {
  const toneClass: Record<typeof tone, string> = {
    default: "text-primary/80",
    danger: "bg-destructive/10 text-destructive",
    muted: "text-muted-foreground",
  }
  return (
    <div
      className={`flex size-full flex-col items-center justify-center gap-2 bg-[radial-gradient(circle_at_30%_20%,color-mix(in_srgb,var(--primary)_18%,transparent),transparent_42%)] ${toneClass[tone]}`}
    >
      {icon}
      <span className="text-[10px] font-semibold uppercase tracking-[0.11em]">{label}</span>
    </div>
  )
}
