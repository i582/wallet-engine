//! A callback-driven engine for TON wallet lifecycle, state, and transfers.
//!
//! The crate owns provider request construction, response parsing, wallet
//! derivation, transaction signing, and operation state. The embedding
//! application owns HTTP transport, protected storage, durable journal
//! storage, and the user interface.
//!
//! The public object model has two main entry points:
//!
//! - [`WalletLifecycle`] creates, imports, reveals, and deletes wallets.
//! - [`WalletClient`] refreshes a wallet, loads activity, and sends transfers.
//!
//! A minimum integration has these steps:
//!
//! 1. Implement [`WalletPlatformHost`] for protected storage and journal data.
//! 2. Use [`WalletLifecycle`] to create or import a [`WalletDescriptor`].
//! 3. Persist the descriptor in application storage.
//! 4. Implement [`WalletHttpHost`] with bounded HTTP requests and cancellation.
//! 5. Construct [`WalletClient`] from the descriptor and provider configuration.
//! 6. Read [`WalletClient::snapshot`] or wait with [`WalletClient::wait_for_change`].
//! 7. Call [`WalletClient::shutdown`] before the application releases host resources.
//!
//! Rust never holds the wallet-state lock while it awaits a host callback.
//! Host callbacks can therefore call unrelated application code without a
//! wallet-state lock cycle. Streaming updates remain outside this crate.

mod domain;
mod engine;
mod types;
mod wallet;

pub use domain::*;
pub use engine::{WalletClient, WalletHttpHost, WalletPlatformHost};
pub use types::{Base64Hash, Base64HashError, UnsignedDecimalString, UnsignedDecimalStringError};
pub use wallet::*;

uniffi::setup_scaffolding!();
