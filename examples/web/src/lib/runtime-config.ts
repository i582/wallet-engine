export interface WalletRuntimeConfig {
  /** HTTP bridge used for new and restored TON Connect sessions. */
  readonly tonConnectBridgeUrl?: string
}

declare global {
  var walletEngineConfig: WalletRuntimeConfig | undefined
}

/** Returns the bridge configured by the host page or the Web build. */
export function tonConnectBridgeUrl(): string | undefined {
  return (
    globalThis.walletEngineConfig?.tonConnectBridgeUrl ??
    import.meta.env.VITE_TON_CONNECT_BRIDGE_URL
  )
}
