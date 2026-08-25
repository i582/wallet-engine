import {
  detectMnemonicSchemes as rawDetectMnemonicSchemes,
  mnemonicWordlist as rawMnemonicWordlist,
} from "../../bindings/wasm/wallet_engine.js"

import {initializeWalletEngine} from "./initialize"

/**
 * A recovery-phrase scheme recognized by {@link detectMnemonicSchemes}.
 *
 * `rotation` is the TEP-0003 rotation mnemonic, the only scheme wallet import
 * accepts. `ton` is the passwordless legacy 24-word TON mnemonic and `bip39`
 * is the standard 24-word BIP-39 (Multichain) phrase; both are detected so the
 * application can explain a rejection, and are never imported.
 */
export type MnemonicScheme = "rotation" | "ton" | "bip39"

/** Returns the 2048 English BIP-39 words accepted by recovery-phrase validation. */
export async function mnemonicWordlist(): Promise<readonly string[]> {
  await initializeWalletEngine()
  return rawMnemonicWordlist() as string[]
}

/**
 * Reports every recovery scheme under which the entered words validate.
 *
 * Pass the words exactly as the user recorded them, one word per element - the
 * same value `importWallet` takes. The checks are independent, so one phrase
 * can match more than one scheme; an empty result means the words validate
 * under no scheme the engine knows. Import succeeds exactly when the result
 * contains `"rotation"`.
 */
export async function detectMnemonicSchemes(
  words: readonly string[],
): Promise<readonly MnemonicScheme[]> {
  await initializeWalletEngine()
  return rawDetectMnemonicSchemes([...words]) as MnemonicScheme[]
}
