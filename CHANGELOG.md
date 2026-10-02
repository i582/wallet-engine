# Changelog

This file records user-visible changes to Wallet Engine.

## [Unreleased]

### Changed

- `create_encrypted_comment` and `resolve_encrypted_comment_recipient` now treat `recipient_public_key` as a hint for an undeployed recipient. They always read the recipient account state: an active wallet's `get_public_key` answer is used and a supplied key is ignored, while a nonexistent or uninitialized account uses the supplied key after verifying that it derives the recipient address. A key derived from the address is only the wallet's initial key, so for a wallet that had rotated its key, a supplied key encrypted comments to a replaced key. Supplying a key no longer skips HTTP, and `resolve_encrypted_comment_recipient` with a key now takes the single-flight slot.

## [0.0.7] - 2026-09-29

### Added

- Added `detect_mnemonic_schemes`, which reports every scheme under which entered recovery words validate: `rotation` (importable), plus detection-only `ton` (passwordless legacy TON mnemonic) and `bip39` (24-word Multichain mnemonic). Applications use it to explain why an import was rejected; the engine still derives keys only from Rotation mnemonics.
- Added `WalletClient::send_boc` and platform bindings for durable submission of an already signed external BOC. It validates fresh metadata, shares the transfer journal and single-flight slot, and returns the standard `SendResult` for normal pending resolution.
- Added `WalletLifecycle::derive_ton_connect_session` and the `TonConnectDerivedSession` object for MTProto-relayed TON Connect sessions whose X25519 key pair is derived from the wallet's current signing key, the dApp client id, and a server nonce. The object exposes only the session public key and the signing public key, opens the server challenge, validates relayed requests, and encrypts raw `nonce || box` replies with caller-supplied ids; it persists nothing. Appended `SecretAccessReason::DeriveTonConnectSessionKey` and `WalletLifecycleError::InvalidTonConnectSessionInput`.
- Added `WalletClient::resolve_encrypted_comment_recipient` and platform bindings. It returns the recipient public key `create_encrypted_comment` would encrypt for, verifying a supplied key locally and otherwise reading the recipient account state and its `get_public_key`, without any protected-secret access, so applications can learn whether a comment can be encrypted before asking for authentication. Appended `WalletClientError::EncryptedCommentLookupFailed`.
- Added TON Connect `signData` (`text`, `binary`, `cell`) to derived sessions. Their connect event now advertises `SignData` beside `SendTransaction`, and `decrypt_request` returns the new `TonConnectIncomingRequest::SignData` with a `TonConnectSignDataRequest` (`TonConnectSignDataPayload`) validated against the session wallet. `WalletLifecycle::sign_ton_connect_data` signs a `TonConnectSignDataSignRequest` with the current signing key and returns `TonConnectSignedData`, which `TonConnectDerivedSession::encrypt_sign_data_success` answers with after verifying it with the session's signing key. The domain must be the manifest host, not a URL. `encrypt_connect_event` requires `ton_addr` to advertise the session's signing public key. Appended `SecretAccessReason::SignTonConnectData`.
- Added recovery of replaced signing keys to `WalletClient::decrypt_comment`, so it also decrypts comments sent to signing keys that a key rotation replaced. It tries the current signing key and then the anchor key without any HTTP request; when neither matches on a rotated wallet, it reads the wallet's `change_wallet_key` actions from Toncenter `/api/v3/actions` (indexer v1.3 or later), recovers each earlier signing key from the encrypted old key its rotation published, and tries those. The protected secret is still read once per call. The public history is cached in memory while it contains the rotation that installed the current signing key, and recovered keys are never returned or stored. `EncryptedCommentLookupFailed` also means that the history does not include the current signing key yet and the call can be retried; `EncryptedCommentUnavailable` means that no key of the wallet decrypts the body. The public API is unchanged.

### Changed

- **Breaking:** `TonConnectIncomingRequest` gained `SignData`, and `SecretAccessReason` gained `DeriveTonConnectSessionKey` and `SignTonConnectData`. Exhaustive Swift `switch` and Kotlin `when` over them must handle the new cases; native bridge sessions never produce `SignData`.
- **Breaking:** Moved `prepare_key_rotation` from `WalletLifecycle` to `WalletClient`. The request no longer contains a wallet descriptor or `seqno`; the client uses fresh account state, fetching the getter only for an active contract and using zero plus anchor-based `StateInit` before deployment.
- **Breaking:** Key-rotation requests (`ChangePublicKeyRequestE` and `ChangePublicKeyRequestI`) now carry a second reference after the rotation-proof signature, `encryptedOldPrivateKey = sha256(newPrivateKey ‖ "keyChangeSaltV1") XOR oldPrivateKey` over 32-byte Ed25519 seeds, and target the Wallet rev00 revision with bytecode hash `e30911420bef1191c09dce58b9df2b4ca4c2d9c383cc3b6a91170349ffa70e2c`. That revision publishes the value in a key-changed external-out log (opcode `0xEBA19948`). Signed rotation BOCs differ from earlier releases. The testnet config param `-123` and the localnet fixture `tests/support/wallet_tg_rev00.code` hold this revision.
- `create_encrypted_comment` now encrypts with the wallet's current signing key, the key its `get_public_key` reports, instead of the anchor key. Recipients are unaffected; the sender's own comments sent before a later rotation are decrypted through the key-change history like received ones.

### Fixed

