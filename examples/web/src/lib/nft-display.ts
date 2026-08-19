import type {NftItem} from "@ton/wallet-engine"

const IMAGE_FIELDS: readonly string[] = [
  "_image_medium",
  "preview",
  "image_url",
  "image",
  "_image_small",
  "_image_big",
]
const IPFS_PATH_PREFIX = /^ipfs\//u
const DATA_IMAGE_TYPES = new Set<string>([
  "image/avif",
  "image/gif",
  "image/jpeg",
  "image/png",
  "image/svg+xml",
  "image/webp",
])

/** Returns the indexed NFT name or a deterministic item-index fallback. */
export function nftDisplayName(item: NftItem): string {
  return firstText(item.content.name, item.content.domain) ?? `NFT #${item.index}`
}

/** Returns the indexed collection label when the provider supplied one. */
export function nftCollectionName(item: NftItem): string | undefined {
  return firstText(item.content.collection_name, item.content.collection)
}

/** Selects the best indexed artwork URL and converts IPFS URIs for browsers. */
export function nftImageUrl(item: NftItem): string | undefined {
  for (const field of IMAGE_FIELDS) {
    const normalized: string | undefined = normalizeNftMediaUrl(item.content[field])
    if (normalized) {
      return normalized
    }
  }
  return undefined
}

/** Allows only browser-loadable HTTP(S), IPFS, and whitelisted inline image references. */
export function normalizeNftMediaUrl(value: string | undefined): string | undefined {
  const candidate: string | undefined = firstText(value)
  if (!candidate) {
    return undefined
  }
  if (candidate.startsWith("ipfs://")) {
    const path: string = candidate.slice("ipfs://".length).replace(IPFS_PATH_PREFIX, "")
    return path.length > 0 ? `https://ipfs.io/ipfs/${path}` : undefined
  }
  if (candidate.toLowerCase().startsWith("data:")) {
    return isSafeDataImage(candidate) ? candidate : undefined
  }
  try {
    const url = new URL(candidate)
    return url.protocol === "https:" || url.protocol === "http:" ? url.toString() : undefined
  } catch {
    return undefined
  }
}

function isSafeDataImage(candidate: string): boolean {
  const separator: number = candidate.indexOf(",")
  if (separator < 6 || separator === candidate.length - 1) {
    return false
  }
  const [mediaType, ...parameters] = candidate
    .slice("data:".length, separator)
    .toLowerCase()
    .split(";")
  if (!(mediaType && DATA_IMAGE_TYPES.has(mediaType))) {
    return false
  }
  return parameters.every(parameter =>
    ["base64", "charset=utf-8", "utf8"].includes(parameter.trim()),
  )
}

function firstText(...values: readonly (string | undefined)[]): string | undefined {
  for (const value of values) {
    const trimmed: string | undefined = value?.trim()
    if (trimmed) {
      return trimmed
    }
  }
  return undefined
}
