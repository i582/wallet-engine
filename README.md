# Wallet Engine

Wallet Engine is a Rust library for TON wallet applications. It gives Swift,
Kotlin, and TypeScript applications the same wallet behavior.

Use Wallet Engine to:

- create and import wallets.
- protect recovery phrases with platform storage.
- parse, validate, and format TON addresses.
- parse `ton://transfer/` links.
- read balances, enriched activity, and NFT collection metadata.
- load additional history pages.
- resolve `.ton` wallet records.
- create and decrypt encrypted transfer comments.
- sign and submit transfers.
- connect dApps, sign `ton_proof`, and approve TON Connect transactions.
- observe immutable wallet snapshots.
- cancel active wallet operations.

The engine supports mainnet and testnet. The host application supplies network
access and platform security services.

## Choose your platform

![Wallet Engine on iOS, web, Android, and C++](readme-platforms.png)

### Swift

Generate the Swift source and its C module:

```shell
just bindings-swift
```

Then read [SWIFT.md](SWIFT.md) for the generated paths and Apple integration
requirements.

For a complete SwiftUI integration, see the
[Swift wallet example](examples/swift/README.md). It runs on macOS and iOS and
implements the HTTP, Keychain, and journal callbacks required by the engine.

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

For a complete Jetpack Compose integration, read the
[Android wallet example](examples/android/README.md). The example implements
the HTTP, Android Keystore, and journal callbacks that the engine requires.

### TypeScript and WebAssembly

Generate the browser WebAssembly package:

```shell
just bindings-wasm
```

The repository also contains the source of the high-level
`@ton/wallet-engine` TypeScript package. It provides a browser HTTP host,
IndexedDB journal storage, wallet lifecycle methods, wallet client methods, and
a wallet-side TON Connect runtime.

Read [WASM.md](WASM.md) for browser setup and security requirements.
Read [TON_CONNECT.md](TON_CONNECT.md) for the TON Connect API and persistence
contract.

For a small React integration, see the
[web wallet example](examples/web/README.md). It creates a testnet wallet,
shows the recovery phrase, refreshes the balance and activity, and loads more
history. The engine integration is kept separate from the interface code.

### Generated C++ (experimental)

Generate the experimental C++20 wrapper:

```shell
just bindings-cpp
```

The output is written to `bindings/cpp-experimental`. See the
[generated C++ Qt example](examples/cpp-bindgen/README.md) for a small desktop
wallet with balance refresh, activity, transfer preview/send, native HTTP and
platform-host implementations, and build commands.

This repository does not track generated bindings. Generate them from the same
revision that you use to build the Rust library.

### Binding generator sources

The pinned binding generators live together under `bindgen/`:

- `bindgen/apple` — Swift and C UniFFI generation.
- `bindgen/kotlin` — Kotlin UniFFI generation.
- `bindgen/wasm` — the WebAssembly facade.
- `bindgen/cpp` — the UniFFI C++ generator and its own test workspace.

Generated packages remain under `bindings/` and are intentionally ignored.

## Releases

An explicit `vMAJOR.MINOR.PATCH` tag creates one GitHub Release. SemVer
pre-release tags are also supported.

Each release contains native Linux and macOS libraries, a Swift package, an
Android AAR, and a TypeScript package with WebAssembly. The release also
contains checksums, a machine-readable manifest, and build provenance.

Read [RELEASING.md](RELEASING.md) for the file list and the release procedure.

## Rust

For a native Rust integration, see the
[Ratatui wallet example](examples/tui/README.md). It implements both host
interfaces directly in Rust and provides create, import, refresh, history,
send, persistence, and delete flows.

This repository does not track generated bindings. Generate them from the same
revision that you use to build the Rust library.

### Rust tests

Install a current Acton CLI that supports `acton localnet`.

`cargo nextest run` executes unit and scenario tests in parallel. Localnet
scenarios start temporary Acton nodes on free loopback ports. The scenarios
cover wallet deployment, transfers, refresh, pagination, cancellation, and
concurrent chain changes. The TON Connect suite also starts the official Go
bridge and a local TypeScript dApp actor.

If Acton is not in `PATH`, set the path explicitly:

```shell
WALLET_ENGINE_ACTON_BIN=/path/to/acton cargo nextest run --locked
```

Use `just test` to also build the generated C++ example and run Rust
documentation tests.

Read [the TON Connect test guide](tests/ton-connect/README.md) for its Go and
Node.js setup.

## Integration model

Wallet Engine owns the wallet state machine. Your application implements one
provider host and the platform host.

| Interface | Your application provides |
| --- | --- |
| `WalletHttpHost` | Bounded HTTP requests and cancellation by request ID |
| `WalletStatuslessHost` | Body-or-error requests through an opaque relay or protocol proxy |
| `WalletPlatformHost` | Protected secrets and durable journal storage |

The host callbacks let the same Rust engine run on Apple, Android, and browser
platforms. The engine does not own platform networking or protected storage.

Use the normal `WalletClient` constructor with `WalletHttpHost`. The HTTP host
must disable redirects and return the observed `status`, headers, body, and
`finalUrl`; the engine rejects a `finalUrl` different from the request URL and
uses HTTP status plus numeric `Retry-After` for retry classification.

Use `WalletClient::new_statusless` in Rust, or the generated `newStatusless`
constructor, when an opaque relay or protocol proxy exposes only the response
body or a transport error. In this mode the request URL is a logical Toncenter
destination rather than proof of a direct origin connection. The host must not
follow or emulate provider redirects, and it does not return a status, headers,
or final URL. The engine can classify only explicit provider codes present in
the body; it does not invent HTTP metadata or `Retry-After`.

See the [TypeScript provider transport example](examples/provider-transports-ts/README.md)
for separate direct HTTP and relay-backed integrations.

TON Connect uses the same wallet hosts. Native applications also own manifest
HTTP, bridge HTTP and SSE, protected session storage, and approval screens.
Read [TON_CONNECT.md](TON_CONNECT.md) for the required delivery order.

