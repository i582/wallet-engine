# Swift wallet example

This example is a small SwiftUI wallet for macOS and iOS. It uses the current
Wallet Engine API directly. It does not depend on the previous wallet runtime.

The application can:

- create a testnet V5R1 wallet;
- store its recovery phrase in Keychain;
- show the balance and recent activity;
- load older transactions;
- send GRAM;
- reveal or delete the selected wallet.

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
the Rust static library for the selected Apple target before it compiles the
application.

For authenticated testnet requests, copy `.env.example` to `.env` in this
directory and set `TONCENTER_TESTNET_API_KEY`. Debug builds copy the value into
the local application bundle. Release builds never include it. Do not use an
embedded service key in a distributed application; inject it from a backend or
use a key owned by the user.

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

## Integration map

- `Infrastructure/AppleWalletHTTPHost.swift` performs bounded HTTPS requests,
  injects host-owned credentials, and rejects redirects.
- `Infrastructure/AppleWalletPlatformHost.swift` stores recovery phrases in
  Keychain and implements the durable send journal.
- `Infrastructure/WalletSession.swift` owns one `WalletClient`, observes newer
  snapshots, and guards client replacement.
- `Infrastructure/WalletLifecycleModel.swift` wraps wallet creation, import,
  recovery-phrase access, and deletion.
- `Model/WalletStore.swift` persists public wallet descriptors. It never stores
  recovery words.
- `Views/ContentView.swift` reads immutable engine snapshots and sends user
  commands back through `WalletSession`.

The `bindings/swift` package is generated and ignored by Git. Regenerate it
after each public Rust API change.
