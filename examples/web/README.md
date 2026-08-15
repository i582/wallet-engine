# Web wallet example

This example shows the smallest complete browser flow for `@ton/wallet-engine`.
It creates a Wallet V5R1 testnet wallet. Then it loads the account, jettons, and activity
snapshot through the browser host callbacks.

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

The example also uses IndexedDB for the durable send journal.
