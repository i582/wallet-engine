# Create a wallet from C

The example provides a small interactive menu that can create mainnet or
testnet wallets and list their saved public metadata. Wallet creation prints
the one-shot 24-word recovery phrase.

This deliberately minimal example has no platform protected-storage adapter.
It writes data to two files in the current working directory:

- `wallet_engine_wallets.tsv` contains public wallet metadata;
- `wallet_engine_secrets.tsv` contains plaintext recovery phrases.

The example restricts the secret file to the current user where the platform
supports file permissions.

The secret file is intentionally insecure and exists only to demonstrate host
callback persistence. Never use this storage implementation in production.
Replace it with Keychain, Keystore, Credential Manager, or an equivalent secure
store.

Build and run it from the repository root:

```shell
just example-c-run
```

Use `just example-c-build` when you only need to build it.

The header is generated from the separate `c-bindings` crate:

```shell
cargo xtask bindings c
```
