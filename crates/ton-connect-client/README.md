# ton-connect-client

Runtime-neutral wallet-side TON Connect session orchestration.

The crate owns protocol state that must remain consistent across HTTP bridge
connections:

- session key generation and authenticated encryption;
- connect and disconnect lifecycle transitions;
- replay-safe dApp request identifiers;
- SSE framing and resume cursors;
- preparation of encrypted bridge responses;
- validated persistence snapshots.

The host owns HTTP streaming, retries, durable storage, wallet operations, and
user approval. This keeps the crate independent from Tokio, reqwest, and a
specific wallet implementation.