- Wallet rev00 key rotation can be prepared repeatedly. Later rotations are authorized by the current signing half while preserving the anchor half and wallet address.
- Key rotation no longer interprets Toncenter's failed `seqno` getter stack for an undeployed account. It derives sequence number zero from account state and prepares a deployable rotation BOC with `StateInit`.
- `create_encrypted_comment` no longer reports a failed, rate-limited, or cancelled recipient key lookup as `EncryptedCommentUnavailable`, which told applications the recipient cannot receive encrypted comments; it returns `EncryptedCommentLookupFailed`. It reads the recipient account state before `get_public_key`, so an undeployed or frozen wallet is reported as unavailable, and it rejects a getter result with a failed TVM exit code instead of reading its stack as a key.
- TON Connect `ton_proof` replies now write `proof.timestamp` as a JSON number in every session, native bridge and derived alike. `@tonconnect/sdk` replaces a proof whose timestamp is a string with an error, so connecting with a proof to a strict dApp failed; `ton-connect-core` still accepts a canonical decimal string when it reads a proof.
- TON Connect `sendTransaction` and `signMessage` messages now use the bounce flag of the destination's user-friendly address, as the TON Connect specification requires, instead of always being non-bounceable.
- TON Connect cell `BoC`s deeper than 1024 cells, the TVM's maximum cell depth, are now rejected as invalid before the cell tree is built. A few thousand nested cells in a `signData` cell or a `sendTransaction` payload overflowed a 512 KiB thread stack and aborted the process. An indexed `BoC` whose header claims more index entries than it carries is rejected before its index is allocated.
- `ton://transfer` `bin` payloads and every other `Boc` read from text (UniFFI arguments, JSON) now pass the same envelope and depth validation as TON Connect cell `BoC`s before `ton_core` parses them. A 32-character `bin` claiming 2^32-1 cells made `ton_core` request about 1.2 TB and abort the process on Linux and Windows, a reference past the last cell panicked, and a few thousand nested cells overflowed the stack.
- TON Connect cell `BoC`s and every engine `Boc` now reject exotic cells whose layout TON refuses: a pruned branch whose size, level mask or stored depth does not match its data, and library, Merkle proof or Merkle update cells of the wrong size or reference count. A pruned branch shorter than its level mask made `ton_core` panic when it hashed the cell, which left a `sendTransaction` preview's slot taken, so every later preview on that client failed as already in progress.
- `prepare_key_rotation` on a nonexistent or uninitialized account now returns `KeyRotationUnavailable` for a post-rotation 24-word phrase, as transfers already did. It signed with words 13-24 while the attached `StateInit` deploys the anchor key, so the contract rejected the request.
- `decrypt_comment` no longer fails with `EncryptedCommentUnavailable` for a comment encrypted to the current signing key of a rotated wallet. It decrypted only with the anchor key, while senders encrypt to the key the wallet's `get_public_key` returns, which is the current signing key.

## [0.0.6] - 2026-08-26

### Added

- Added `WalletLifecycle::prepare_key_rotation` and platform bindings. It creates the second 12-word half, both contract signatures, and the signed key-change BOC.
- Added Windows x86-64 MSVC release archives with the static library, DLL, import library, and C++ wrapper.

### Changed

- **Breaking:** Wallet derivation and signing now use the embedded Wallet rev00 contract instead of the experimental Wallet V5 placeholder, including its contract code, subwallet IDs, state layout, and external and internal request encoding.

### Fixed

- Post-rotation Wallet rev00 requests now use the signing half of the recovery phrase while the account address and `StateInit` remain anchored to the first half. Deployment from a post-rotation phrase is rejected until the account is active on-chain.

## [0.0.5] - 2026-08-25

### Added

- Added `WalletStatuslessHost` and `WalletClient::new_statusless`, including generated platform bindings, for relays and protocol proxies that return only a provider body or an opaque host error.
- Added a runnable TypeScript provider-transport example covering both metadata-rich HTTP and body-only relay integrations.

### Fixed

- Post-rotation Wallet rev00 requests now use the signing half of the recovery phrase while the account address and `StateInit` remain anchored to the first half.
- Strict `ton://transfer/` parsing now rejects control characters, ambiguous normalized paths and authorities, and noncanonical raw recipient or jetton-master addresses.
- TON Connect device information now accepts the legacy `"SendTransaction"` feature alongside its detailed descriptor while continuing to reject exact duplicates.
- Status-less provider transports now recognize Toncenter error envelopes with explicit body codes, including rate limits and authentication failures.

## [0.0.4] - 2026-08-24

### Added

- Added `parse_ton_address`, `convert_ton_address`, and `is_valid_ton_address` for TON address parsing, validation, and conversion.
- Added `mnemonic_wordlist`, which returns the BIP-39 English word list in its original order.
- Added transaction fees, transfer statuses, plaintext comments, and encrypted-comment BOCs to activity items.
- Added NFT collection descriptors with the collection address, name, description, image, and provider metadata.
- Added `create_encrypted_comment` and `decrypt_comment` with protected-key access through the platform host.
- Added `.ton` DNS wallet-record resolution. `ProviderConfig.dns_root_address` overrides the default root for the selected network.
- Added strict `ton://transfer/` parsing for Gram and jetton transfers, exact amounts, text or BOC payloads, and expiration.

### Changed

- **Breaking:** Wallet creation and import now use TEP-0003 Rotation mnemonics.
- New wallets return a 12-word phrase before the first key rotation.
- Wallet import accepts 12-word pre-rotation phrases and 24-word post-rotation phrases.
- Wallet import now rejects TON mnemonics and plain Multichain mnemonics.

### Fixed

- C++ typed binding errors now return the Rust `Display` message from `what()`.
- The TUI recovery grid now uses the actual recovery-phrase length.

## [0.0.3] - 2026-08-20

Test third version

## [0.0.2] - 2026-08-19

Test second version

## [0.0.1] - 2026-08-19

Test first version
