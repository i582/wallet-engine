# Android wallet example

This example is a Jetpack Compose wallet for Android. It uses the current
Wallet Engine Kotlin API directly. It does not use the old JNI adapter.

The application can:

- create and import a testnet wallet.
- store its recovery phrase with Android Keystore encryption.
- show the balance and recent activity.
- load older transactions.
- send GRAM.
- rename or delete a wallet.

The example uses periodic refresh. It does not use a streaming connection or
a fiat-rate provider.

## Build the application

Install the Android SDK and Android NDK first. Then run this command from the
repository root:

```shell
just example-android-build
```

The command generates the Kotlin source. It also builds the arm64 and x86_64
Rust libraries before Gradle builds the APK.

The APK is at `examples/android/app/build/outputs/apk/debug/app-debug.apk`.

## Add a Toncenter key

Copy the local environment template:

```shell
cp examples/android/.env.example examples/android/.env
```

Set `TONCENTER_TESTNET_API_KEY` in `examples/android/.env`. A debug build puts
this value in `BuildConfig` and injects it only for the testnet Toncenter host.

CAUTION: Do not distribute an application with a service key in `BuildConfig`.
Use a user key or a backend for a distributed application.

## Install the application

Start an Android emulator or connect a device. Then run:

```shell
just example-android-install
```

## Integration map

- `AndroidWalletHttpHost.kt` performs bounded HTTPS requests and rejects redirects.
- `SecureWalletStore.kt` stores wallet secrets and the send journal.
- `WalletRepository.kt` owns `WalletLifecycle` and the active `WalletClient`.
- `WalletViewModel.kt` maps immutable engine snapshots to UI state.
- `TonWalletApp.kt` contains the existing Compose wallet interface.

The host encrypts recovery phrases with an Android Keystore key. This example
does not show `BiometricPrompt` before each secret read. Add that flow before
you use this storage policy in a production wallet.
