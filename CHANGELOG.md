# Changelog

This file records user-visible changes to Wallet Engine.

## [Unreleased]

### Added

- Added `detect_mnemonic_schemes`, which reports every scheme under which entered recovery words validate: `rotation` (importable), plus detection-only `ton` (passwordless legacy TON mnemonic) and `bip39` (24-word Multichain mnemonic). Applications use it to explain why an import was rejected; the engine still derives keys only from Rotation mnemonics.

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
