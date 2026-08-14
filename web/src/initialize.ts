import initWasm, {type InitInput} from "../../bindings/wasm/wallet_engine.js"
import {walletEngineWasmBase64} from "../../bindings/wasm/wallet_engine_bg_base64"

let initialization: Promise<unknown> | undefined

export function initializeWalletEngine(input?: InitInput): Promise<unknown> {
  if (input === undefined) {
    // The property name is part of the generated wasm-bindgen API.
    // biome-ignore lint/style/useNamingConvention: Keep the generated API name.
    initialization ??= initWasm({module_or_path: decodeBase64(walletEngineWasmBase64)})
  } else {
    // The property name is part of the generated wasm-bindgen API.
    // biome-ignore lint/style/useNamingConvention: Keep the generated API name.
    initialization ??= initWasm({module_or_path: input})
  }
  return initialization
}

function decodeBase64(value: string): Uint8Array {
  const binary: string = globalThis.atob(value)
  const bytes: Uint8Array = new Uint8Array(binary.length)
  for (let index: number = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index)
  }
  return bytes
}
