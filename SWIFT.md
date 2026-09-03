# Swift bindings

Generate the Swift UniFFI source and its C module:

```shell
just bindings-swift
# or: cargo xtask bindings swift
```

The generated files are written to:

- `bindings/swift/Sources/WalletEngineFFI/WalletEngineFFI.swift`
- `bindings/swift/Sources/wallet_engineFFI/wallet_engineFFI.h`
- `bindings/swift/Sources/wallet_engineFFI/module.modulemap`

The `bindings/` directory is ignored because the Rust ABI and the pinned
generator are the source of truth.

Build the Rust library for each Apple platform that the application supports.
For example, build the iOS Simulator library with:

```shell
cargo build --release --locked --target aarch64-apple-ios-sim
```

The application must package the generated Swift and C modules and link the
matching `libwallet_engine.a` for its target. The generated `WalletHttpHost`,
`WalletStatuslessHost`, and `WalletPlatformHost` protocols use Swift `async`
methods. The generator adds the annotations required by Swift 6 strict
concurrency. Use `WalletClient.newStatusless` for a relay or protocol proxy
that returns only a body or an opaque transport error.

## Use a release package

Each tagged release contains `wallet-engine-swift-VERSION.zip`. This archive is
a local Swift package with the generated wrapper and a static `XCFramework`.

Extract the archive. Then add its top-level directory as a local package in
Xcode or `Package.swift`.

The `XCFramework` contains macOS, iOS, and iOS Simulator slices. The minimum
versions are macOS 15 and iOS 18.

## Address utilities

The generated module validates raw and user-friendly addresses, exposes the
friendly flags, and converts between canonical formats:

```swift
let info = try parseTonAddress(value: input)
let valid = isValidTonAddress(value: input)
let raw = try convertTonAddress(value: input, format: .raw)
let display = try convertTonAddress(
    value: raw,
    format: .userFriendly(bounceable: false, testnet: false)
)
```

The `.userFriendly` case inside `info.format` contains the parsed `bounceable`
and `testnet` flags. Raw input has `.raw` because the raw representation does
not carry these flags.

## TON transfer links

Parse a standard transfer link before showing its confirmation screen:

```swift
let invoice = try parseTonTransferLink(
    value: "ton://transfer/\(recipient)?amount=1000000000&text=hello%20TON"
)
```

The result preserves the recipient, Gram or jetton asset, optional exact
elementary-unit amount, text or BOC payload, and expiration policy. This
function only parses the strict `ton://transfer/` baseline: `amount`, `text`,
`exp`, `jetton`, and `bin`. It does not resolve chain state, check expiration,
select bounce behavior, or authorize a send.

## Mnemonic word list

Use the engine's BIP-39 word list for recovery-phrase input:

```swift
let words = mnemonicWordlist()
let suggestions = words.filter { $0.hasPrefix(input.lowercased()) }
```

The list contains the 2048 English BIP-39 words accepted by the same
recovery-phrase validation used for wallet import. Its order is the BIP-39
index order.

## Mnemonic scheme detection

Classify entered recovery words to explain a failed import:

```swift
let schemes = detectMnemonicSchemes(words: enteredWords)
if schemes.contains(.rotation) {
    // importWallet accepts this phrase.
} else if !schemes.isEmpty {
    // A TON (.ton) or 24-word BIP-39 (.bip39) phrase from another
    // wallet scheme. Explain that in your own product wording.
} else {
    // Not a known mnemonic.
}
```

Pass the words exactly as you would pass them to `importWallet`. Only
`.rotation` phrases can be imported; `.ton` and `.bip39` are detection only
and the engine derives no keys from them.

## Prepare key rotation

Create a new signing half and the signed Wallet rev00 request:

