//! Wallet client orchestration and callback interfaces for the host application.
//!
//! The submodules separate lifecycle, refresh, pagination, send, HTTP boundary,
//! validation, and mutable state. No operation holds the state lock while it
//! awaits a host callback.

mod activity;
mod client;
mod emulation;
mod expiration;
mod host;
mod http;
mod nft;
mod nft_transfer;
mod preview;
mod provider;
mod refresh;
mod resolution;
mod resolution_http;
mod resolve;
mod send;
mod send_http;
mod send_state;
mod sign_message;
mod state;
mod validation;

pub use client::WalletClient;
pub use host::{WalletHttpHost, WalletPlatformHost};
