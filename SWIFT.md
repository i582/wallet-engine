# Swift bindings

Generate the Swift UniFFI source and its C module:

```shell
just bindings-swift
# or: cargo xtask bindings swift
```

The generated files are written to:

- `bindings/swift/Sources/WalletEngineFFI/WalletEngineFFI.swift`
- `bindings/swift/Sources/wallet_engineFFI/wallet_engineFFI.h`
- `bindings/swift/Sources/wallet_engineFFI/module.modulemap`

The `bindings/` directory is ignored because the Rust ABI and the pinned
generator are the source of truth.

Build the Rust library for each Apple platform that the application supports.
For example, build the iOS Simulator library with:

```shell
cargo build --release --locked --target aarch64-apple-ios-sim
```

The application must package the generated Swift and C modules and link the
matching `libwallet_engine.a` for its target. The generated `WalletHttpHost`
and `WalletPlatformHost` protocols use Swift `async` methods. The generator
adds the annotations required by Swift 6 strict concurrency.

## Use a release package

Each tagged release contains `wallet-engine-swift-VERSION.zip`. This archive is
a local Swift package with the generated wrapper and a static `XCFramework`.

Extract the archive. Then add its top-level directory as a local package in
Xcode or `Package.swift`.

The `XCFramework` contains macOS, iOS, and iOS Simulator slices. The minimum
versions are macOS 15 and iOS 18.

## Address utilities

The generated module validates raw and user-friendly addresses, exposes the
friendly flags, and converts between canonical formats:

```swift
let info = try parseTonAddress(value: input)
let valid = isValidTonAddress(value: input)
let raw = try convertTonAddress(value: input, format: .raw)
let display = try convertTonAddress(
    value: raw,
    format: .userFriendly(bounceable: false, testnet: false)
)
```

The `.userFriendly` case inside `info.format` contains the parsed `bounceable`
and `testnet` flags. Raw input has `.raw` because the raw representation does
not carry these flags.

## TON Connect

The generated Swift module includes `TonConnectSession`, manifest parsing,
account reply data, and `ton_proof` signing. The application owns manifest and
bridge transport, approval screens, and protected session storage.

Read [TON_CONNECT.md](TON_CONNECT.md) for the required session and bridge POST
order. The [Swift example](examples/swift/README.md) contains a complete macOS
and iOS integration.

## Clear client secret copies

Rust clears the secret buffers that it owns. Swift `String` values are
immutable, so the application cannot reliably clear `RecoveryPhrase.phrase`.

Keep the phrase only while the recovery screen is visible. Do not write it to
logs, errors, analytics, state restoration, or application storage.

Use `Data` for mutable secret copies in the platform host. Reset every byte in
a `defer` block after the callback finishes.
