# Swift wallet example

This example is a small SwiftUI wallet for macOS and iOS. It uses the current
Wallet Engine API directly. It does not depend on the previous wallet runtime.

The application can:

- create a testnet wallet.
- store its recovery phrase in Keychain.
- show the balance and recent activity.
- load older transactions.
- send GRAM.
- connect dApps through TON Connect.
- sign `ton_proof` ownership challenges.
- preview and approve TON Connect transaction batches before signing.
- sign gasless TON Connect messages for submission by a dApp relayer.
- restore and disconnect encrypted TON Connect sessions.
- reveal or delete the selected wallet.

The approval screen lists each message amount, recipient, body type, and
`StateInit` presence.

The example does not use a streaming connection or a fiat-rate provider.

## Open in Xcode

Generate the local Swift package first:

```shell
just bindings-swift
```

Then open the project:

```shell
just example-swift-open
```

Choose `WalletEngineApp` and run it on macOS or an iOS Simulator. Xcode builds
the Rust static library in the debug profile for the selected Apple target
before it compiles the application. The example intentionally uses debug Rust
and Swift builds while TON Connect integration is under active development.

For authenticated Toncenter requests, copy `.env.example` to `.env` in this
directory and set `TONCENTER_API_KEY`. The same key is available to mainnet and
testnet requests. Debug builds copy the value into the local application
bundle. Release builds never include it. Do not use an embedded service key in
a distributed application. Inject it from a backend or use a key owned by the
user.

## Build from the command line

Build the macOS application:

```shell
just example-swift-build-macos
```

Build the arm64 iOS Simulator application:

```shell
just example-swift-build-ios
```

The iOS Simulator build uses Xcode's local ad-hoc signature. The simulated
application identifier is required for Keychain access. The macOS command-line
recipe remains a compile-only unsigned build. To run the application on a
physical device, select your development team in Xcode and use the normal Run
action.

## iOS client E2E tests

The iOS suite runs the shared client scenarios in an iPhone 16 Pro Simulator.
It starts the official Go bridge and the official TypeScript TON Connect dApp
SDK. Most scenarios use deterministic provider responses. The transaction-history
scenario starts an isolated Acton localnet. The test target drives the real
SwiftUI application through accessibility identifiers.

Install the dApp dependencies. Then run the suite:

```shell
just example-swift-e2e-install
TON_CONNECT_BRIDGE_BIN=/absolute/path/to/bridge3 just example-swift-e2e
```

Install Acton before you run the localnet scenario. Set
`WALLET_ENGINE_ACTON_BIN` if the Acton binary is not at
`$HOME/.acton/bin/acton` or on `PATH`.

The runner generates Swift bindings and shared scenario JSON files. It also
builds the TypeScript dApp actor before it starts `xcodebuild`.

Set `IOS_E2E_DESTINATION` to use a different simulator destination. The value
must include an explicit simulator `id`. Set `IOS_SNAPSHOT_VARIANT` when that
destination requires a separate baseline.

Set `IOS_E2E_DERIVED_DATA_PATH` to use a separate Xcode build directory.

Set `IOS_E2E_ONLY_TESTING` to an Xcode test identifier to run one scenario.
For example:

```shell
IOS_E2E_ONLY_TESTING=WalletEngineAppUITests/WalletEngineAppUITests/testRejectedTonConnectScenario \
  TON_CONNECT_BRIDGE_BIN=/absolute/path/to/bridge3 just example-swift-e2e
```

The suite records screenshots in dark appearance. The runner fixes the
simulator status bar during each run so that system time and indicators do not
change the baselines. To replace the baselines, run:

```shell
TON_CONNECT_BRIDGE_BIN=/absolute/path/to/bridge3 just example-swift-e2e-update-snapshots
```

Review every changed image before commit. Git stores the PNG files in Git LFS.
The recovery screenshot masks the wallet address and recovery words before it
writes the image.

## Integration map

- `Infrastructure/AppleWalletHTTPHost.swift` performs bounded HTTPS requests,
  injects the host-owned Toncenter key, and rejects redirects.
- `Infrastructure/AppleWalletPlatformHost.swift` stores recovery phrases in
  Keychain and implements the durable send journal.
- `Infrastructure/WalletSession.swift` owns one `WalletClient`, observes newer
  snapshots, and guards client replacement.
- `Infrastructure/TonConnectCoordinator.swift` binds the Rust TON Connect
  reducer to manifest loading, request preview, user approval, send, and
  sign-only handoff.
- `Infrastructure/TonConnectTransport.swift` performs redirect-free manifest,
  bridge POST, and streaming SSE requests.
- `Infrastructure/TonConnectSessionStore.swift` keeps secret-bearing session
  keys and durable pending responses in Keychain.
- `WalletEngineAppUITests` interprets the shared scenarios and compares dark
  UI screenshots on an iOS Simulator.
- `Infrastructure/WalletLifecycleModel.swift` wraps wallet creation, import,
  recovery-phrase access, and deletion.
- `Model/WalletStore.swift` persists public wallet descriptors. It never stores
  recovery words.
- `Views/ContentView.swift` reads immutable engine snapshots and sends user
  commands back through `WalletSession`.

The `bindings/swift` package is generated and ignored by Git. Regenerate it
after each public Rust API change.
