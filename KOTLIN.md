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

## TON transfer links

Parse a standard transfer link before showing its confirmation screen:

```kotlin
val invoice = parseTonTransferLink(
    "ton://transfer/$recipient?amount=1000000000&text=hello%20TON",
)
```

The result preserves the recipient, Gram or jetton asset, optional exact
elementary-unit amount, text or BOC payload, and expiration policy. This
function only parses the strict `ton://transfer/` baseline: `amount`, `text`,
`exp`, `jetton`, and `bin`. It does not resolve chain state, check expiration,
select bounce behavior, or authorize a send.

## Mnemonic word list

Use the engine's BIP-39 word list for recovery-phrase input:

```kotlin
val words = mnemonicWordlist()
val suggestions = words.filter { it.startsWith(input.lowercase()) }
```

The list contains the 2048 English BIP-39 words accepted by the same
recovery-phrase validation used for wallet import. Its order is the BIP-39
index order.

## Prepare key rotation

Create the second mnemonic half and the signed Wallet rev00 request:

```kotlin
val prepared = lifecycle.prepareKeyRotation(
    PrepareKeyRotationRequest(
        descriptor = descriptor,
        seqno = freshSeqno,
        validUntil = validUntil,
        messageKind = KeyRotationMessageKind.EXTERNAL,
    ),
)
```

This call requests the protected secret with
`SecretAccessReason.PREPARE_KEY_ROTATION`. It returns a complete 24-word phrase,
the new public key, and a signed BOC. It does not change protected storage or
submit the BOC.

Before submission, store the phrase in protected storage. Store the pending
BOC in a durable journal. Until the application resolves the on-chain result,
block ordinary signing.

## Enriched activity

Every nonzero transfer returned in `WalletSnapshot.activity.items` includes
`transactionFeeNanograms`, `status`, and an optional decoded plaintext
`comment`. The fee is the total fee for the transaction. It is repeated when
one transaction produces multiple activity rows, so do not sum it per row.

## Encrypted comments

Call `WalletClient.createEncryptedComment` with the recipient and UTF-8 text.
It loads the recipient's `get_public_key`, requests the protected mnemonic with
`SecretAccessReason.ENCRYPT_COMMENT`, and returns a BOC. Use that BOC as a
`SendMessageBody.RawPayload`, then preview and send the same intent.

Encrypted activity has `encryptedComment` instead of `comment`. Decrypt it
explicitly with `WalletClient.decryptComment`, passing the sender address. For
received activity the sender is `counterparty`; for sent activity it is the
wallet address. This read uses `SecretAccessReason.DECRYPT_COMMENT`, so refresh
does not trigger device authentication. Plaintext is limited to 960 UTF-8
bytes.

## TON DNS

Resolve the standard `wallet` record for a `.ton` name before you build a send
intent:

```kotlin
val address: String? = client.resolveDns("foundation.ton")
```

The engine normalizes the name to lowercase and follows TON DNS delegation
through the provider. `ProviderConfig.dnsRootAddress` can override the root;
`null` uses the built-in current root for the wallet network. The result is
`null` when the name has no wallet record. Invalid names and malformed provider
data throw `DnsResolutionUnavailable`. This read-only call does not request the
protected mnemonic.

## NFT collection metadata

An NFT that belongs to a collection includes a neutral `collection` descriptor
with its address, standard TEP-64 fields, and complete string metadata. Keep
product-specific classification, such as Telegram gifts, in the application.

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
