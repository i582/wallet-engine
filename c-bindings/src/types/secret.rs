//! C representations of protected-secret types.

use wallet_engine::{ProtectedSecretHostErrorKind, ProtectedSecretRef, ProtectedSecretStore};

use crate::abi::{WalletEngineAbiStatus, WalletEngineBytesView, WalletEngineStringView};

/// A validated numeric protected-storage failure kind.
pub type WalletEngineProtectedSecretHostErrorKind = u32;

/// No secret exists for the supplied reference.
pub const WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_NOT_FOUND:
    WalletEngineProtectedSecretHostErrorKind = 0;

/// User or device authentication failed.
pub const WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_AUTHENTICATION_FAILED:
    WalletEngineProtectedSecretHostErrorKind = 1;

/// The user or host cancelled the operation.
pub const WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_CANCELLED:
    WalletEngineProtectedSecretHostErrorKind = 2;

/// Protected storage is temporarily unavailable.
pub const WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE:
    WalletEngineProtectedSecretHostErrorKind = 3;

/// The request violates a platform security policy.
pub const WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION:
    WalletEngineProtectedSecretHostErrorKind = 4;

/// The failure does not match another kind.
pub const WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER:
    WalletEngineProtectedSecretHostErrorKind = 5;

/// Converts a C protected-storage failure kind into the core domain type.
///
/// # Errors
///
/// Returns [`WalletEngineAbiStatus::InvalidArgument`] for an unknown value.
pub const fn protected_secret_host_error_kind_from_abi(
    value: WalletEngineProtectedSecretHostErrorKind,
) -> Result<ProtectedSecretHostErrorKind, WalletEngineAbiStatus> {
    match value {
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_NOT_FOUND => {
            Ok(ProtectedSecretHostErrorKind::NotFound)
        }
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_AUTHENTICATION_FAILED => {
            Ok(ProtectedSecretHostErrorKind::AuthenticationFailed)
        }
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_CANCELLED => {
            Ok(ProtectedSecretHostErrorKind::Cancelled)
        }
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE => {
            Ok(ProtectedSecretHostErrorKind::Unavailable)
        }
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION => {
            Ok(ProtectedSecretHostErrorKind::PolicyViolation)
        }
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER => {
            Ok(ProtectedSecretHostErrorKind::Other)
        }
        _ => Err(WalletEngineAbiStatus::InvalidArgument),
    }
}

/// Converts the core protected-storage failure kind into its stable C value.
#[must_use]
pub const fn protected_secret_host_error_kind_to_abi(
    value: ProtectedSecretHostErrorKind,
) -> WalletEngineProtectedSecretHostErrorKind {
    match value {
        ProtectedSecretHostErrorKind::NotFound => {
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_NOT_FOUND
        }
        ProtectedSecretHostErrorKind::AuthenticationFailed => {
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_AUTHENTICATION_FAILED
        }
        ProtectedSecretHostErrorKind::Cancelled => {
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_CANCELLED
        }
        ProtectedSecretHostErrorKind::Unavailable => {
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE
        }
        ProtectedSecretHostErrorKind::PolicyViolation => {
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION
        }
        ProtectedSecretHostErrorKind::Other => WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER,
    }
}

/// A borrowed protected-storage reference.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WalletEngineProtectedSecretRefView {
    /// Host-defined storage key. It never contains secret bytes.
    pub value: WalletEngineStringView,
}

impl From<&ProtectedSecretRef> for WalletEngineProtectedSecretRefView {
    fn from(value: &ProtectedSecretRef) -> Self {
        Self {
            value: WalletEngineStringView::from(value.value.as_str()),
        }
    }
}

/// A borrowed request to store secret bytes in platform protected storage.
///
/// Every nested view remains valid only for the duration of the host callback.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WalletEngineProtectedSecretStoreView {
    /// Host-defined storage reference for the mnemonic.
    pub secret_ref: WalletEngineProtectedSecretRefView,
    /// Mnemonic bytes. The host must never log them.
    pub bytes: WalletEngineBytesView,
    /// Whether later reads require user presence or device authentication.
    pub require_user_presence: bool,
}

impl From<&ProtectedSecretStore> for WalletEngineProtectedSecretStoreView {
    fn from(value: &ProtectedSecretStore) -> Self {
        Self {
            secret_ref: WalletEngineProtectedSecretRefView::from(&value.secret_ref),
            bytes: WalletEngineBytesView::from(value.bytes.as_slice()),
            require_user_presence: value.require_user_presence,
        }
    }
}
