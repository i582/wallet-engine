# TypeScript and WebAssembly

Wallet Engine provides a WebAssembly package and a high-level TypeScript API.
The local TypeScript package is named `@ton/wallet-engine`.

This repository does not publish the package to npm. Link the `web` directory
as a workspace package while you develop an application.

Each tagged release contains `ton-wallet-engine-VERSION.tgz`. The archive has
the TypeScript API, declarations, JavaScript module, and WebAssembly runtime.

Install the downloaded archive with your package manager. For example:

```shell
npm install ./ton-wallet-engine-0.1.0.tgz
```

The package contains no user interface. Your application owns its screens,
routing, wallet metadata, and stream connections.

## Generate the WebAssembly package

Install the repository tools first:

```shell
just install-tools
```

Generate the package:

```shell
just bindings-wasm
```

The command writes generated files to `bindings/wasm`. Git ignores this
directory. Generate the files from the same revision as the TypeScript source.

The generated package uses the `web` target of `wasm-pack`. It contains the
WebAssembly binary, JavaScript glue, and TypeScript declarations.

## Check the TypeScript package

Install the package dependencies:

```shell
just web-install
```

Run all TypeScript checks:

```shell
just web-fmt-check
just web-lint
just web-build
just web-test
```

The package follows the Bun, Biome, and TypeScript rules from Acton. The tests
load the real WebAssembly binary. They also run Rust callbacks through
TypeScript host objects.

## Initialize the engine

Link the `web` directory as `@ton/wallet-engine` in your workspace. Then import
the high-level API:

```ts
import {
  BrowserPlatformHost,
  IndexedDbJournalStore,
  WalletClient,
  WalletLifecycle,
  initializeWalletEngine,
  type ProtectedSecretStoreHost,
} from "@ton/wallet-engine"

await initializeWalletEngine()
```

A bundler can load the adjacent `wallet_engine_bg.wasm` file. You can also pass
an explicit `InitInput` to `initializeWalletEngine`.

## Address utilities

The high-level package validates raw and user-friendly addresses, exposes the
friendly flags, and converts between canonical formats:

```ts
import {
  convertTonAddress,
  isValidTonAddress,
  parseTonAddress,
} from "@ton/wallet-engine"

const info = await parseTonAddress(input)
const valid = await isValidTonAddress(input)
const raw = await convertTonAddress(input, {kind: "raw"})
const display = await convertTonAddress(raw, {
  kind: "userFriendly",
  bounceable: false,
  testnet: false,
})
```

`info.format` contains either `{kind: "raw"}` or the parsed user-friendly
flags. Both standard and URL-safe Base64 friendly inputs are accepted. Friendly
output is canonical unpadded URL-safe Base64.

## Mnemonic word list

Use the engine's BIP-39 word list for recovery-phrase input:

```ts
import {mnemonicWordlist} from "@ton/wallet-engine"

const words = await mnemonicWordlist()
const suggestions = words.filter(word => word.startsWith(input.toLowerCase()))
```

The list contains the 2048 English BIP-39 words accepted by the same
recovery-phrase validation used for wallet import. Its order is the BIP-39
index order.

## Implement protected storage

The package does not include an insecure recovery-phrase store. Implement the
`ProtectedSecretStoreHost` interface for your product.

```ts
const secrets: ProtectedSecretStoreHost = {
  async read(request) {
    return vault.read(request.secretRef.value, request.reason)
  },
  async store(request) {
    await vault.store(request.secretRef.value, new Uint8Array(request.bytes))
  },
  async delete(secretRef) {
    await vault.delete(secretRef.value)
  },
}

const platformHost = new BrowserPlatformHost({
  secrets,
  journal: new IndexedDbJournalStore(),
})
```

CAUTION: Do not store a recovery phrase as plain text in local storage or
IndexedDB. Same-origin JavaScript can read unprotected browser data.

The browser cannot provide the same security boundary as Keychain or Android
Keystore. Use an external signer when your product requires that boundary.

## Create a wallet client

Create one HTTP host for each client. The high-level API does this for you.

```ts
const client = await WalletClient.create(
  {
    recordId: descriptor.recordId,
    address: descriptor.address,
    network: descriptor.network,
    providers: {
      toncenterBaseUrl: "https://testnet.toncenter.com",
      requestTimeoutMs: 15_000,
    },
  },
  {platformHost},
)

const update = await client.refresh()
console.log(update.snapshot.account)
```

Every nonzero transfer in `update.snapshot.activity.items` includes
`transactionFeeNanograms`, `status` (`"success"`, `"failed"`, or `"bounced"`),
and an optional decoded plaintext `comment`. The fee is the total fee for the
transaction. It is repeated when one transaction produces multiple activity
rows, so do not sum it per row.

Create an encrypted-comment BOC before previewing the transfer:

```ts
const boc = await client.createEncryptedComment({
  recipient: destination,
  comment: "private hello",
})

const body = {kind: "rawPayload" as const, boc}
```

