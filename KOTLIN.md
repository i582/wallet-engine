# Kotlin and Android bindings

Generate the Kotlin UniFFI source:

```shell
just bindings-kotlin
# or: cargo xtask bindings kotlin
```

The generated file is written to
`bindings/kotlin/src/main/kotlin/org/ton/wallet/engine/wallet_engine.kt`.
The `bindings/` directory is ignored because the Rust ABI and the pinned
generator are the source of truth.

Build Android native libraries for `arm64-v8a` and `x86_64`:

```shell
just build-android
# or: cargo xtask android --abi all
```

The libraries are written to `target/android/jniLibs/<abi>/libwallet_engine.so`.
Both generated bindings and native libraries must be packaged by the Android
application.

The Android module needs these runtime dependencies:

```kotlin
implementation("androidx.annotation:annotation:1.9.1")
implementation("net.java.dev.jna:jna:5.12.0@aar")
implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
```

The generated `WalletHttpHost` and `WalletPlatformHost` callbacks and the
exported engine methods use Kotlin `suspend` functions. No handwritten JNI
adapter is required.

## Use a release package

Each tagged release contains `wallet-engine-android-VERSION.aar` and matching
Maven metadata. The AAR contains the Kotlin wrapper and native libraries for
`arm64-v8a` and `x86_64`.

Add the AAR to the application as a local dependency. Add the runtime
dependencies from this document to the application module.

The release AAR requires Android API level 28 or newer.

## Address utilities

The generated module validates raw and user-friendly addresses, exposes the
friendly flags, and converts between canonical formats:

```kotlin
val info = parseTonAddress(input)
val valid = isValidTonAddress(input)
val raw = convertTonAddress(input, TonAddressFormat.Raw)
val display = convertTonAddress(
    raw,
    TonAddressFormat.UserFriendly(bounceable = false, testnet = false),
)
```

`TonAddressFormat.UserFriendly` inside `info.format` contains the parsed
`bounceable` and `testnet` flags. Raw input has `TonAddressFormat.Raw` because
the raw representation does not carry these flags.

## TON Connect

The generated Kotlin module includes `TonConnectSession`, manifest parsing,
account reply data, and `ton_proof` signing. The application owns manifest and
bridge transport, approval screens, and protected session storage.

Read [TON_CONNECT.md](TON_CONNECT.md) for the required session and bridge POST
order. The current Android example does not implement the TON Connect user
interface.

## Clear client secret copies

Rust clears the secret buffers that it owns. Kotlin `String` values are
immutable, so the application cannot reliably clear `RecoveryPhrase.phrase`.

Keep the phrase only while the recovery screen is visible. Do not write it to
logs, errors, analytics, saved state, or application storage.

Use `ByteArray` for mutable secret copies in the platform host. Call `fill(0)`
in a `finally` block after the callback finishes.
