# ton-connect-client

Runtime-neutral wallet-side TON Connect session orchestration.

The crate owns protocol state that must remain consistent across HTTP bridge
connections:

- session key generation and authenticated encryption.
- connect and disconnect lifecycle transitions.
- replay-safe dApp request identifiers.
- SSE framing and resume cursors.
- preparation of encrypted bridge responses.
- validated persistence snapshots.

The host owns HTTP streaming, retries, durable storage, wallet operations, and
user approval. This keeps the crate independent from Tokio, reqwest, and a
specific wallet implementation.

## Delivery contract

Persist the client after each accepted SSE chunk and before you process any
returned request. This stores the replay reducer and the latest resume cursor.

Persist the client after it prepares a wallet event or RPC response. Then send
the exact `PreparedBridgePost`. If delivery is uncertain, retry that post
without creating a new response.

`PersistedTonConnectClient` contains the HTTP session secret key. Store it with
the same protection as wallet authentication credentials.

The root `wallet-engine` crate adds an FFI-safe session wrapper, wallet account
mapping, proof signing, and raw transaction mapping. Read
[TON_CONNECT.md](../../TON_CONNECT.md) for the complete host flow and supported
request surface.
