import {
  BrowserPlatformHost,
  WalletLifecycle,
  type WalletClientConfig,
  type WalletDescriptor,
} from "@ton/wallet-engine"

import {createHttpClient} from "./http-transport"
import {BodyOnlyFetchRelay, createRelayClient} from "./relay-transport"

const secrets = new Map<string, Uint8Array>()
const transport = selectedTransport()

const platformHost = new BrowserPlatformHost({
  secrets: {
    read: async request => {
      const value = secrets.get(request.secretRef.value)
      if (!value) {
        throw Object.assign(new Error("Secret not found"), {
          kind: "notFound",
          diagnostic: "Secret not found",
        })
      }
      return value.slice()
    },
    store: async request => {
      secrets.set(request.secretRef.value, new Uint8Array(request.bytes))
    },
    delete: async secretRef => {
      secrets.delete(secretRef.value)
    },
  },
  journal: {
    load: async () => undefined,
    compareExchange: async mutation => ({
      applied: true,
      current: mutation.replacement,
    }),
  },
})

const lifecycle = await WalletLifecycle.create(platformHost)
const created = await lifecycle.createWallet({
  recordId: "provider-transport-demo",
  network: "testnet",
})
const config = clientConfig(created.descriptor)

const client =
  transport === "http"
    ? await createHttpClient(config, platformHost)
    : await createRelayClient(
        config,
        platformHost,
        new BodyOnlyFetchRelay(config.providers.toncenterBaseUrl),
      )

try {
  const update = await client.refresh()

  console.log({
    transport,
    outcome: update.outcome,
    account: update.snapshot.account,
  })
} finally {
  await client.close()
  lifecycle.close()
}

function selectedTransport(): "http" | "relay" {
  const runtime = globalThis as typeof globalThis & {
    readonly Bun?: {readonly argv: readonly string[]}
  }
  const value = runtime.Bun?.argv.at(-1)
  if (value === "http" || value === "relay") {
    return value
  }
  throw new Error("Usage: bun --cwd examples/provider-transports-ts <http|relay>")
}

function clientConfig(descriptor: WalletDescriptor): WalletClientConfig {
  return {
    recordId: descriptor.recordId,
    address: descriptor.address,
    publicKey: descriptor.publicKey,
    network: descriptor.network,
    sendValiditySeconds: 300,
    resolutionMarginSeconds: 60,
    providers: {
      toncenterBaseUrl: "https://testnet.toncenter.com",
      requestTimeoutMs: 15_000,
    },
  }
}
