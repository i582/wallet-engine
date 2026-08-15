#![allow(unsafe_code)]

use wallet_engine::{
    CreateWalletRequest, CreatedWallet, Network, ProtectedSecretHostErrorKind, ProtectedSecretRef,
    ProtectedSecretStore, RecoveryPhrase, WalletDescriptor, WalletLifecycleError,
};
use wallet_engine_c::{
    WALLET_ENGINE_NETWORK_MAINNET, WALLET_ENGINE_NETWORK_TESTNET,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_AUTHENTICATION_FAILED,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_CANCELLED,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_NOT_FOUND,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION,
    WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE, WalletEngineAbiStatus,
    WalletEngineCreateWalletRequest, WalletEngineProtectedSecretStoreView, WalletEngineStringView,
    WalletEngineWalletLifecycleErrorCode, WalletEngineWalletLifecycleErrorView,
    protected_secret_host_error_kind_from_abi, protected_secret_host_error_kind_to_abi,
    with_created_wallet_view,
};

#[test]
fn create_wallet_request_converts_to_the_core_type() {
    let request = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView::from("wallet-1"),
        network: WALLET_ENGINE_NETWORK_TESTNET,
    };

    // SAFETY: The record-ID view points to a live string literal.
    let core = unsafe { request.try_to_core() };
    assert_eq!(
        core,
        Ok(CreateWalletRequest {
            record_id: "wallet-1".to_owned(),
            network: Network::Testnet,
        })
    );
}

#[test]
fn create_wallet_request_rejects_invalid_boundary_values() {
    let invalid_network = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView::from("wallet-1"),
        network: 2,
    };
    let invalid_utf8 = [0xff];
    let invalid_record_id = WalletEngineCreateWalletRequest {
        record_id: WalletEngineStringView {
            data: invalid_utf8.as_ptr().cast(),
            len: invalid_utf8.len(),
        },
        network: WALLET_ENGINE_NETWORK_MAINNET,
    };

    // SAFETY: The record-ID view points to a live string literal.
    let invalid_network = unsafe { invalid_network.try_to_core() };
    assert_eq!(invalid_network, Err(WalletEngineAbiStatus::InvalidArgument));
    // SAFETY: The record-ID view points to the complete, live `invalid_utf8`
    // array.
    let invalid_record_id = unsafe { invalid_record_id.try_to_core() };
    assert_eq!(invalid_record_id, Err(WalletEngineAbiStatus::InvalidUtf8));
}

#[test]
fn protected_secret_store_view_borrows_the_core_request() {
    let request = ProtectedSecretStore {
        secret_ref: ProtectedSecretRef {
            value: "wallet:wallet-1:mnemonic".to_owned(),
        },
        bytes: b"secret bytes".to_vec(),
        require_user_presence: true,
    };
    let view = WalletEngineProtectedSecretStoreView::from(&request);

    // SAFETY: The nested views borrow live fields of `request`.
    let secret_ref = unsafe { view.secret_ref.value.try_to_string() };
    // SAFETY: The nested views borrow live fields of `request`.
    let bytes = unsafe { view.bytes.try_to_vec() };
    assert_eq!(secret_ref.as_deref(), Ok("wallet:wallet-1:mnemonic"));
    assert_eq!(bytes.as_deref(), Ok(b"secret bytes".as_slice()));
    assert!(view.require_user_presence);
}

#[test]
fn created_wallet_view_borrows_descriptor_and_words_for_the_callback() {
    let wallet = CreatedWallet {
        descriptor: WalletDescriptor {
            record_id: "wallet-1".to_owned(),
            address: "UQExampleAddress".to_owned(),
            public_key: vec![7; 32],
            network: Network::Testnet,
            secret_ref: ProtectedSecretRef {
                value: "wallet:wallet-1:mnemonic".to_owned(),
            },
        },
        recovery_phrase: RecoveryPhrase {
            phrase: (1..=24)
                .map(|index| format!("word{index}"))
                .collect::<Vec<_>>()
                .join(" "),
        },
    };

    with_created_wallet_view(&wallet, |view| {
        // SAFETY: All nested views are valid for this callback invocation.
        let record_id = unsafe { view.descriptor.record_id.try_to_string() };
        // SAFETY: All nested views are valid for this callback invocation.
        let address = unsafe { view.descriptor.address.try_to_string() };
        // SAFETY: All nested views are valid for this callback invocation.
        let public_key = unsafe { view.descriptor.public_key.try_to_vec() };
        // SAFETY: All nested views are valid for this callback invocation.
        let secret_ref = unsafe { view.descriptor.secret_ref.value.try_to_string() };
        assert_eq!(record_id.as_deref(), Ok("wallet-1"));
        assert_eq!(address.as_deref(), Ok("UQExampleAddress"));
        assert_eq!(public_key.as_deref(), Ok([7; 32].as_slice()));
        assert_eq!(view.descriptor.network, WALLET_ENGINE_NETWORK_TESTNET);
        assert_eq!(secret_ref.as_deref(), Ok("wallet:wallet-1:mnemonic"));

        // SAFETY: The phrase view borrows the live `wallet` value.
        let phrase = unsafe { view.recovery_phrase.phrase.try_to_string() };
        assert_eq!(phrase, Ok(wallet.recovery_phrase.phrase.clone()));
    });
}

