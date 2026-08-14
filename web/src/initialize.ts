import initWasm, {type InitInput} from "../../bindings/wasm/wallet_engine.js"

let initialization: Promise<unknown> | undefined

export function initializeWalletEngine(input?: InitInput): Promise<unknown> {
  if (input === undefined) {
    initialization ??= initWasm()
  } else {
    // The property name is part of the generated wasm-bindgen API.
    // biome-ignore lint/style/useNamingConvention: Keep the generated API name.
    initialization ??= initWasm({module_or_path: input})
  }
  return initialization
}