The engine loads the recipient's `get_public_key` and asks the platform host
for the protected mnemonic with reason `"encryptComment"`. Put `body` into the
same immutable intent passed to `previewSend` and `send`.

Encrypted activity exposes `encryptedComment` instead of `comment`. Decryption
is explicit, so refresh never opens an authentication prompt:

```ts
const sender = item.direction === "received" ? item.counterparty : config.address
if (item.encryptedComment && sender) {
  const comment = await client.decryptComment({
    sender,
    body: item.encryptedComment,
  })
}
```

This protected-secret read uses reason `"decryptComment"`. Plaintext is
limited to 960 UTF-8 bytes.

An NFT that belongs to a collection includes a neutral `collection` descriptor
with its address, standard TEP-64 fields, and complete string metadata. Keep
product-specific classification, such as Telegram gifts, in the application.

Call `close()` before you discard the client:

```ts
await client.close()
```

## Use a Toncenter API key

Do not put a private service credential in a browser bundle. Use a public user
credential or a backend proxy.

Pass a user-owned key to the browser host. The host adds it only to requests
whose origin matches `toncenterBaseUrl`:

```ts
const client = await WalletClient.create(config, {
  platformHost,
  toncenterApiKey,
})
```

The browser HTTP host rejects redirects. It also enforces the response limits
from Rust. The host uses `AbortController` for cancellation.

## Create and import wallets

Use `WalletLifecycle` for recovery-phrase operations:

```ts
const lifecycle = await WalletLifecycle.create(platformHost)

const created = await lifecycle.createWallet({
  recordId: crypto.randomUUID(),
  network: "testnet",
})

showRecoveryPhrase(created.recoveryPhrase.phrase)
saveWalletDescriptor(created.descriptor)
```

Discard the recovery phrase after the required user flow. Persist only the
wallet descriptor.

## Use TON Connect

Create one `TonConnectWallet` for the active wallet descriptor. Supply your
wallet registry identifier and application version.

```ts
import {TonConnectWallet, type TonConnectWalletEvent} from "@ton/wallet-engine"

const tonConnect = new TonConnectWallet({
  descriptor,
  walletClient: client,
  lifecycle,
  identity: {
    appName: "your-wallet-registry-id",
    appVersion: "1.0.0",
  },
  storage: tonConnectStorage,
})

const unsubscribe = tonConnect.onEvent((event: TonConnectWalletEvent) => {
  if (event.kind !== "interaction") {
    return
  }
  showTonConnectApproval(event.interaction, approved => {
    tonConnect.respond(event.interaction.id, approved)
  })
})

await tonConnect.restore()

async function openTonConnectLink(connectionLink: string): Promise<void> {
  await tonConnect.start(connectionLink)
}
```

The `interaction` event contains either connection details or a transaction
preview. Show the preview before you call `respond` for a transaction.

`TonConnectStorage` stores a secret-bearing session record. Use an
encrypted browser vault or another protected store. Plain `localStorage` and
IndexedDB do not protect this record from same-origin JavaScript.

Call `disconnect()` to notify the dApp and remove the session. Call `close()`
to stop transport while keeping the session available for restoration.

Read [TON_CONNECT.md](TON_CONNECT.md) for protocol limits, transaction mapping,
storage requirements, and native integration.

## Clear client secret copies

Rust clears the secret buffers that it owns. JavaScript strings are immutable,
so the application cannot reliably clear `RecoveryPhrase.phrase` in memory.

Keep the phrase only while the user needs it. Do not write it to logs, errors,
analytics, state snapshots, or browser storage.

Use a `Uint8Array` for mutable secret copies. Call `fill(0)` in a `finally`
block after the host callback finishes.

## Store send records

`IndexedDbJournalStore` uses one IndexedDB transaction for each compare-and-swap
operation. This journal stores the exact signed BoC before submission.

Browser storage can be cleared or evicted. Keep a recovery path for the user.
Do not automatically create a new transfer after `submissionUnknown`.
After showing the unresolved transfer, an application can request explicit user
confirmation and resend with `SendRequest.force = true`. The original transfer
can still execute, so both transfers can affect the wallet balance.

Use `{ kind: "exact", unixTimestamp: value }` for an exact expiration in the
Web API. TON Connect uses `valid_until` on the protocol wire and `validUntil` in
the dApp SDK; those names are not `SendExpiration` fields.

## Chain streaming

`WalletClient` does not contain chain streaming methods. The TypeScript
application owns its chain stream and reconnect policy. `TonConnectWallet`
separately owns its TON Connect bridge stream.

## Browser lifecycle

Browsers can freeze or discard pages. Cancel active work when the page stops.
Create a new client and refresh wallet data when the page resumes.

The WebAssembly wrapper uses a single-thread browser executor. Do not move one
client between Web Workers. Create a separate client inside each worker.
