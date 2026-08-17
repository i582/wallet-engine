# ton-connect-core

Runtime-neutral TON Connect protocol types, session cryptography, replay-safe
state transitions, proof verification, and HTTP bridge framing.

## Try the HTTP bridge demo

The included example implements a narrow real wallet flow:

- parse a TON Connect v2 link;
- fetch and validate the dApp manifest;
- ask for terminal approval;
- return `ton_addr` and, when requested, `ton_proof`;
- receive encrypted requests through bridge SSE;
- acknowledge `disconnect` and end the session.

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

Copy the resulting link into:

```console
cargo run -p ton-connect-core --example http_bridge_wallet -- \
  '<connect-link>' \
  --bridge https://connect.ton.org/bridge \
  --network testnet
```

Use `--network mainnet` only when the connect request explicitly targets
mainnet. The example defaults to testnet and refuses an exact network mismatch.

This is intentionally not a funds-capable wallet. It generates an ephemeral
V5R1 account, keeps both signing and bridge secret keys only in memory, advertises
no transaction methods, and does not persist the session across process restarts.
The `--yes` switch exists for automated experiments and skips terminal approval.

For compatibility with a link generated from Tonkeeper's wallet source, the
example reports `DeviceInfo.appName = "tonkeeper"`. Do not ship that identity:
a distributable wallet must register and report its own wallets-list `app_name`.
