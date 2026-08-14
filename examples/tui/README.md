# Terminal wallet example

This example is a small testnet wallet built with Rust and
[Ratatui](https://ratatui.rs). It demonstrates the native Rust API without
Swift, Kotlin, TypeScript, or generated bindings.

The application can:

- create a V5R1 testnet wallet;
- import 24 recovery words;
- restore the wallet after a restart;
- refresh the balance and activity;
- load older activity;
- sign and submit a testnet transfer;
- delete the local wallet.

## Run

From the repository root:

```shell
just example-tui-run
```

Toncenter works without a key at a limited request rate. To use your own key:

```shell
TONCENTER_API_KEY=your-key just example-tui-run
```

Set `WALLET_ENGINE_TUI_HOME` to change the data directory. By default, the
example stores its data in `~/.wallet-engine-tui/wallet.json`.

## Controls

The available keys are shown on each screen. The dashboard uses:

- `c` to copy the wallet address;
- `s` to send;
- `r` to refresh;
- `l` to load older activity;
- `d` to delete the wallet;
- `q` to quit.

Press `Ctrl+C` on any screen to quit.

## Security

This is a testnet integration example. Its `WalletPlatformHost` stores recovery
words in a local file with owner-only permissions on Unix. It does not use an
OS keychain or request device authentication. Do not import a mainnet recovery
phrase and do not reuse the generated phrase for a mainnet wallet.

A production application must replace `DiskStore` with protected platform
storage. The journal implementation must also retain its atomic and durable
compare-and-swap behavior.
