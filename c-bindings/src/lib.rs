//! C ABI for Wallet Engine.
//!
//! This crate owns the C-compatible representations, exported symbols, and
//! panic containment needed by native consumers. The core `wallet-engine`
//! crate remains a Rust API and contains no C ABI declarations.

// Exporting stable symbols requires Rust's unsafe `no_mangle` attribute. Keep
// that exception local to the ABI modules.
#[allow(unsafe_code)]
mod abi;

pub use abi::{ABI_VERSION, wallet_engine_abi_version};