```swift
let prepared = try await client.prepareKeyRotation(
    request: PrepareKeyRotationRequest(
        validUntil: validUntil,
        messageKind: .external
    )
)

let request = SendBocRequest(
    operationId: UUID().uuidString.lowercased(),
    force: false,
    signedBoc: prepared.signedBoc,
    seqno: prepared.seqno,
    validUntil: prepared.validUntil
)
let preview = try await client.previewSendBoc(request: request)

// Persist the replacement phrase in the application's protected storage first.
let result = try await client.sendBoc(request: request)
```

The client first fetches fresh account state through its configured HTTP host.
It calls the `seqno` getter only for an active contract; an undeployed account
uses `seqno` zero and gets anchor-based `StateInit` in the prepared BOC.
It then requests the protected secret with
`SecretAccessReason.prepareKeyRotation`. It returns a complete 24-word phrase,
the new public key, a signed BOC, and the `seqno` covered by the signature. It
does not change protected storage or submit the BOC by itself.

For a later rotation, protected storage must contain the 24-word phrase from
the last successful rotation.

`previewSendBoc` validates fresh `seqno`, destination, and expiration, then
emulates the exact signed BOC without journaling or submitting it. Its returned
message list is empty because it does not decode the opaque BOC into a
`SendIntent`.

Before `sendBoc`, store the phrase in protected storage. `sendBoc` validates
fresh `seqno`, destination, and expiration, then stores the exact BOC in the
same durable wallet-wide journal used by `send` before provider handoff. It
returns `SendResult`; `submissionUnknown`, `resolvePending`, `cancelSend`, and
the ordinary-send block have the same semantics as transfer submission.

## Enriched activity

Every nonzero transfer returned in `WalletSnapshot.activity.items` includes
`transactionFeeNanograms`, `status`, and an optional decoded plaintext
`comment`. The fee is the total fee for the transaction. It is repeated when
one transaction produces multiple activity rows, so do not sum it per row.

## Encrypted comments

Call `WalletClient.createEncryptedComment` with the recipient and UTF-8 text.
It loads the recipient's `get_public_key`, requests the protected mnemonic with
`SecretAccessReason.encryptComment`, and returns a BOC. Use that BOC as a
`SendMessageBody.rawPayload`, then preview and send the same intent.

Encrypted activity has `encryptedComment` instead of `comment`. Decrypt it
explicitly with `WalletClient.decryptComment`, passing the sender address. For
received activity the sender is `counterparty`; for sent activity it is the
wallet address. This read uses `SecretAccessReason.decryptComment`, so refresh
does not trigger device authentication. Plaintext is limited to 960 UTF-8
bytes.

## TON DNS

Resolve the standard `wallet` record for a `.ton` name before you build a send
intent:

```swift
let address: String? = try await client.resolveDns(name: "foundation.ton")
```

The engine normalizes the name to lowercase and follows TON DNS delegation
through the provider. `ProviderConfig.dnsRootAddress` can override the root;
`nil` uses the built-in current root for the wallet network. The result is `nil`
when the name has no wallet record. Invalid names and malformed provider data
throw `DnsResolutionUnavailable`. This read-only call does not request the
protected mnemonic.

## NFT collection metadata

An NFT that belongs to a collection includes a neutral `collection` descriptor
with its address, standard TEP-64 fields, and complete string metadata. Keep
product-specific classification, such as Telegram gifts, in the application.

## TON Connect

The generated Swift module includes `TonConnectSession`, manifest parsing,
account reply data, and `ton_proof` signing. The application owns manifest and
bridge transport, approval screens, and protected session storage.

Read [TON_CONNECT.md](TON_CONNECT.md) for the required session and bridge POST
order. The [Swift example](examples/swift/README.md) contains a complete macOS
and iOS integration.

## Clear client secret copies

Rust clears the secret buffers that it owns. Swift `String` values are
immutable, so the application cannot reliably clear `RecoveryPhrase.phrase`.

Keep the phrase only while the recovery screen is visible. Do not write it to
logs, errors, analytics, state restoration, or application storage.

Use `Data` for mutable secret copies in the platform host. Reset every byte in
a `defer` block after the callback finishes.
