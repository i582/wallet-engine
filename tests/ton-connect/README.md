# TON Connect end-to-end tests

These tests are regular integration tests. They are not ignored and require the
official Go bridge plus the compiled TypeScript dApp actor.

## Requirements

- The repository's Rust toolchain.
- Node.js and npm.
- Go 1.26 or newer to build the current official bridge.
- Permission to bind local loopback ports.

The test uses only local processes. It does not connect to TON mainnet or
testnet. The dApp actor serves its manifest and icon from a local HTTPS endpoint
with the test-only certificate in `dapp/fixtures`. The Rust harness accepts that
self-signed certificate only for these integration tests.

## One-time setup

Build the official bridge at the default location:

```bash
git clone https://github.com/ton-connect/bridge /tmp/ton-connect-research/bridge
cd /tmp/ton-connect-research/bridge
make build3
```

Install and compile the TypeScript actor:

```bash
cd tests/ton-connect/dapp
npm ci
npm run build
```

## Run

From the wallet-engine repository root:

```bash
cargo test --test ton_connect_e2e
```

The default bridge binary is
`/tmp/ton-connect-research/bridge/bridge3`. Override it when necessary:

```bash
TON_CONNECT_BRIDGE_BIN=/absolute/path/to/bridge3 \
  cargo test --test ton_connect_e2e
```

Set `NODE` to override the Node.js executable:

```bash
NODE=/absolute/path/to/node cargo test --test ton_connect_e2e
```

The harness starts bridge and dApp processes on available ports, waits for
readiness, and always terminates child processes when a scenario finishes.
The Rust CI job builds the pinned official bridge revision and TypeScript dApp
before it runs the scenarios. It sets `TON_CONNECT_BRIDGE_BIN` explicitly.

## Coverage

The suite covers these protocol and chain results:

- connect approval with matching, absent, and mismatched network requests.
- plain sends, ordered batches, payloads, and destination `StateInit`.
- local-network contract deployment and a later second transaction.
- transaction rejection without an on-chain deployment.
- sign-only batches that do not broadcast from the wallet.
- gasless account deployment through a deterministic local relayer.
- preservation of the signed body and wallet `StateInit` during relaying.
- expired requests and requests for another sender or network.
- message-count and extra-currency capability limits.
- unknown or revoked dApp sessions.

## Scenario dApp configuration

Keep the complete dApp configuration in one scenario constant. Use
`{actor_origin}` where the harness must substitute the local HTTPS origin:

```rust
const TESTNET_NETWORK: &str = "-3";

const TEST_DAPP_CONFIG: DappConfig = DappConfig::new(
    "{actor_origin}/tonconnect-manifest.json",
    DappManifestConfig::new(
        "{actor_origin}",
        "Wallet Engine TON Connect Test dApp",
        "{actor_origin}/icon.png",
    ),
)
.universal_link("tc://")
.in_network(TESTNET_NETWORK);
```
