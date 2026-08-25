//! A callback-driven engine for TON wallet lifecycle, state, and transfers.
//!
//! The crate owns provider request construction, response parsing, wallet
//! derivation, transaction signing, and operation state. The embedding
//! application owns provider transport, protected storage, durable journal
//! storage, and the user interface.
//!
//! The public object model has three main entry points:
//!
//! - [`WalletLifecycle`] creates, imports, reveals, deletes, and prepares key rotation.
//! - [`WalletClient`] refreshes a wallet, loads activity, and sends transfers.
//! - [`TonConnectSession`] manages one encrypted native TON Connect session.
//!
//! A minimum integration has these steps:
//!
//! 1. Implement [`WalletPlatformHost`] for protected storage and journal data.
//! 2. Use [`WalletLifecycle`] to create or import a [`WalletDescriptor`].
//! 3. Persist the descriptor in application storage.
//! 4. Implement [`WalletHttpHost`] or [`WalletStatuslessHost`] with bounded
//!    provider requests and cancellation.
//! 5. Construct [`WalletClient`] from the descriptor and provider configuration.
//! 6. Read [`WalletClient::snapshot`] or wait with [`WalletClient::wait_for_change`].
//! 7. Call [`WalletClient::shutdown`] before the application releases host resources.
//!
//! Rust never holds the wallet-state lock while it awaits a host callback.
//! Host callbacks can therefore call unrelated application code without a
//! wallet-state lock cycle. Hosts own chain streams and native TON Connect
//! transport connections.

#![cfg_attr(
    test,
    allow(
        unused_results,
        clippy::arithmetic_side_effects,
        clippy::as_conversions,
        clippy::indexing_slicing,
        clippy::iter_over_hash_type,
        clippy::panic,
        clippy::pedantic,
        reason = "unit-test fixtures may use concise assertions and deliberately discard setup results"
    )
)]

mod domain;
mod engine;
mod ton_connect;
mod transport;
mod types;
mod wallet;

pub use domain::*;
pub use engine::{WalletClient, WalletPlatformHost};
pub use ton_connect::*;
pub use transport::*;
pub use types::{
    Base64Hash, Base64HashError, Boc, BocError, NonEmptyString, NonEmptyStringError,
    TonAddressError, TonAddressFormat, TonAddressInfo, TonAddressString, TonAddressStringError,
    UnsignedDecimalString, UnsignedDecimalStringError, convert_ton_address, is_valid_ton_address,
    mnemonic_wordlist, parse_ton_address,
};
pub use wallet::*;

uniffi::setup_scaffolding!();
