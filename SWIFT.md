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
