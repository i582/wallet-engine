//! Synchronous lifecycle bridge used only by the C ABI crate.

use crate::wallet::crypto::generate_mnemonic;
use crate::{
    CreateWalletRequest, CreatedWallet, ImportWalletRequest, ProtectedSecretStore,
    WalletDescriptor, WalletLifecycleError,
    wallet::{crypto::SensitiveMnemonic, derive_descriptor, recovery_phrase, validate_record_id},
};

/// Failure produced before or during a synchronous protected-storage call.
#[derive(Debug)]
pub enum WalletLifecycleCallError<E> {
    /// Wallet validation, mnemonic, or derivation failed.
    Wallet(WalletLifecycleError),
    /// The embedding application's synchronous storage callback failed.
    Store(E),
}

impl<E> From<WalletLifecycleError> for WalletLifecycleCallError<E> {
    fn from(value: WalletLifecycleError) -> Self {
        Self::Wallet(value)
    }
}

/// Creates a wallet and stores its secret synchronously through `store`.
pub fn create_wallet<E>(
    request: CreateWalletRequest,
    store: impl FnOnce(ProtectedSecretStore) -> Result<(), E>,
) -> Result<CreatedWallet, WalletLifecycleCallError<E>> {
    validate_record_id(&request.record_id)?;

    let secret = generate_mnemonic().map_err(|_| WalletLifecycleError::InvalidRecoveryPhrase)?;
    let descriptor = derive_descriptor(&request.record_id, request.network, &secret)?;
    store(ProtectedSecretStore {
        secret_ref: descriptor.secret_ref.clone(),
        bytes: secret.as_bytes().to_vec(),
        require_user_presence: true,
    })
    .map_err(WalletLifecycleCallError::Store)?;
    let recovery_phrase = recovery_phrase(&secret)?;

    Ok(CreatedWallet {
        descriptor,
        recovery_phrase,
    })
}

/// Imports a wallet and stores its secret synchronously through `store`.
pub fn import_wallet<E>(
    request: ImportWalletRequest,
    store: impl FnOnce(ProtectedSecretStore) -> Result<(), E>,
) -> Result<WalletDescriptor, WalletLifecycleCallError<E>> {
    validate_record_id(&request.record_id)?;

    let secret = SensitiveMnemonic::from_words(request.recovery_words)
        .map_err(|_| WalletLifecycleError::InvalidRecoveryPhrase)?;
    let descriptor = derive_descriptor(&request.record_id, request.network, &secret)?;
    store(ProtectedSecretStore {
        secret_ref: descriptor.secret_ref.clone(),
        bytes: secret.as_bytes().to_vec(),
        require_user_presence: true,
    })
    .map_err(WalletLifecycleCallError::Store)?;

    Ok(descriptor)
}
