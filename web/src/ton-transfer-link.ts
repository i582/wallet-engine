import {parseTonTransferLink as rawParseTonTransferLink} from "../../bindings/wasm/wallet_engine.js"

import {initializeWalletEngine} from "./initialize"

/** The asset selected by a parsed TON transfer link. */
export type TonTransferAsset =
  | {readonly kind: "gram"}
  | {readonly kind: "jetton"; readonly master: string}

/** The optional message payload selected by a parsed TON transfer link. */
export type TonTransferPayload =
  | {readonly kind: "none"}
  | {readonly kind: "text"; readonly text: string}
  | {readonly kind: "boc"; readonly boc: string}

/** The expiration policy preserved without losing the full unsigned 64-bit range. */
export type TonTransferExpiration =
  | {readonly kind: "engineDefault"}
  | {readonly kind: "exact"; readonly unixTimestamp: bigint}

/** A syntax-validated transfer invoice that still requires admission and user approval. */
export interface ParsedTonTransferLink {
  readonly recipient: string
  readonly asset: TonTransferAsset
  readonly amount?: string
  readonly payload: TonTransferPayload
  readonly expiration: TonTransferExpiration
}

/** Parses the strict baseline `ton://transfer/` format without reading chain or clock state. */
export async function parseTonTransferLink(value: string): Promise<ParsedTonTransferLink> {
  await initializeWalletEngine()
  return rawParseTonTransferLink(value) as ParsedTonTransferLink
}
