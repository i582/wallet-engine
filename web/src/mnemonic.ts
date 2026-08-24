import {mnemonicWordlist as rawMnemonicWordlist} from "../../bindings/wasm/wallet_engine.js"

import {initializeWalletEngine} from "./initialize"

/** Returns the 2048 English words accepted by TON mnemonic validation. */
export async function mnemonicWordlist(): Promise<readonly string[]> {
  await initializeWalletEngine()
  return rawMnemonicWordlist() as string[]
}
