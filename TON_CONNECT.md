# TON Connect wallet integration

Wallet Engine implements the wallet side of TON Connect protocol version 2.
It supports encrypted HTTP bridge sessions, `ton_addr`, `ton_proof`, raw
`sendTransaction` requests, and disconnect requests.

The host application still owns network transport, protected storage, and all
approval screens. Private wallet keys and recovery phrases never enter a dApp
or bridge request.

## Choose an API

| Integration | API | Host responsibilities |
| --- | --- | --- |
| Swift, Kotlin, or generated C++ | `TonConnectSession` from the root crate | Manifest HTTP, bridge HTTP and SSE, protected session storage, approvals |
| Native Rust without FFI | `ton-connect-client` | The same work, plus wallet-specific request mapping |
| Browser TypeScript | `TonConnectWallet` from `@ton/wallet-engine` | Session storage and approval UI |

The TypeScript runtime includes its own bridge and manifest transport. The
native APIs return complete URLs and encrypted bodies for the host to send.

## Supported protocol surface

| Request or feature | Behavior |
| --- | --- |
| Connect link | Full TON Connect v2 links with one `ton_addr` item |
| `ton_proof` | Optional proof challenge during connection |
| `sendTransaction` | One raw internal message |
| Message amount | Exact nanogram amount |
| Message body | Empty body or one Base64-encoded payload cell |
| Contract deployment | Optional Base64-encoded destination `StateInit` |
| Expiration | dApp `valid_until`, or the engine policy when it is absent |
| Disconnect | dApp-initiated and wallet-initiated disconnect |
| Session recovery | Session keys, replay state, SSE cursor, pending requests, and pending responses |

The wallet advertises `SendTransaction` with `maxMessages: 1` and
`extraCurrencySupported: false`. The current implementation does not support
these requests:

- transactions with multiple messages.
- structured transaction items.
- extra currencies.
- `signData` and `signMessage`.
- embedded connect-and-act requests.

The engine compares each request network and source address with the connected
account. Mainnet uses `-239`. Testnet uses `-3`. A mismatch produces a bad
request and never reaches transaction approval.

## Send intent model

Regular transfers and TON Connect requests use the same `SendIntent` model:

- `SendExpiration.engineDefault` uses fresh provider time and
  `sendValiditySeconds`.
- `SendExpiration.exact` preserves the dApp `valid_until` timestamp.
- `SendMessageBody.empty` creates a value-only message.
- `SendMessageBody.comment` creates a standard text comment.
- `SendMessageBody.rawPayload` preserves a caller-built body cell.
- `SendMessage.stateInit` attaches a destination contract `StateInit`.

`stateInit` is independent of the message body. A deploy request can contain an
empty body, a comment, or a raw payload.

Use `previewTonConnect` for a `SendRequest` from TON Connect. The preview uses
the exact expiration, payload, and `StateInit` from the dApp request. Then show
the returned fees, actions, warnings, destination, amount, payload presence,
and deployment state before approval.

After approval, pass the same `SendRequest` to `send`. Return the signed BoC to
the dApp only for `submitted`, `submissionUnknown`, or `confirmed`. A
`submissionUnknown` result can already be in the network, so do not sign a
replacement automatically. TON Connect requests decode with `force = false`.
If the user explicitly approves a replacement, the wallet can set `force =
true` on the decoded `SendRequest`; the original transfer can still execute.

## Native session flow

The root crate exposes an FFI-safe `TonConnectSession`. Generated bindings use
the naming rules of Swift, Kotlin, and C++.

1. Create `TonConnectSessionConfig` with the wallet bridge URL, an SSE event
   size limit, and a message TTL.
2. Call `ton_connect_session_from_link` with the complete connection link.
3. Read `connect_prompt` and compare its requested network with the selected
   wallet.
4. Fetch the manifest URL with a bounded HTTPS request. Reject redirects.
5. Call `parse_ton_connect_manifest` and show the returned name, URL, and icon.
6. Call `WalletLifecycle.ton_connect_account` for the selected wallet.
7. If the prompt requests a proof, call
   `WalletLifecycle.sign_ton_connect_proof` with the approved manifest domain,
   current Unix timestamp, and exact challenge.
8. Create `TonConnectDevice` with the current platform, registered wallet name,
   and application version.
9. Call `approve_connect` or `reject_connect`.
10. Persist the session before you send the returned bridge POST.
11. Mark the POST complete only after the bridge accepts it. Then persist the
    session again.

The native device platform is `iphone`, `ipad`, `android`, `windows`, `mac`,
`linux`, or `browser`. The engine supplies the protocol version and advertised
features from its implemented request surface.

