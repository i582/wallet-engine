//! C ABI for Wallet Engine.
//!
//! This crate owns the C-compatible representations, exported symbols, and
//! panic containment needed by native consumers. The core `wallet-engine`
//! crate remains a Rust API and contains no C ABI declarations.

// Exporting stable symbols requires Rust's unsafe `no_mangle` attribute. Keep
// that exception local to the ABI modules.
#[allow(unsafe_code)]
mod abi;
mod types;

pub use abi::{
    ABI_VERSION, WalletEngineAbiStatus, WalletEngineBytesView, WalletEngineStringView,
    wallet_engine_abi_version,
};
pub use types::{
    WALLET_ENGINE_NETWORK_MAINNET, WALLET_ENGINE_NETWORK_TESTNET, WalletEngineNetwork,
    network_from_abi, network_to_abi,
};
