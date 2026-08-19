export type NftRoute =
  | {readonly kind: "wallet"}
  | {readonly kind: "collection"}
  | {readonly kind: "detail"; readonly address: string}

const COLLECTION_PATH = "/collectibles"

/** Parses the dependency-free hash routes used by the static Web example. */
export function parseNftRoute(hash: string): NftRoute {
  const path: string = hash.startsWith("#") ? hash.slice(1) : hash
  if (path === COLLECTION_PATH || path === `${COLLECTION_PATH}/`) {
    return {kind: "collection"}
  }
  const detailPrefix = `${COLLECTION_PATH}/`
  if (!path.startsWith(detailPrefix)) {
    return {kind: "wallet"}
  }
  const encodedAddress: string = path.slice(detailPrefix.length)
  if (!encodedAddress || encodedAddress.includes("/")) {
    return {kind: "collection"}
  }
  try {
    const address: string = decodeURIComponent(encodedAddress).trim()
    return address ? {kind: "detail", address} : {kind: "collection"}
  } catch {
    return {kind: "collection"}
  }
}

export function nftDetailHash(address: string): string {
  return `#${COLLECTION_PATH}/${encodeURIComponent(address)}`
}
