//! Callback-driven wallet engine.
//!
//! V3 owns request construction and provider parsing, while the embedding
//! language performs bounded HTTP calls. Streaming is entirely outside this
//! crate's API and state model.

mod domain;
mod engine;
mod provider;
mod send;
mod signer;
mod wallet;
mod wallet_crypto;

pub use domain::*;
pub use engine::{WalletClientV3, WalletHttpHostV3, WalletPlatformHostV3};
pub use wallet::*;

uniffi::setup_scaffolding!();
