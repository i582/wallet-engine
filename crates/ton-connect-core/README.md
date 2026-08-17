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
