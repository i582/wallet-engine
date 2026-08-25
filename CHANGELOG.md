# Changelog

This file records user-visible changes to Wallet Engine.

## [0.0.5] - 2026-08-25

### Added

- Added `WalletStatuslessHost` and `WalletClient::new_statusless`, including generated platform bindings, for relays and protocol proxies that return only a provider body or an opaque host error.
- Added a runnable TypeScript provider-transport example covering both metadata-rich HTTP and body-only relay integrations.

### Fixed

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
