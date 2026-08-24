import {
  convertTonAddress as rawConvertTonAddress,
  isValidTonAddress as rawIsValidTonAddress,
  parseTonAddress as rawParseTonAddress,
} from "../../bindings/wasm/wallet_engine.js"

import {initializeWalletEngine} from "./initialize"

/** A canonical raw address or a user-friendly address with explicit flags. */
export type TonAddressFormat =
  | {readonly kind: "raw"}
  | {
      readonly kind: "userFriendly"
      readonly bounceable: boolean
      readonly testnet: boolean
    }

/** Parsed TON account identity and the representation carried by the input. */
export interface TonAddressInfo {
  readonly raw: string
  readonly workchain: number
  readonly format: TonAddressFormat
}

/** Parses raw, standard-Base64, or URL-safe user-friendly TON address text. */
export async function parseTonAddress(value: string): Promise<TonAddressInfo> {
  await initializeWalletEngine()
  return rawParseTonAddress(value) as TonAddressInfo
}

/** Reports whether a string is a valid raw or user-friendly TON address. */
export async function isValidTonAddress(value: string): Promise<boolean> {
  await initializeWalletEngine()
  return rawIsValidTonAddress(value)
}

/** Converts an address to canonical raw or URL-safe user-friendly form. */
export async function convertTonAddress(value: string, format: TonAddressFormat): Promise<string> {
  await initializeWalletEngine()
  return rawConvertTonAddress(value, format)
}
