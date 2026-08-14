# Kotlin and Android bindings

Generate the Kotlin UniFFI source:

```shell
just bindings-kotlin
```

The generated file is written to
`bindings/kotlin/src/main/kotlin/org/ton/wallet/engine/wallet_engine.kt`.
The `bindings/` directory is ignored because the Rust ABI and the pinned
generator are the source of truth.

Build Android native libraries for `arm64-v8a` and `x86_64`:

```shell
just build-android
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