After connection, call `begin_events_subscription` and open the returned SSE
URL. Pass each received byte chunk to `ingest_sse_chunk` with the current Unix
time. Persist the session after every successful call, even when it returns no
requests. This stores the latest SSE cursor and replay state.

Handle each returned request by its `kind`:

- For `sendTransaction`, preview `sendRequest`, show approval, and call `send`.
  Pass `SendResult.signedBoc` to `prepare_send_success` only for `submitted`,
  `submissionUnknown`, or `confirmed`.
- If the user rejects a transaction, call `prepare_error` with `UserDeclined`.
- For `disconnect`, call `prepare_disconnect_success` and close the application
  session after its response is durable.
- For `unsupported`, call `prepare_error` with the supplied `errorCode` and
  `errorMessage`.

To restore a native session, call `ton_connect_session_restore` with the stored
value and the same session configuration. Inspect `phase`, then call
`pending_post` before you reopen SSE. If no POST is pending, call
`pending_requests` to recover authenticated requests that still need a
response.

For a wallet-initiated disconnect, call `disconnect` and deliver its prepared
POST with the same durable sequence. Use `reject_connect` when the user rejects
the initial connection.

## Durable bridge responses

Each connect event, RPC response, or disconnect event creates a
`TonConnectPreparedPost`. The session keeps this response in `pending_post`
until the host reports successful delivery.

Use this order for every prepared POST:

1. Call `persisted` and store the returned value in protected durable storage.
2. Send the exact `url` and `body` from `TonConnectPreparedPost`.
3. If delivery is uncertain, keep the pending POST and retry it unchanged.
4. If the bridge accepts the POST, call `complete_pending_post`.
5. Call `persisted` and replace the stored session value.

Do not read more events while a POST is pending. After a restart, call
`pending_post` before you reopen the SSE stream.

## Browser TypeScript

The browser runtime owns manifest loading, encrypted bridge traffic, SSE
reconnection, replay filtering, and pending-response retries.

```ts
import {
  TonConnectWallet,
  type TonConnectStorage,
  type TonConnectWalletEvent,
} from "@ton/wallet-engine"

const tonConnect = new TonConnectWallet({
  descriptor,
  walletClient,
  lifecycle,
  identity: {
    appName: "your-wallet-registry-id",
    appVersion: "1.0.0",
  },
  storage: protectedTonConnectStorage,
})

const unsubscribe = tonConnect.onEvent((event: TonConnectWalletEvent) => {
  if (event.kind === "interaction") {
    showApproval(event.interaction, approved => {
      tonConnect.respond(event.interaction.id, approved)
    })
  }
})

await tonConnect.restore()

async function openTonConnectLink(connectionLink: string): Promise<void> {
  await tonConnect.start(connectionLink)
}
```

`identity.appName` is the wallet registry identifier. It must identify your
wallet. Do not copy the identifier of another wallet.

Implement `TonConnectStorage` with one atomic value per key. The stored JSON
contains the session secret key and possibly an undelivered wallet response.
Do not use plain `localStorage` for this value.

One `TonConnectWallet` instance owns one active dApp session. The current
browser storage key also keeps one resumable session for each wallet record.

The event stream reports approval interactions, successful connection,
transaction completion, disconnect, and transport or protocol errors.

Call `disconnect()` to notify the dApp and delete the stored session. Call
`close()` to stop transport work while keeping the session available for
`restore()`.

The browser runtime uses `https://connect.ton.org/bridge` by default. Set
`bridgeUrl` when the wallet uses another compatible bridge.

## Manifest and proof security

- Fetch manifests only over HTTPS.
- Reject redirects and enforce a response size limit.
- Show the validated dApp name, origin, icon, requested network, and proof
  request before approval.
- Bind `ton_proof` to the exact manifest domain and dApp challenge.
- Use a current Unix timestamp for proof signing.
- Never expose a generic Ed25519 signing method to the dApp.

The proof-signing API constructs the TON Connect digest in Rust. It requests
the protected recovery phrase with the `signTonConnectProof` access reason and
returns only the 64-byte signature.

## Session storage security

Treat the serialized TON Connect session like wallet authentication material.
It contains a bridge session secret key, but it does not contain the wallet
private key or recovery phrase.

Use Keychain, Android Keystore-backed encrypted storage, or an equivalent
protected store. Do not log the serialized session, encrypted POST body,
signed transaction BoC, or proof signature.

## Tests

The end-to-end suite starts the official Go bridge, a TypeScript dApp actor,
and an Acton local network. It covers connection, network checks, transaction
approval and rejection, payloads, contract deployment, sequential sends,
expiration, protocol errors, and restart-safe delivery.

Read [tests/ton-connect/README.md](tests/ton-connect/README.md) for setup and
run commands.