## Basic wallet flow

1. Implement either `WalletHttpHost` or `WalletStatuslessHost`, plus
   `WalletPlatformHost`, for your platform.
2. Create `WalletLifecycle` with your platform host.
3. Create or import a wallet.
4. Store the returned wallet descriptor in your application storage.
5. Show the recovery phrase only in the required user flow.
6. Create `WalletClient` with the descriptor data and both host objects.
7. Call `refresh()` to load the initial wallet state.
8. Publish the returned snapshot in your user interface.
9. Call `waitForChange` to receive later snapshot revisions.
10. Call `shutdown()` before you discard the client.

The wallet descriptor contains the stable application record ID, address,
Ed25519 public key, network, and protected-secret reference. Your application
needs these values after a restart. The public key is not a secret.

## Standalone utilities

The engine exports these utilities through Rust, Swift, Kotlin, TypeScript, and
the experimental C++ binding. They do not need a `WalletClient` or host
callbacks.

### Address formats

Swift, Kotlin, and TypeScript expose `parseTonAddress`, `isValidTonAddress`, and
`convertTonAddress`. Rust and C++ expose `parse_ton_address`,
`is_valid_ton_address`, and `convert_ton_address`.

These functions accept raw and
[TEP-2 user-friendly addresses](https://docs.ton.org/llms/foundations/addresses/formats/content.md).
Parsing returns the workchain and the user-friendly `bounceable` and `testnet`
flags. Conversion returns a canonical raw or user-friendly representation.
User-friendly output uses unpadded URL-safe Base64.

### TON transfer links

Swift, Kotlin, and TypeScript expose `parseTonTransferLink`. Rust and C++ expose
`parse_ton_transfer_link`.

The parser implements the strict `ton://transfer/` baseline. It accepts
`amount`, `text`, `exp`, `jetton`, and `bin`. The result preserves the recipient,
Gram or jetton asset, exact amount, text or BOC payload, and expiration policy.

The parser validates addresses, decimal values, and single-root BOCs. It does
not resolve chain state, check expiration, select bounce behavior, or authorize
a send. Query parsing follows URI rules, so a literal `+` remains `+`.

### Mnemonic word list

Swift, Kotlin, and TypeScript expose `mnemonicWordlist`. Rust and C++ expose
`mnemonic_wordlist`.

The function returns the 2048 English BIP-39 words in their original index
order. It does not filter or reorder the list. Applications can filter the list
locally for prefix suggestions.

This export provides the word vocabulary, not another recovery scheme. Wallet
import accepts only the Rotation mnemonic scheme described below.

## Key derivation

The engine uses the **Rotation mnemonic** scheme defined in
[TEP-0003 section 3.3](https://github.com/ton-blockchain/TEPs/blob/master/text/0003-wallets.md#33-rotation-mnemonic).
A complete phrase contains 24 words, divided into two independent 12-word
Multichain mnemonics.

| Half    | Words | Key         | Role                                                                            |
|---------|-------|-------------|---------------------------------------------------------------------------------|
| Anchor  | 1–12  | Anchor key  | Determines the wallet account address and authorizes the first rotation. Never changes. |
| Signing | 13–24 | Signing key | Signs ordinary outgoing messages. Replaced on rotation.                         |

Each half is a valid 12-word Multichain mnemonic on its own, BIP-39 checksum
included. The halves are generated independently, converted to keys
independently, and never joined into a single 24-word mnemonic. The TEP defines
the derivation; this document does not repeat it.

This is the only recovery scheme the engine supports. A new wallet starts
before its one-time key rotation: the signing key equals the anchor key, so
`createWallet` generates a single 12-word phrase. Rotation later gives the
phrase its second, independent half. `importWallet` therefore accepts the
phrase exactly as the user recorded it - 12 words before rotation or 24 words
after it - and expands the 12-word form internally; applications never
duplicate words themselves. TON mnemonics and plain Multichain mnemonics are
rejected as invalid.

### Compatibility

One 24-word phrase can be valid under more than one scheme. On import an
application should run all the checks it supports — the TON mnemonic checksum,
the Multichain checksum, and the two Rotation checksums over words 1–12 and
13–24 — and should not pick a scheme silently. When more than one validates,
derive the accounts for each scheme and let the user choose. See
[sections 7 and 8](https://github.com/ton-blockchain/TEPs/blob/master/text/0003-wallets.md#8-wallet-import-mnemonic-scheme-detection).

Rotation support is optional for other wallets. One that skips it reports a
Rotation phrase as invalid, which is the expected outcome.

### Wallet contract

A Rotation mnemonic needs a wallet contract whose address does not depend on
the signing key, and which accepts only its stored signing key for ordinary
outgoing messages
([section 13.1](https://github.com/ton-blockchain/TEPs/blob/master/text/0003-wallets.md#131-account-requirements)).
Wallet V3R1 through V5R1 have no rotatable signing key, and the TEP forbids
deploying them from a Rotation mnemonic.

The engine embeds Wallet rev00. This revision is not declared final. Its
initial `StateInit` and address use the anchor public key. Ordinary external
and owner-authorized internal requests use the signing key. Before rotation
the two keys are equal. After rotation a 24-word phrase can sign requests for
an already active account without changing its address.

The engine does not yet construct the one-time on-chain key-change request or
compare an imported signing key with the key stored in an active account. It
therefore rejects deployment from a post-rotation 24-word phrase: the initial
contract still expects the anchor key until that on-chain change has happened.

### Host and API impact

`WalletDescriptor.publicKey` holds the anchor public key: it determines the
account address and never changes, so persisted descriptors survive rotation.
The current signing key is on-chain state, not descriptor state.
`revealRecoveryPhrase` returns the phrase exactly as recorded: 12 words before
rotation or 24 words after it. Protected storage holds one secret.

## Wallet flow diagrams

The diagrams below cover the wallet lifecycle and transfer flows. Terminal
nodes use both a text prefix and a color, so their meaning does not depend on
color alone:

- `CALL ERROR` means a thrown Swift/Kotlin error or a rejected TypeScript
  promise.
- `RETURN VALUE` means the method completed normally, even when its outcome or
  phase is not successful.
- `SNAPSHOT STATE` means the method published observable wallet state.
- green is successful, red is a call error, blue is snapshot state, and yellow
  needs explicit handling but is safe to return.

```mermaid
flowchart TD
    Lifecycle["WalletLifecycle<br/>create · import · reveal · delete"]
    Descriptor["WalletDescriptor<br/>public metadata + secret reference"]
    Lifecycle --> Descriptor
    Descriptor --> Client["WalletClient"]
    Client --> Read["refresh · loadMoreActivity"]
    Client --> Preview["previewSend"]
    Client --> Send["send"]
    Client --> Resolve["resolvePending"]
    Client --> Observe["snapshot · waitForChange"]
    Read --> Snapshot["WalletSnapshot"]
    Send --> Snapshot
    Resolve --> Snapshot
    Observe --> Snapshot
    Client --> Stop["cancel… · shutdown"]

    classDef result fill:#dcfce7,stroke:#15803d,color:#14532d;
    class Descriptor,Snapshot result;
```

Operations that require a running client can return `Shutdown` or
`StateUnavailable`. Operations that allocate request IDs or revisions can
return `IdentifierExhausted`. Provider flows can return
`InvalidProviderBaseUrl` while constructing a request. These shared guards are
omitted below unless they materially change a flow.

### Create, import, reveal, and delete

Recovery words exist only across the lifecycle and protected-storage boundary.
They are never stored in `WalletDescriptor` or `WalletSnapshot`.

```mermaid
flowchart TD
    LifecycleNew["WalletLifecycle.new(platformHost)<br/>RETURN WalletLifecycle"]
    LifecycleNew --> Create
    LifecycleNew --> Import
    LifecycleNew --> Reveal
    LifecycleNew --> Delete

    Create["createWallet"] --> ValidateCreate{"Valid record ID?"}
    ValidateCreate -- no --> InvalidRecord["CALL ERROR<br/>InvalidRecordId"]
    ValidateCreate -- yes --> Generate["Generate 12-word Rotation mnemonic"]
    Generate --> Generated{"Mnemonic generated?"}
    Generated -- no --> InvalidGenerated["CALL ERROR<br/>InvalidRecoveryPhrase"]
    Generated -- yes --> DeriveCreate["Derive wallet address and public key"]

    Import["importWallet"]
    ValidateImport{"Valid record ID<br/>and mnemonic?"}
    InvalidImport["CALL ERROR<br/>InvalidRecordId or InvalidRecoveryPhrase"]
    Import --> ValidateImport
    ValidateImport -- no --> InvalidImport
    ValidateImport -- yes --> DeriveCreate

    DeriveCreate --> DeriveOk{"Derivation succeeded?"}
    DeriveOk -- no --> DeriveError["CALL ERROR<br/>AddressDerivationFailed"]
    Store["Host stores mnemonic<br/>with user-presence policy"]
    DeriveOk -- yes --> Store
    Store --> StoreOk{"Stored?"}
    StoreOk -- no --> SecretHost["CALL ERROR<br/>ProtectedSecretHost"]
    Created["RETURN VALUE<br/>create: descriptor + one-shot phrase<br/>import: descriptor"]
    StoreOk -- yes --> Created

    Reveal["revealRecoveryPhrase"]
    ValidateDescriptor{"Descriptor identity valid?"}
    Reveal --> ValidateDescriptor
    ValidateDescriptor -- no --> InvalidDescriptor["CALL ERROR<br/>InvalidRecordId"]
    ReadSecret["Host authenticates user<br/>and reads mnemonic"]
    ValidateDescriptor -- yes --> ReadSecret
    ReadSecret --> ReadOk{"Readable and valid?"}
    ReadOk -- host failure --> SecretHost
    ReadOk -- invalid mnemonic --> InvalidPhrase["CALL ERROR<br/>InvalidRecoveryPhrase"]
    ReadOk -- yes --> RevealDerive{"Address derivation succeeds?"}
    RevealDerive -- no --> DeriveError
    RevealDerive -- yes --> Match{"Mnemonic derives<br/>descriptor address?"}
    Match -- no --> Mismatch["CALL ERROR<br/>SecretWalletMismatch"]
    Match -- yes --> Phrase["RETURN VALUE<br/>one-shot RecoveryPhrase"]

    Delete["deleteWallet"] --> ValidateDelete{"Descriptor identity valid?"}
    ValidateDelete -- no --> InvalidDescriptor
    ValidateDelete -- yes --> DeleteSecret["Host deletes protected secret"]
    DeleteSecret --> DeleteOk{"Deleted?"}
    DeleteOk -- no --> SecretHost
    DeleteOk -- yes --> Deleted["RETURN VALUE<br/>success; metadata and journal remain"]

    classDef result fill:#dcfce7,stroke:#15803d,color:#14532d;
    classDef callError fill:#fee2e2,stroke:#b91c1c,color:#7f1d1d;
    class Created,Phrase,Deleted result;
    class InvalidRecord,InvalidImport,InvalidGenerated callError;
    class DeriveError,SecretHost callError;
    class InvalidDescriptor,InvalidPhrase,Mismatch callError;
```

### Construct and observe a client

`WalletSnapshot` is immutable. Each published change creates a new revision;
observers never receive a mutable view of engine state.

```mermaid
flowchart TD
    New["Construct WalletClient"]
    SecretRef{"Local secret reference<br/>is absent or nonblank?"}
    New --> SecretRef
    SecretRef -- no --> BadSecretRef["CALL ERROR<br/>InvalidLocalSecretReference"]
    SecretRef -- yes --> PublicKey{"Public key derives wallet state?"}
    PublicKey -- no --> BadKey["CALL ERROR<br/>InvalidWalletPublicKey"]
    PublicKey -- yes --> Identity{"Address matches key + network?"}
    Identity -- no --> BadIdentity["CALL ERROR<br/>WalletIdentityMismatch"]
    Initial["RETURN VALUE<br/>WalletSnapshot revision 0<br/>all resources idle"]
    Identity -- yes --> Initial

    Initial --> SnapshotCall["snapshot"]
    SnapshotCall --> Clone["RETURN VALUE<br/>immutable snapshot clone"]

    Initial --> Wait["waitForChange(afterRevision)"]
    StartupResolve["Best-effort resolvePending<br/>failure does not block observation"]
    Wait --> StartupResolve
    StartupResolve --> Newer{"Current revision is newer?"}
    Newer -- yes --> Changed["RETURN VALUE<br/>current snapshot"]
    Newer -- no --> Suspend["Suspend without blocking a thread"]
    Suspend --> Published["A newer revision is published"]
    Published --> Changed
    Suspend --> Shut["CALL ERROR<br/>Shutdown"]

    classDef result fill:#dcfce7,stroke:#15803d,color:#14532d;
    classDef callError fill:#fee2e2,stroke:#b91c1c,color:#7f1d1d;
    class Initial,Clone,Changed result;
    class BadSecretRef,BadKey,BadIdentity,Shut callError;
```

A public-key-only client has no `localSecretRef`. It can refresh, paginate,
preview, observe, and resolve. Only `send` returns `LocalSigningUnavailable`.
`snapshot()` remains readable after shutdown; a suspended `waitForChange`
returns `Shutdown`.

### Refresh account and activity

Account and activity requests run concurrently. The engine waits for both, then
publishes the account and activity resource states separately. An HTTP or
provider failure is stored as `ResourceState.error`; it does not turn the whole
call into a thrown `WalletClientError`.

Each nonzero transfer in `ActivityItem` includes the total transaction fee in
`transactionFeeNanograms`, an on-chain `status` (`success`, `failed`, or
`bounced`), and an optional zero-opcode plaintext `comment`. If one transaction
produces multiple activity rows, its total fee is repeated on every row and
must not be summed per row.

An item with opcode `0x2167da4b` instead exposes its complete message-body BOC
as `encryptedComment`. Decryption is deliberately explicit: pass that BOC and
the sender address to `WalletClient.decryptComment`. For a received item the
sender is `counterparty`; for a sent item it is the configured wallet address.
The call asks `WalletPlatformHost` for the protected mnemonic with
`SecretAccessReason.decryptComment`, so an ordinary refresh never opens an
authentication prompt.

```mermaid
flowchart TD
    Refresh["refresh"]
    Recover["Best-effort resolvePending<br/>failure is ignored here"]
    Refresh --> Recover
    Recover --> Loading["account = loading<br/>activity = loading"]
    Loading --> Parallel["Run both HTTP requests concurrently"]
    Parallel --> Joined["Wait for both responses"]

    Joined --> Account{"Account response valid?"}
    Account -- yes --> AccountReady["SNAPSHOT STATE<br/>account = ready"]
    AccountFailed["SNAPSHOT STATE<br/>failed + DomainError<br/>keep old account"]
    Account -- no --> AccountFailed

    AccountReady --> Activity{"Activity response valid?"}
    AccountFailed --> Activity
    ActivityReady["SNAPSHOT STATE<br/>merge head page; activity = ready"]
    ActivityFailed["SNAPSHOT STATE<br/>failed + DomainError<br/>keep old activity"]
    Activity -- yes --> ActivityReady
    Activity -- no --> ActivityFailed

    ActivityReady --> Count
    ActivityFailed --> Count
    Count{"Failed resources"}
    Count -- 0 --> Complete["RETURN VALUE<br/>WalletUpdate: Completed"]
    Count -- 1 --> Partial["RETURN VALUE<br/>WalletUpdate: PartiallyCompleted"]
    Count -- 2 --> Failed["RETURN VALUE<br/>WalletUpdate: Failed"]

    Refresh --> NewRefresh["A newer refresh starts"]
    Superseded["RETURN VALUE<br/>old refresh: Superseded"]
    NewRefresh --> Superseded

    classDef result fill:#dcfce7,stroke:#15803d,color:#14532d;
    classDef state fill:#dbeafe,stroke:#1d4ed8,color:#1e3a8a;
    classDef soft fill:#fef3c7,stroke:#b45309,color:#78350f;
    class Complete result;
    class AccountReady,ActivityReady,AccountFailed,ActivityFailed state;
    class Partial,Failed,Superseded soft;
```

`DomainError` classifies transport, provider protocol, rate limit,
cancellation, and host-policy failures and includes retry advice.

### Load more activity

```mermaid
flowchart TD
    More["loadMoreActivity"]
    Allowed{"No refresh/page load active,<br/>hasMore and cursor exist?"}
    More --> Allowed
    Skipped["RETURN VALUE<br/>WalletUpdate: Skipped<br/>no HTTP request"]
    Allowed -- no --> Skipped
    Allowed -- yes --> Page["Request next older page"]
    Page --> Response{"Response"}
    Response -- valid --> Advance{"Cursor moves to older LT?"}
    Advance -- yes --> Merge["Merge by item ID<br/>sort by descending LT"]
    Completed["RETURN VALUE<br/>WalletUpdate: Completed<br/>itemsAdded = new rows"]
    Merge --> Completed
    StopPages["RETURN VALUE<br/>Completed; 0 added; hasMore = false"]
    Advance -- no --> StopPages
    Cancelled["RETURN VALUE: Cancelled<br/>SNAPSHOT STATE: pagination idle"]
    Failed["RETURN VALUE: Failed<br/>SNAPSHOT STATE: DomainError"]
    Response -- host cancelled --> Cancelled
    Response -- other error --> Failed

    More --> RefreshWins["A refresh starts"]
    RefreshWins --> Superseded["RETURN VALUE<br/>page call: Superseded"]

    classDef result fill:#dcfce7,stroke:#15803d,color:#14532d;
    classDef soft fill:#fef3c7,stroke:#b45309,color:#78350f;
    classDef error fill:#fee2e2,stroke:#b91c1c,color:#7f1d1d;
    class Merge,Completed result;
    class Skipped,StopPages,Cancelled,Failed soft;
    class Superseded soft;
```

### Preview a transfer

Preview uses fresh chain state and a fake signature. It never reads the journal
or recovery phrase, and its BOC is never reused by `send`.

```mermaid
flowchart TD
    Preview["previewSend"] --> Busy{"Send or preview already active?"}
    Busy -- send --> SendBusy["CALL ERROR<br/>SendAlreadyInProgress"]
    Busy -- preview --> PreviewBusy["CALL ERROR<br/>SendPreviewAlreadyInProgress"]
    Busy -- no --> Account["Load fresh account state"]
    Account --> AccountOk{"Valid response?"}
    AccountOk -- no --> PreviewFailed["CALL ERROR<br/>SendPreviewFailed"]
    AccountOk -- yes --> Balance{"Exact amount <= balance?"}
    Balance -- no --> NoBalance["CALL ERROR<br/>InsufficientBalance"]
    Balance -- yes --> Status{"Account status"}
    Status -- active --> Seqno["Load fresh seqno"]
    Status -- nonexistent / uninitialized --> Zero["Use seqno 0 + StateInit"]
    Status -- frozen / unknown --> Unavailable["CALL ERROR<br/>SendAccountUnavailable"]
    Seqno --> SeqnoOk{"Valid seqno response?"}
    SeqnoOk -- no --> PreviewFailed
    SeqnoOk -- yes --> Build
    Zero --> Build["Calculate validUntil<br/>build fake-signed wallet BOC"]
    Build --> Prepared{"Preview BOC prepared?"}
    Prepared -- time overflow --> PreviewFailed
    Prepared -- BOC build failed --> PreviewFailed
    Prepared -- yes --> Emulate["Toncenter emulation<br/>ignore_chksig"]
    Emulate --> EmulationResult{"Emulation result"}
    EmulationFailed["CALL ERROR<br/>EmulationFailed"]
    NotAccepted["CALL ERROR<br/>EmulationMessageNotAccepted"]
    Rejected["CALL ERROR<br/>EmulationRejected<br/>includes TVM phase codes"]
    Fees{"Exact amount + wallet fee<br/>fits with positive remainder?"}
    EmulationResult -- transport / schema failure --> EmulationFailed
    EmulationResult -- message not accepted --> NotAccepted
    EmulationResult -- wallet transaction failed --> Rejected
    EmulationResult -- wallet succeeded --> Fees
    Fees -- no --> FeeBalance["CALL ERROR<br/>InsufficientBalanceForFees"]
    Result["RETURN VALUE<br/>SendPreview<br/>fees · actions · warnings · BOC"]
    Fees -- yes --> Result

    classDef result fill:#dcfce7,stroke:#15803d,color:#14532d;
    classDef callError fill:#fee2e2,stroke:#b91c1c,color:#7f1d1d;
    class Result result;
    class SendBusy,PreviewBusy,PreviewFailed,NoBalance,Unavailable callError;
    class EmulationFailed,NotAccepted,Rejected,FeeBalance callError;
```

Child transaction failures can make `traceSucceeded` false without rejecting
the preview. Only failure of the source wallet transaction produces
`EmulationRejected`.

### Sign and submit a transfer

The durable journal write is the critical boundary. Before it, cancellation is
safe. After it starts, the exact signed BOC may survive a crash or reach the
provider, so an uncertain result must remain unresolved.

```mermaid
flowchart TD
    Send["send"] --> Signer{"Local signing configured?"}
    Signer -- no --> NoSigner["CALL ERROR<br/>LocalSigningUnavailable"]
    Signer -- yes --> Busy{"Send or resolver active?"}
    Busy -- yes --> Already["CALL ERROR<br/>SendAlreadyInProgress"]
    Busy -- no --> Journal["Load durable wallet send slot"]
    Journal --> JournalOk{"Journal valid and readable?"}
    JournalOk -- no --> PreError["CALL ERROR<br/>SendFailed<br/>SNAPSHOT STATE: Failed"]
    JournalOk -- yes --> Account["Load fresh account + provider time"]
    Account --> AccountOk{"Fresh account response valid?"}
    AccountOk -- no --> PreError
    AccountOk -- yes --> Prior{"Unresolved signed BOC exists?"}
    Prior -- yes --> Force{"request.force?"}
    Force -- no --> Resolve["Run pending-resolution flow"]
    Force -- yes --> Checks
    Blocked["CALL ERROR<br/>PreviousSubmissionUnresolved<br/>do not replace"]
    Resolve -- still pending --> Blocked
    Resolve -- provider / journal failure --> PreError
    Resolve -- terminal --> Checks
    Prior -- no --> Checks{"Balance, status, seqno,<br/>validUntil checks pass?"}
    Typed["CALL ERROR<br/>InsufficientBalance, account unavailable,<br/>or SendFailed"]
    Unlock["Host authenticates user<br/>and reads protected mnemonic"]
    Checks -- no --> Typed
    Checks -- yes --> Unlock
    Unlock --> Secret{"Mnemonic valid and<br/>matches wallet address?"}
    Secret -- no --> SecretError["CALL ERROR<br/>InvalidProtectedSecret or SendFailed"]
    Sign["Build and sign fresh wallet BOC<br/>zeroize Rust secret buffer"]
    Signed{"Signed BOC built?"}
    Boundary["DURABLE COMMIT BOUNDARY<br/>CAS signed BOC into journal"]
    Secret -- yes --> Sign
    Sign --> Signed
    Signed -- no --> PreError
    Signed -- yes --> Boundary

    Boundary --> Persisted{"Prepared journal CAS result"}
    UnknownError["CALL ERROR<br/>SubmissionUnknown<br/>SNAPSHOT STATE: SubmissionUnknown"]
    JournalConflict["CALL ERROR<br/>SendAlreadyInProgress<br/>SNAPSHOT STATE: Failed"]
    Persisted -- host error --> UnknownError
    Persisted -- CAS conflict --> JournalConflict
    Persisted -- applied --> Submit["Submit the exact journaled BOC"]
    Submit --> Provider{"Definite provider result?"}
    Accepted["Provider accepted BOC"]
    Rejected["Provider explicitly rejected BOC"]
    Unknown["Provider outcome is ambiguous"]
    Provider -- accepted --> Accepted
    Provider -- explicit rejection --> Rejected
    Provider -- ambiguous response --> Unknown
    Accepted --> TerminalPersist{"Terminal CAS succeeds?"}
    Rejected --> TerminalPersist
    Unknown --> TerminalPersist
    TerminalPersist -- no --> UnknownError
    TerminalPersist -- accepted persisted --> SubmittedResult
    TerminalPersist -- rejection persisted --> FailedResult
    TerminalPersist -- uncertainty persisted --> UnknownResult

    SubmittedResult["RETURN VALUE<br/>phase = Submitted<br/>not confirmed on-chain"]
    FailedResult["RETURN VALUE: phase = Failed<br/>SNAPSHOT STATE: Failed"]
    UnknownResult["RETURN VALUE: SubmissionUnknown<br/>SNAPSHOT STATE: SubmissionUnknown"]

    classDef result fill:#dcfce7,stroke:#15803d,color:#14532d;
    classDef soft fill:#fef3c7,stroke:#b45309,color:#78350f;
    classDef callError fill:#fee2e2,stroke:#b91c1c,color:#7f1d1d;
    class Boundary soft;
    class SubmittedResult result;
    class FailedResult,UnknownResult soft;
    class NoSigner,Already,PreError,Blocked,Typed callError;
    class SecretError,UnknownError,JournalConflict callError;
```

An explicit provider rejection becomes a definite `SendResult` with phase
`failed` only after the terminal journal CAS succeeds. A transport failure
after submission is not definite: it produces `submissionUnknown`, and the
application must resolve it before signing another payment by default.

Set `SendRequest.force` only after the user explicitly confirms a replacement.
It allows a new signature while the previous signed BoC remains unresolved.
The previous transfer can still execute, so both transfers can affect the
wallet balance. `SendSnapshot.resolution.canForceRetry` reports whether the
current durable send can be overridden this way.

### Resolve a durable pending send

`resolvePending` never unlocks the mnemonic and never signs. It evaluates
evidence from strongest to weakest so a confirmed message is not mistaken for
a replacement merely because both advance the wallet seqno.

```mermaid
flowchart TD
    Resolve["resolvePending"] --> Busy{"Send or resolver active?"}
    Busy -- yes --> Already["CALL ERROR<br/>SendAlreadyInProgress"]
    Busy -- no --> Journal["Load durable send journal"]
    Journal --> Exists{"Journal state"}
    Exists -- absent --> Current["RETURN VALUE<br/>current SendSnapshot"]
    Exists -- already terminal --> Terminal["RETURN VALUE<br/>terminal SendSnapshot"]
    Exists -- invalid / host failure --> Failed["CALL ERROR<br/>SendFailed"]
    Exists -- pending --> Time["Load provider synchronization time"]
    Time -- provider failure --> Failed
    Time --> Executed{"Transaction found by<br/>inbound message hash?"}
    Executed -- provider failure --> Failed
    Executed -- yes --> Confirmed["Confirmed"]
    Executed -- no --> Mempool{"Message in pending set?"}
    Mempool -- yes --> InMempool["Still pending: InMempool"]
    Mempool -- absent or 404 / 405 --> SeqnoObserved
    Mempool -- other endpoint error --> SeqnoUnknown

    SeqnoObserved["Load wallet seqno<br/>pending absence observed"]
    SeqnoUnknown["Load wallet seqno<br/>expiration proof unavailable"]
    SeqnoObserved -- provider failure --> Failed
    SeqnoUnknown -- provider failure --> Failed
    SeqnoObserved --> AdvancedObserved{"Indexed seqno > signed seqno?"}
    SeqnoUnknown --> AdvancedUnknown{"Indexed seqno > signed seqno?"}
    Recheck{"Recheck transaction by message<br/>to close index snapshot race"}
    AdvancedObserved -- yes --> Recheck
    AdvancedUnknown -- yes --> Recheck
    Recheck -- provider failure --> Failed
    Recheck -- found --> Confirmed
    Recheck -- absent --> Replaced["Replaced"]
    Expired{"Pending absence observed and<br/>time > validUntil + margin?"}
    AdvancedObserved -- no --> Expired
    AdvancedUnknown -- no --> Awaiting
    Expired -- yes --> ExpiredResult["Expired"]
    Expired -- no --> Awaiting["Still pending: AwaitingWindow"]

    InMempool --> PendingReturn
    Awaiting --> PendingReturn
    PendingReturn["RETURN VALUE + SNAPSHOT STATE<br/>still unresolved; do not replace"]

    Confirmed --> Persist["CAS terminal evidence to journal"]
    Replaced --> Persist
    ExpiredResult --> Persist
    Persist --> Cas{"CAS result"}
    Cas -- applied --> Publish["RETURN VALUE + SNAPSHOT STATE<br/>terminal SendSnapshot"]
    Cas -- same resolution already persisted --> Publish
    Cas -- same pending record changed --> Retry["Retry CAS up to 3 times"]
    Retry --> Persist
    Cas -- invalid / disappeared / contended --> Failed

    classDef result fill:#dcfce7,stroke:#15803d,color:#14532d;
    classDef soft fill:#fef3c7,stroke:#b45309,color:#78350f;
    classDef callError fill:#fee2e2,stroke:#b91c1c,color:#7f1d1d;
    class Current,Terminal,Confirmed,Publish result;
    class Replaced,ExpiredResult,InMempool,Awaiting soft;
    class PendingReturn,Retry soft;
    class Already,Failed callError;
```

`refresh` and `waitForChange` make a best-effort attempt to recover a durable
pending send and ignore resolver failure. A later `send` runs the same evidence
algorithm but reports failure and refuses to sign while the old message remains
unresolved.

### Cancel and shut down

```mermaid
flowchart TD
    Shutdown["shutdown"] --> DoneAlready{"Already shut down?"}
    DoneAlready -- yes --> Done["RETURN VALUE<br/>success"]
    Closing["New running-only calls<br/>CALL ERROR: Shutdown"]
    DoneAlready -- no --> Closing
    Closing --> Committing{"A send crossed the<br/>durable commit boundary?"}
    Wait["Wait for active send to finish<br/>possibly as SubmissionUnknown"]
    Committing -- yes --> Wait
    Wait --> Committing
    CancelAll["Cancel reads, preview, resolver,<br/>and reversible send"]
    States["Loading resources become idle<br/>reversible send becomes cancelled"]
    Committing -- no --> CancelAll
    CancelAll --> States
    States --> Wake["Release waitForChange callers<br/>with CALL ERROR: Shutdown"]
    Wake --> Done

    CancelApi["cancelRefresh<br/>cancelLoadMoreActivity<br/>cancelSendPreview"]
    CancelHttp["RETURN VALUE<br/>success; cancel tracked HTTP<br/>no-op when idle"]
    CancelApi --> CancelHttp
    CancelHttp --> ReadResult["Active refresh/page<br/>RETURN VALUE: Superseded"]
    CancelHttp --> PreviewResult["Active preview<br/>CALL ERROR: StateUnavailable"]

    CancelSend["cancelSend"] --> Active{"Send active?"}
    Active -- no --> CancelNoop["RETURN VALUE<br/>success; no state change"]
    Active -- yes --> Boundary{"Before durable boundary?"}
    SendCancelled["RETURN VALUE: cancel success<br/>SNAPSHOT STATE: Cancelled"]
    Boundary -- yes --> SendCancelled
    SendCancelled --> SendResult["Active send<br/>CALL ERROR: StateUnavailable"]
    Boundary -- no --> TooLate["CALL ERROR<br/>SendCancellationTooLate"]

    classDef result fill:#dcfce7,stroke:#15803d,color:#14532d;
    classDef soft fill:#fef3c7,stroke:#b45309,color:#78350f;
    classDef callError fill:#fee2e2,stroke:#b91c1c,color:#7f1d1d;
    class Done result;
    class Wait,CancelHttp,ReadResult,CancelNoop,SendCancelled soft;
    class PreviewResult,SendResult,TooLate callError;
```

## Wallet state

`WalletSnapshot` is the only state that the user interface needs. It contains:

- the account balance and status.
- recent activity and its pagination cursor.
- owned NFTs and their resolved collection descriptors.
- independent states for account, activity, and pagination resources.
- the latest send state.
- a monotonic revision number.

A refresh can complete partially. Read each resource state before you replace
previous data or show an error.

When an NFT belongs to a collection, `NftItem.collection` contains a neutral
`NftCollectionDescriptor` with the verified collection address, standard
TEP-64 `name`, `description`, and `image` fields, plus all string-valued
collection metadata. Product-specific kinds such as Telegram gifts, usernames,
or numbers remain application-side classifications.

Use `waitForChange(afterRevision:)` on Swift. Use
`waitForChange(afterRevision)` on Kotlin. Both calls wait until a newer snapshot
is available.

## TON DNS

Call `WalletClient.resolveDns` with a `.ton` name before building a send intent.
The engine validates and lowercases the name, follows provider-side DNS
delegation, and selects the standard `wallet` category. It returns a
network-correct `TonAddressString`, or `None`/`nil`/`null` when the name has no
wallet record. It does not read the protected mnemonic.

`ProviderConfig.dnsRootAddress` is an optional resolver override. When it is
absent, the engine uses the current root address for the wallet network that was
snapshotted from blockchain config `#4`. This avoids an extra provider request
per lookup while allowing an application to update the resolver immediately.

DNS resolution is explicit and does not rewrite `SendIntent` destinations.
Malformed names, provider failures, or invalid provider records return the
typed `DnsResolutionUnavailable` error. The current API resolves ASCII `.ton`
names and wallet records only; product-specific naming and presentation stay
in the host. See the [TON DNS documentation](https://docs.ton.org/llms/foundations/web3/ton-dns/content.md)
and [TEP-81](https://github.com/ton-blockchain/TEPs/blob/master/text/0081-dns-standard.md).

## Sending GRAM

Call `previewSend` to show fees, actions, and execution warnings before the
user confirms a transfer. This call does not read the recovery phrase.

The preview is information, not permission to send. A preview failure does not
block `send`.

Chain state can change after the preview. Therefore, `send` always rebuilds the
wallet message from fresh account state. `engineDefault` also resolves a new
expiration from provider time. `exact` preserves the timestamp in the intent.

Call `send` after user confirmation. Give it a unique operation ID and the
same immutable `SendIntent` that the user approved.

`SendIntent` contains one expiration policy and an ordered batch of 1 to 255
`SendMessage` values. Each message contains its destination, amount, body, and
optional destination `StateInit`. A body is empty, a plaintext comment, or one
caller-built payload cell.

For an encrypted transfer comment, call `createEncryptedComment` before
preview. The engine calls `get_public_key` on the recipient wallet, authorizes
the protected sender mnemonic through the platform host, applies the TON
Ed25519/X25519, HMAC-SHA512, AES-256-CBC, and snake-cell format, and returns a
complete BOC. Put that BOC in `SendMessageBody.rawPayload`, then preview and
send the same immutable intent. Plaintext is limited to 960 UTF-8 bytes. A
recipient without an available on-chain `get_public_key` cannot receive this
format through this API. See the [TON encrypted-comments
format](https://docs.ton.org/llms/contracts/standard/wallets/interact/content.md#encrypted-comments).

`SendAmount.all` must be the only message in its batch. Wallet V5 applies the
batch in order.

Use `engineDefault` expiration for a normal wallet transfer. Use `exact` only
when a trusted caller supplies a Unix expiration timestamp. TON Connect uses
`exact` to preserve the dApp `valid_until` value.

Set `sendValiditySeconds` in `WalletClientConfig` according to your product's
submission policy. The engine adds this duration to the synchronization time
from the fresh Toncenter account response. It does not trust the device clock.
A short duration can expire before inclusion. A long duration leaves the
signed message valid for longer.

`previewSend` does these steps:

1. loads a fresh account state and sequence number.
2. builds the complete wallet intent from the stored public key.
3. emulates it with a placeholder signature and `ignore_chksig`.
4. returns fees, actions, trace status, and the preview expiration time.

`previewTonConnect` accepts the complete TON Connect `SendRequest`. It keeps the
dApp expiration, message order, payloads, and destination `StateInit` values.

`previewSignMessage` validates a sign-only intent from fresh public state. It
does not report wallet-paid fees because a relayer supplies the inbound TON.

`signMessage` signs and journals a Wallet V5 `internal_signed` request. It
returns a complete relaxed internal-message BoC with the `handedOff` phase. It
does not submit the message.

`send` starts a new workflow after confirmation:

1. loads the durable send journal.
2. loads a new account state and sequence number.
3. resolves the intent expiration policy.
4. requests the protected recovery phrase from the host.
5. makes sure that the phrase belongs to the selected wallet.
6. builds and signs a new wallet message in Rust.
7. stores the exact signed BoC in the host journal.
8. submits that BoC to the provider.

`SendRequest.force` defaults to `false` at JSON boundaries. When it is `true`,
`send` can replace an unresolved durable signed BoC without first obtaining
terminal provider evidence. Use it only after showing the unresolved transfer
and receiving explicit user confirmation.

`SendResult.signedBoc` contains the exact signed external-message BoC. TON
Connect returns this value to the dApp after an accepted or uncertain
submission. Do not log it.

The emulation request does not contain the mnemonic or private key. Child
transaction failures remain in `SendEmulation.traceSucceeded`; they do not
automatically block submission because a recipient can reject or bounce.
`SendEmulation.actions` contains the high-level actions returned by Toncenter.
Each action has validated Base64 identifiers, involved accounts, and its
action-specific details as JSON.

`SendPreviewFailed` means fresh state could not be loaded or the fake-signed
message could not be built. `EmulationFailed` means that the emulation service,
transport, or response failed. `EmulationMessageNotAccepted` means that the
current chain state rejected the external message, for example after another
client advanced the wallet seqno. `EmulationRejected` means that the external
message created a wallet transaction, but its compute or action phase failed.
The last error includes the returned TVM phase codes.

Handle every send phase explicitly:

| Phase | Meaning |
| --- | --- |
| `submitted` | Provider accepted the BoC; it is not confirmed on-chain. |
| `failed` | Provider explicitly rejected the request. |
| `submissionUnknown` | The BoC can be accepted, but the result is unknown. |
| `cancelled` | The operation stopped before the durable submission boundary. |
| `handedOff` | The signed internal BoC is durable and available to a relayer. |
| `sequenceNumberConsumed` | The reserved sequence number changed, but the exact relayer transaction is unknown. |

CAUTION: If the phase is `submissionUnknown`, do not create and submit a new
transfer automatically. The first signed message can already be in the
network.

The journal uses compare-and-swap writes. The host implementation must make
these writes durable before it reports success.

## TON Connect

Wallet Engine implements wallet-side TON Connect v2 sessions for native and
browser applications. It handles link parsing, authenticated encryption,
replay protection, raw transaction batches, gasless `signMessage`, proof
signing, and restart-safe responses.

The host shows every approval and owns protected session storage. Native hosts
also own manifest and bridge transport. The browser package provides this
transport through `TonConnectWallet`.

Read [TON_CONNECT.md](TON_CONNECT.md) for supported requests, send mapping,
session recovery, and the durable bridge-response sequence. See the
[Swift](examples/swift/README.md), [web](examples/web/README.md), and
[terminal](examples/tui/README.md) examples for complete integrations.

## Streaming

`WalletClient` does not contain a chain streaming API. The host application
owns its chain stream, reconnect policy, and application lifecycle.

Native TON Connect sessions parse SSE chunks but do not open the connection.
The native host owns the bridge stream and its reconnect policy. The browser
`TonConnectWallet` includes this transport.

The engine also does not reconcile stream events. A host can start a normal
refresh when its product behavior requires new wallet data.

## Security requirements

- Store recovery phrases with the platform protected-storage API.
- Require user authentication according to your product policy.
- Do not log recovery phrases, secret values, or signed BoCs.
- Add a Toncenter credential only when the request origin matches the configured
  Toncenter base URL.
- Enforce the request and response limits from each `HttpRequest`.
- Honor `cancelHttp` for requests that have not completed.
- Store wallet descriptors separately from protected recovery phrases.
- Store serialized TON Connect sessions as protected authentication material.
- Persist a pending TON Connect response before its bridge POST starts.

The engine clears the secret buffers that it owns. The FFI boundary and the
host language can create additional copies.

The host must limit the lifetime of these copies. The host must clear mutable
byte buffers after each callback. Immutable client strings cannot be reliably
cleared, so release them immediately after the recovery screen closes.

See [Swift](SWIFT.md), [Kotlin](KOTLIN.md), and [WebAssembly](WASM.md) for the
client-specific rules.

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

Run the bounded Kani proofs for the root Rust crate:

```shell
just kani-setup
just kani
```

Use `just kani-list` to list the available proof harnesses. Kani 0.67 bundles
Rust 1.93, so `verification/kani/Cargo.toml` points a verification-only package
at the production `src/lib.rs`. This keeps the production Rust 1.96.1
requirement intact and verifies the same source code.

Run `cargo xtask --help` to see the binding and Android build commands.

## License

Wallet Engine is available under the Apache License 2.0 or the MIT License.
See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT).
