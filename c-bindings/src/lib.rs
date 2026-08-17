//! C ABI for Wallet Engine.
//!
//! This crate owns the C-compatible representations, exported symbols, and
//! panic containment needed by native consumers. The core `wallet-engine`
//! crate remains a Rust API and contains no C ABI declarations.

// Exporting stable symbols requires Rust's unsafe `no_mangle` attribute. Keep
// that exception local to the ABI modules.
#[allow(unsafe_code)]
mod abi;
#[allow(unsafe_code)]
mod host;
#[allow(unsafe_code)]
mod lifecycle;
mod types;

pub use abi::{
    ABI_VERSION, WalletEngineAbiStatus, WalletEngineBytesView, WalletEngineStringView,
    wallet_engine_abi_version,
};
pub use host::{
    WALLET_ENGINE_PLATFORM_HOST_CALLBACKS_SIZE, WalletEngineContextReleaseFn,
    WalletEngineContextRetainFn, WalletEnginePlatformHostAdapter,
    WalletEnginePlatformHostCallbacks, WalletEngineProtectedSecretStoreCompletion,
    WalletEngineStoreProtectedSecretFn, wallet_engine_protected_secret_store_completion_complete,
    wallet_engine_protected_secret_store_completion_free,
};
pub use lifecycle::{
    WalletEngineCreateWalletOperation, WalletEngineCreateWalletResultFn,
    WalletEngineImportWalletOperation, WalletEngineImportWalletResultFn, WalletEngineLifecycle,
    WalletEngineOperationPollState, wallet_engine_create_wallet_operation_free,
    wallet_engine_create_wallet_operation_poll, wallet_engine_import_wallet_operation_free,
    wallet_engine_import_wallet_operation_poll, wallet_engine_lifecycle_create_wallet_start,
    wallet_engine_lifecycle_free, wallet_engine_lifecycle_import_wallet_start,
    wallet_engine_lifecycle_new,
};
pub use types::{
    WALLET_ENGINE_NETWORK_MAINNET, WALLET_ENGINE_NETWORK_TESTNET,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_AUTHENTICATION_FAILED,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_CANCELLED,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_NOT_FOUND,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE, WalletEngineCreateWalletRequest,
    WalletEngineCreatedWalletView, WalletEngineImportWalletRequest, WalletEngineNetwork,
    WalletEngineProtectedSecretHostErrorKind, WalletEngineProtectedSecretHostErrorView,
    WalletEngineProtectedSecretRefView, WalletEngineProtectedSecretStoreView,
    WalletEngineRecoveryPhraseView, WalletEngineStringViewSlice, WalletEngineWalletDescriptorView,
    WalletEngineWalletLifecycleErrorCode, WalletEngineWalletLifecycleErrorView, network_from_abi,
    network_to_abi, protected_secret_host_error_kind_from_abi,
    protected_secret_host_error_kind_to_abi, with_created_wallet_view,
};