#[test]
fn protected_secret_host_error_kinds_are_stable_and_validated() {
    let cases = [
        (
            ProtectedSecretHostErrorKind::NotFound,
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_NOT_FOUND,
        ),
        (
            ProtectedSecretHostErrorKind::AuthenticationFailed,
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_AUTHENTICATION_FAILED,
        ),
        (
            ProtectedSecretHostErrorKind::Cancelled,
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_CANCELLED,
        ),
        (
            ProtectedSecretHostErrorKind::Unavailable,
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE,
        ),
        (
            ProtectedSecretHostErrorKind::PolicyViolation,
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_POLICY_VIOLATION,
        ),
        (
            ProtectedSecretHostErrorKind::Other,
            WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_OTHER,
        ),
    ];

    for (core, abi) in cases {
        assert_eq!(protected_secret_host_error_kind_to_abi(core), abi);
        assert_eq!(protected_secret_host_error_kind_from_abi(abi), Ok(core));
    }
    assert_eq!(
        protected_secret_host_error_kind_from_abi(6),
        Err(WalletEngineAbiStatus::InvalidArgument)
    );
}

#[test]
fn wallet_lifecycle_error_codes_and_views_are_stable() {
    let cases = [
        (
            WalletLifecycleError::InvalidRecordId,
            WalletEngineWalletLifecycleErrorCode::InvalidRecordId,
        ),
        (
            WalletLifecycleError::InvalidRecoveryPhrase,
            WalletEngineWalletLifecycleErrorCode::InvalidRecoveryPhrase,
        ),
        (
            WalletLifecycleError::AddressDerivationFailed,
            WalletEngineWalletLifecycleErrorCode::AddressDerivationFailed,
        ),
        (
            WalletLifecycleError::SecretWalletMismatch,
            WalletEngineWalletLifecycleErrorCode::SecretWalletMismatch,
        ),
    ];

    assert_eq!(
        WalletEngineWalletLifecycleErrorCode::InvalidRecordId as u32,
        0
    );
    assert_eq!(
        WalletEngineWalletLifecycleErrorCode::InvalidRecoveryPhrase as u32,
        1
    );
    assert_eq!(
        WalletEngineWalletLifecycleErrorCode::AddressDerivationFailed as u32,
        2
    );
    assert_eq!(
        WalletEngineWalletLifecycleErrorCode::SecretWalletMismatch as u32,
        3
    );
    assert_eq!(
        WalletEngineWalletLifecycleErrorCode::ProtectedSecretHost as u32,
        4
    );

    for (error, expected_code) in cases {
        let view = WalletEngineWalletLifecycleErrorView::from(&error);
        assert_eq!(view.code, expected_code);
        assert!(!view.has_protected_secret_host_error_kind);
        // SAFETY: The empty diagnostic does not dereference its pointer.
        let diagnostic = unsafe { view.diagnostic.try_to_string() };
        assert_eq!(diagnostic.as_deref(), Ok(""));
    }
}

#[test]
fn protected_secret_lifecycle_error_preserves_details() {
    let error = WalletLifecycleError::ProtectedSecretHost {
        kind: ProtectedSecretHostErrorKind::Unavailable,
        diagnostic: "keychain unavailable".to_owned(),
    };
    let view = WalletEngineWalletLifecycleErrorView::from(&error);

    assert_eq!(
        view.code,
        WalletEngineWalletLifecycleErrorCode::ProtectedSecretHost
    );
    assert!(view.has_protected_secret_host_error_kind);
    assert_eq!(
        view.protected_secret_host_error_kind,
        WALLET_ENGINE_PROTECTED_SECRET_HOST_ERROR_KIND_UNAVAILABLE
    );
    // SAFETY: The diagnostic view borrows the live `error` value.
    let diagnostic = unsafe { view.diagnostic.try_to_string() };
    assert_eq!(diagnostic.as_deref(), Ok("keychain unavailable"));
}
