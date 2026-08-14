//! C representations of Wallet Engine domain types.

#[allow(unsafe_code)]
mod secret;
#[allow(unsafe_code)]
mod wallet;

use wallet_engine::Network;

use crate::abi::WalletEngineAbiStatus;

/// A validated numeric TON network value.
///
/// C supplies this value as an integer so Rust can reject unknown values before
/// constructing a core network value.
pub type WalletEngineNetwork = u32;

/// The production TON network.
pub const WALLET_ENGINE_NETWORK_MAINNET: WalletEngineNetwork = 0;

/// The public TON test network.
pub const WALLET_ENGINE_NETWORK_TESTNET: WalletEngineNetwork = 1;

/// Converts a C network value into the core domain type.
///
/// # Errors
///
/// Returns [`WalletEngineAbiStatus::InvalidArgument`] for an unknown value.
pub const fn network_from_abi(
    value: WalletEngineNetwork,
) -> Result<Network, WalletEngineAbiStatus> {
    match value {
        WALLET_ENGINE_NETWORK_MAINNET => Ok(Network::Mainnet),
        WALLET_ENGINE_NETWORK_TESTNET => Ok(Network::Testnet),
        _ => Err(WalletEngineAbiStatus::InvalidArgument),
    }
}

/// Converts the core domain type into its stable C value.
#[must_use]
pub const fn network_to_abi(value: Network) -> WalletEngineNetwork {
    match value {
        Network::Mainnet => WALLET_ENGINE_NETWORK_MAINNET,
        Network::Testnet => WALLET_ENGINE_NETWORK_TESTNET,
    }
}

pub use secret::{
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_AUTHENTICATION_FAILED,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_CANCELLED,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_NOT_FOUND,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE,
    WalletEngineProtectedSecretHostErrorKind, WalletEngineProtectedSecretHostErrorView,
    WalletEngineProtectedSecretRefView, WalletEngineProtectedSecretStoreView,
    protected_secret_host_error_kind_from_abi, protected_secret_host_error_kind_to_abi,
};
pub use wallet::{
    WalletEngineCreateWalletRequest, WalletEngineCreatedWalletView, WalletEngineRecoveryPhraseView,
    WalletEngineStringViewSlice, WalletEngineWalletDescriptorView,
    WalletEngineWalletLifecycleErrorCode, WalletEngineWalletLifecycleErrorView,
    with_created_wallet_view,
};
