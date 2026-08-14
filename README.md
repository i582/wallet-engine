# Wallet Engine

Wallet Engine is a Rust library for TON wallet applications. It gives Swift,
Kotlin, and TypeScript applications the same wallet behavior.

Use Wallet Engine to:

- create and import V5R1 wallets.
- protect recovery phrases with platform storage.
- read balances and transaction history.
- load additional history pages.
- get the GRAM/USD price.
- sign and submit transfers.
- observe immutable wallet snapshots.
- cancel active wallet operations.

The engine supports mainnet and testnet. The host application supplies network
access and platform security services.

## Choose your platform

### Swift

Generate the Swift source and its C module:

```shell
just bindings-swift
```

Then read [SWIFT.md](SWIFT.md) for the generated paths and Apple integration
requirements.

### Kotlin and Android

Generate the Kotlin source:

```shell
just bindings-kotlin
```

Build the Android libraries:

```shell
just build-android
```

Then read [KOTLIN.md](KOTLIN.md) for Android packaging and runtime
dependencies.

### TypeScript and WebAssembly

Generate the browser WebAssembly package:

```shell
just bindings-wasm
```

The repository also contains the source of the high-level
`@ton/wallet-engine` TypeScript package. It provides a browser HTTP host,
IndexedDB journal storage, wallet lifecycle methods, and wallet client methods.

Read [WASM.md](WASM.md) for browser setup and security requirements.

This repository does not track generated bindings. Generate them from the same
revision that you use to build the Rust library.

## Integration model

Wallet Engine owns the wallet state machine. Your application implements two
asynchronous host interfaces.

| Interface | Your application provides |
| --- | --- |
| `WalletHttpHost` | Bounded HTTP requests and cancellation by call ID |
| `WalletPlatformHost` | Time, protected secrets, and durable journal storage |

The host callbacks let the same Rust engine run on Apple, Android, and browser
platforms. The engine does not own platform networking or protected storage.

## Basic wallet flow

1. Implement `WalletHttpHost` and `WalletPlatformHost` for your platform.
2. Create `WalletLifecycle` with your platform host.
3. Create or import a wallet.
4. Store the returned wallet descriptor in your application storage.
5. Show the recovery phrase only in the required user flow.
6. Create `WalletClient` with the descriptor data and both host objects.
7. Call `refresh()` to load the initial wallet state.
8. Publish the returned snapshot in your user interface.
9. Call `waitForChange` to receive later snapshot revisions.
10. Call `shutdown()` before you discard the client.

The wallet descriptor contains the stable wallet ID, address, network, and
protected-secret reference. Your application needs these values after a
restart.

## Wallet state

`WalletSnapshot` is the only state that the user interface needs. It contains:

- the account balance and status.
- recent activity and its pagination cursor.
- independent states for account, activity, pagination, and price resources.
- the latest send state.
- a monotonic revision number.

A refresh can complete partially. Read each resource state before you replace
previous data or show an error.

Use `waitForChange(afterRevision:)` on Swift. Use
`waitForChange(afterRevision)` on Kotlin. Both calls wait until a newer snapshot
is available.

## Sending GRAM

Call `send` with a unique operation ID, destination address, amount in
nanograms, and the wallet secret reference.

Before submission, the engine:

1. loads a fresh account state and sequence number.
2. requests the protected recovery phrase from the host.
3. makes sure that the phrase belongs to the selected wallet.
4. signs the V5R1 external message in Rust.
5. stores the exact signed BoC in the host journal.
6. submits that BoC to the provider.

Handle every send phase explicitly:

| Phase | Meaning |
| --- | --- |
| `submitted` | The provider accepted the signed BoC. This is not an on-chain confirmation. |
| `failed` | The provider rejected the request before an ambiguous result occurred. |
| `submissionUnknown` | The provider can have received the BoC, but the engine cannot prove the result. |
| `cancelled` | The operation stopped before the durable submission boundary. |

CAUTION: If the phase is `submissionUnknown`, do not create and submit a new
transfer automatically. The first signed message can already be in the
network.

The journal uses compare-and-swap writes. The host implementation must make
these writes durable before it reports success.

## Streaming

Wallet Engine does not contain a streaming API. The host application owns its
stream connection, reconnect policy, and application lifecycle.

The engine also does not reconcile stream events. A host can start a normal
refresh when its product behavior requires new wallet data.

## Security requirements

- Store recovery phrases with the platform protected-storage API.
- Require user authentication according to your product policy.
- Do not log recovery phrases, secret values, or signed BoCs.
- Bind provider credentials to an exact HTTPS origin.
- Enforce the request and response limits from each `HttpCall`.
- Honor `cancelHttp` for calls that have not completed.
- Store wallet descriptors separately from protected recovery phrases.

The engine clears its temporary recovery-phrase buffer. The host language and
the FFI boundary can create additional memory copies. The host must limit the
lifetime of these copies.

## Build from source

The repository pins Rust 1.96.1 in `rust-toolchain.toml`.

Build the release library:

```shell
just build
```

Run all repository checks:

```shell
just check
```

Install the optional repository tools before the first complete check:

```shell
just install-tools
```

Run `cargo xtask --help` to see the binding and Android build commands.

## License

Wallet Engine is available under the Apache License 2.0 or the MIT License.
See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT).
