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
implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.7.3")
```

The generated `WalletHttpHost` and `WalletPlatformHost` callbacks and the
exported engine methods use Kotlin `suspend` functions. No handwritten JNI
adapter is required.

## Clear client secret copies

Rust clears the secret buffers that it owns. Kotlin `String` values are
immutable, so the application cannot reliably clear `RecoveryPhrase.phrase`.

Keep the phrase only while the recovery screen is visible. Do not write it to
logs, errors, analytics, saved state, or application storage.

Use `ByteArray` for mutable secret copies in the platform host. Call `fill(0)`
in a `finally` block after the callback finishes.
