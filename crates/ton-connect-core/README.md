# ton-connect-core

Runtime-neutral TON Connect protocol types, session cryptography, replay-safe
state transitions, proof verification, and HTTP bridge framing.

## Protocol coverage

The crate follows the pinned [TON Connect specification revision
`5656a962eee30819a31a9e918e3de0b9614713b6`](https://github.com/ton-blockchain/ton-connect/commit/5656a962eee30819a31a9e918e3de0b9614713b6)
from May 18, 2026.

| Area | Core support |
| --- | --- |
| Session | X25519 client IDs, NaCl box encryption, nonce generation, and persisted key validation |
| Connect | Requests, item replies, events, capabilities, `ton_proof`, and fixed-account validation |
| RPC | `sendTransaction`, `signMessage`, `signData`, `disconnect`, typed results, and error codes |
| Structured items | TON, jetton, NFT, extra currencies, and runtime capability validation |
| Embedded requests | Compact wire decoding, encoding, capability validation, and responses without an RPC ID |
| Links | Universal, `tc://`, custom-scheme, reduced, return, trace, and embedded request parameters |
| HTTP bridge | Endpoint construction, both heartbeat modes, SSE framing, encryption, replay cursor, and trace IDs |
| JS bridge | Runtime-neutral interface contract, wallet metadata, connection restore, send, and event subscription |
| Metadata | App manifests and `wallets-v2.json` entries with semantic validation |
| Account proof | Standard wallet state parsing, address binding, public-key extraction, and signature verification |

The host fetches manifests and wallet lists. The host also owns HTTP I/O,
cache policy, user prompts, persistence, wallet signing, and transaction submission.
These operations require platform services and are outside the protocol core.

## Try the HTTP bridge demo

The [Ratatui wallet example](https://github.com/i582/wallet-engine/tree/master/examples/tui)
is a workspace-only example and is not included in this published crate. Run
it from a checkout of `wallet-engine`:

It demonstrates a narrow real-wallet HTTP bridge flow:

- parse a TON Connect v2 link;
- fetch and validate the dApp manifest;
- ask for terminal approval;
- return `ton_addr` and, when requested, `ton_proof`;
- receive encrypted requests through bridge SSE;
- approve and submit one raw native `sendTransaction`, including a contract-call payload;
- acknowledge `disconnect` and end the session.

First create or import the wallet that the example will expose:

```console
cargo run --manifest-path examples/tui/Cargo.toml
```

Generate a link while a dApp connector remains open:

```ts
import { TonConnect } from '@tonconnect/sdk';

const connector = new TonConnect({
  manifestUrl: 'https://your-app.example/tonconnect-manifest.json',
});

const link = connector.connect({
  universalLink: 'tc://',
  bridgeUrl: 'https://connect.ton.org/bridge',
});

console.log(link);
```

Start the TUI, open the TON Connect dialog with `t`, and paste the resulting
link:

```console
cargo run --manifest-path examples/tui/Cargo.toml
```

The network and account come from the stored wallet. The example refuses an
exact network or `from` mismatch instead of silently switching accounts.

The example loads the wallet created or imported by the terminal example from
its protected local store. TON Connect proof signing stays inside `wallet-engine`;
the mnemonic and private key do not cross the API boundary. The `--yes` switch
exists for automated connection experiments. Transaction approval is always
interactive.

For compatibility with a link generated from Tonkeeper's wallet source, the
example reports `DeviceInfo.appName = "tonkeeper"`. Do not ship that identity:
a distributable wallet must register and report its own wallets-list `app_name`.
