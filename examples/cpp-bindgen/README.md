# Create a wallet with generated C++ bindings

This example mirrors `examples/c`, but uses the experimental C++ API generated
by `uniffi-bindgen-cpp`. It provides a small interactive menu that can create
mainnet or testnet wallets and list their saved public metadata.

The example deliberately uses an insecure file-backed host:

- `wallet_engine_wallets.tsv` contains public wallet metadata;
- `wallet_engine_secrets.tsv` contains plaintext recovery phrases.

The secret file exists only to demonstrate a C++ implementation of
`WalletPlatformHost`. Never use this storage implementation in production.
Replace it with Keychain, Keystore, Credential Manager, or an equivalent secure
store.

The generated API currently exposes Rust async functions as blocking C++
methods. The example therefore calls `WalletLifecycle::create_wallet` directly;
the host callback is completed synchronously by the generated bridge.

Build and run it from the repository root:

```shell
just example-cpp-bindgen-run
```

Use `just example-cpp-bindgen-build` when you only need to build it. Both
commands regenerate `bindings/cpp-experimental` first.
