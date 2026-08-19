# Web client E2E tests

The suite drives the real React application and WASM wallet engine in Chromium. TON Connect
scenarios use the official TypeScript dApp SDK and the official Go bridge. Scripted scenarios use
deterministic provider responses. The transaction-history scenario uses an isolated Acton localnet.

Build the Web application and dApp actor, then run all functional and visual scenarios:

```sh
bun run e2e:install
bun run e2e
```

The install command also installs the official dApp actor's pinned npm dependencies. CI uses
`bun run e2e:install:ci` to install Chromium's system packages as well.

Install Acton before you run `localnet-activity`. Set `WALLET_ENGINE_ACTON_BIN` if the Acton binary
is not at `$HOME/.acton/bin/acton` or on `PATH`.

The localnet scenario funds the new wallet with 10 GRAM. It submits two TON Connect transfers and
checks history only after explicit refresh actions. It also checks ordering, duplicate removal,
and restoration after a browser reload.

Update committed screenshots only after reviewing the intended UI changes:

```sh
bun run e2e:update-snapshots
```

Visual baselines are separated by operating system. This prevents browser text rendering on one
platform from silently replacing another platform's reference images. Playwright records all
baselines with the dark color scheme. Git stores every PNG file in Git LFS.

Run the same scenarios without pixel comparisons in a functional CI job:

```sh
bun run e2e:functional
```

Set `TON_CONNECT_BRIDGE_BIN` when the official `bridge3` executable is not available at
`/tmp/ton-connect-research/bridge/bridge3`.

The TON Connect fixture injects its isolated bridge URL through
`globalThis.walletEngineConfig`. A deployed host can use the same runtime configuration, or set
`VITE_TON_CONNECT_BRIDGE_URL` when building the example.

Scenario definitions contain serializable steps. The export command writes the platform-neutral
JSON files to `examples/client-e2e/scenarios`. Browser selectors and process control stay in the
Web runner and fixture adapters. The iOS runner interprets the same exported steps.
