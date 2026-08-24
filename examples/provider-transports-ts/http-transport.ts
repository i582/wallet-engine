import {
  WalletClient,
  type BrowserHttpHostOptions,
  type BrowserPlatformHost,
  type WalletClientConfig,
} from "@ton/wallet-engine"

/**
 * Creates a client backed by BrowserHttpHost.
 *
 * The host returns status, headers, body, and finalUrl, allowing the engine to
 * apply HTTP retry, redirect, and origin rules.
 */
export async function createHttpClient(
  config: WalletClientConfig,
  platformHost: BrowserPlatformHost,
  options: BrowserHttpHostOptions = {},
): Promise<WalletClient> {
  return WalletClient.create(config, {
    platformHost,
    ...options,
  })
}
