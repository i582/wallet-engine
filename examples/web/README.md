# Web wallet example

This example shows the smallest complete browser flow for `@ton/wallet-engine`.
It creates a testnet wallet. Then it loads the account and activity
snapshot through the browser host callbacks.

Paste a full `tc://` link to start TON Connect. The example loads the dApp
manifest, shows connection approval, signs an optional `ton_proof`, previews
raw batches of up to 255 messages, and restores the encrypted session after
reload.

The approval dialog supports `sendTransaction` and gasless `signMessage`.
For `signMessage`, it shows that a relayer submits the signed request and pays
the inbound TON fee. The dApp receives the complete signed internal-message
BoC.

The approval dialog lists every message in order. Each entry shows its amount,
recipient, body type, and `StateInit` presence.

The example reports Tonkeeper's registered `appName` so current dApps can find
wallet metadata. A production wallet must register and use its own identity.

The send screen calls `previewSend` before confirmation. It shows Toncenter
fees and actions when they are available. A preview failure displays a warning
and keeps the **Send anyway** action available.

The later `send` call loads a new sequence number and builds a new signed
message. It does not reuse the preview message.

The interface also reads the public GRAM/USD market rate from TonAPI. This
example-only request is not part of Wallet Engine or its host callback API.

The example keeps interface code separate from engine code:

- `src/lib/wallet-session.ts` contains the Wallet Engine integration.
- `src/lib/browser-wallet-store.ts` persists the descriptor and secret host data.
- `src/components/ton-connect-dialog.tsx` shows connection and transaction approvals.
- `src/components/` contains the React interface.
- `src/components/ui/` contains the local shadcn components.

## Run the example

Run these commands from the repository root:

```sh
just bindings-wasm
just example-web-install
just example-web-dev
```

Then open `http://localhost:3000`.

Toncenter permits public testnet requests with a low rate limit. You can copy
`.env.example` to `.env.local` and add a restricted testnet key.

CAUTION: A Vite environment value is public browser data. Do not use a private
service credential in this example.

## Security scope

`BrowserWalletStore` keeps the descriptor and recovery phrase in IndexedDB, so
the example wallet remains available after a reload.

This behavior is useful for an example. IndexedDB is not protected wallet
storage. Use an external signer or an audited encrypted browser vault for a
production wallet.

The example also uses IndexedDB for the durable send journal and TON Connect
session. The TON Connect value contains a session secret key. Use protected
storage in a production wallet.

Read [TON_CONNECT.md](../../TON_CONNECT.md) for the complete integration and
security contract.
