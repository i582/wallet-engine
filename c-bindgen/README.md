# Wallet Engine C bindgen

Custom UniFFI backend that will generate the typed Wallet Engine C facade from
the `ComponentInterface` embedded in the compiled Rust library.

The current scaffold already:

- accepts a compiled library and output directory;
- extracts and validates UniFFI metadata with `BindgenLoader`;
- derives the private UniFFI FFI functions;
- writes a deterministic `wallet_engine.c-api.json` inventory.

Header, facade, codecs, callback adapters, and export lists will be added on top
of the normalized component model. The crate intentionally has no dependency on
the `wallet-engine` or `c-bindings` crates.

```shell
cargo run --locked --manifest-path c-bindgen/Cargo.toml -- \
  --library target/bindings-host/debug/libwallet_engine.dylib \
  --out-dir /tmp/wallet-engine-c
```

From the repository root, the experimental recipe builds the library and runs
the generator without touching the production `bindings/c` output:

```shell
just bindings-c-experimental
```

The output directory is always `bindings/c-experimental`.
