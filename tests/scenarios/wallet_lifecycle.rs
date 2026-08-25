use wallet_engine::KeyRotationMessageKind;
use wallet_engine::{Network, ProtectedSecretHostErrorKind, WalletLifecycleError};

use crate::support::*;

#[test]
fn create_reveal_and_delete_round_trip() {
    wallet_lifecycle_scenario("a created wallet survives the complete secret lifecycle")
        .when(create_wallet("create", "created-wallet", Network::Testnet))
        .then(descriptor_is("create", "created-wallet", Network::Testnet))
        .then(phrase_has_words("create", 12))
        .then(protected_secret_is_stored("create"))
        .when(reveal_wallet("reveal", "create"))
        .then(phrases_match("create", "reveal"))
        .then(protected_secret_was_revealed("create"))
        .when(delete_wallet("delete", "create"))
        .then(lifecycle_succeeds("delete"))
        .then(protected_secret_is_deleted("create"))
        .run();
}

#[test]
fn importing_known_words_derives_the_expected_testnet_wallet() {
    let fixture = test_wallet();
    let words = fixture.recovery_words();

    wallet_lifecycle_scenario("known recovery words derive a stable wallet address")
        .when(import_wallet(
            "import",
            "imported-wallet",
            Network::Testnet,
            words.clone(),
        ))
        .then(descriptor_is("import", "imported-wallet", Network::Testnet))
        .then(descriptor_address_is("import", fixture.testnet_address()))
        .then(protected_secret_is_stored("import"))
        .when(reveal_wallet("reveal", "import"))
        .then(phrase_is("reveal", words))
        .then(protected_secret_was_revealed("import"))
        .run();
}

#[test]
fn key_rotation_preparation_returns_new_material_without_replacing_storage() {
    let fixture = test_wallet();

    wallet_lifecycle_scenario("key rotation preparation creates one-shot material")
        .when(import_wallet(
            "import",
            "rotation-wallet",
            Network::Testnet,
            fixture.recovery_words(),
        ))
        .when(prepare_key_rotation(
            "rotation",
            "import",
            KeyRotationMessageKind::External,
        ))
        .then(key_rotation_material_is(
            "rotation",
            KeyRotationMessageKind::External,
        ))
        .then(protected_secret_was_read_for_key_rotation("import"))
        .when(reveal_wallet("reveal", "import"))
        .then(phrase_has_words("reveal", 12))
        .run();
}

#[test]
fn key_rotation_preparation_rejects_an_already_rotated_phrase() {
    let words = "notice tortoise soup strong gun divide offer process salon siren general carry clump left year void clutch tool case burden fix income champion lounge"
        .split_whitespace()
        .map(str::to_owned)
        .collect();

    wallet_lifecycle_scenario("a distinct signing half cannot rotate twice")
        .when(import_wallet(
            "import",
            "rotated-wallet",
            Network::Testnet,
            words,
        ))
        .when(prepare_key_rotation(
            "rotation",
            "import",
            KeyRotationMessageKind::Internal,
        ))
        .then(lifecycle_error(
            "rotation",
            WalletLifecycleError::SigningKeyAlreadyRotated,
        ))
        .run();
}

#[test]
fn network_changes_the_wallet_contract_address() {
    let words = test_wallet().recovery_words();

    wallet_lifecycle_scenario("the same key uses a network-specific wallet id")
        .when(import_wallet(
            "testnet",
            "testnet-wallet",
            Network::Testnet,
            words.clone(),
        ))
        .when(import_wallet(
            "mainnet",
            "mainnet-wallet",
            Network::Mainnet,
            words,
        ))
        .then(descriptor_is("testnet", "testnet-wallet", Network::Testnet))
        .then(descriptor_is("mainnet", "mainnet-wallet", Network::Mainnet))
        .then(descriptor_addresses_differ("testnet", "mainnet"))
        .then(protected_secret_is_stored("testnet"))
        .then(protected_secret_is_stored("mainnet"))
        .run();
}

#[test]
fn invalid_recovery_words_never_reach_protected_storage() {
    let mut words = test_wallet().recovery_words();
    words.pop();

    wallet_lifecycle_scenario("an invalid phrase fails before protected storage changes")
        .when(import_wallet(
            "import",
            "invalid-phrase",
            Network::Testnet,
            words,
        ))
        .then(lifecycle_error(
            "import",
            WalletLifecycleError::InvalidRecoveryPhrase,
        ))
        .then(no_protected_secrets_were_stored())
        .run();
}

#[test]
fn invalid_record_id_never_generates_or_stores_a_wallet() {
    wallet_lifecycle_scenario("invalid application identity fails before wallet generation")
        .when(create_wallet(
            "create",
            "invalid record id",
            Network::Testnet,
        ))
        .then(lifecycle_error(
            "create",
            WalletLifecycleError::InvalidRecordId,
        ))
        .then(no_protected_secrets_were_stored())
        .run();
}

#[test]
fn reveal_rejects_a_valid_secret_from_another_wallet() {
    wallet_lifecycle_scenario("protected storage cannot substitute another wallet's phrase")
        .when(import_wallet(
            "expected",
            "expected-wallet",
            Network::Testnet,
            test_wallet().recovery_words(),
        ))
        .when(create_wallet("other", "other-wallet", Network::Testnet))
        // Simulate a host-storage mix-up with a valid phrase. Parsing succeeds,
        // so only the derived-address check can prevent revealing the wrong key.
        .when(replace_protected_secret("expected", "other"))
        .when(reveal_wallet("reveal", "expected"))
        .then(lifecycle_error(
            "reveal",
            WalletLifecycleError::SecretWalletMismatch,
        ))
        .then(protected_secret_was_revealed("expected"))
        .run();
}

#[test]
fn protected_storage_failure_during_import_leaves_no_wallet_secret() {
    wallet_lifecycle_scenario("import publishes the exact protected-storage failure")
        .when(fail_next_protected_secret_store())
        .when(import_wallet(
            "import",
            "store-failure",
            Network::Testnet,
            test_wallet().recovery_words(),
        ))
        .then(lifecycle_error(
            "import",
            WalletLifecycleError::ProtectedSecretHost {
                kind: ProtectedSecretHostErrorKind::Other,
                diagnostic: "scripted protected secret store failure".to_owned(),
            },
        ))
        .then(no_protected_secrets_were_stored())
        .run();
}

#[test]
fn protected_storage_failure_during_reveal_preserves_the_secret() {
    wallet_lifecycle_scenario("reveal reports authorization storage failure without deleting data")
        .when(import_wallet(
            "import",
            "read-failure",
            Network::Testnet,
            test_wallet().recovery_words(),
        ))
        .when(fail_next_protected_secret_read())
        .when(reveal_wallet("reveal", "import"))
        .then(lifecycle_error(
            "reveal",
            WalletLifecycleError::ProtectedSecretHost {
                kind: ProtectedSecretHostErrorKind::Other,
                diagnostic: "scripted protected secret failure".to_owned(),
            },
        ))
        .then(protected_secret_is_stored("import"))
        .run();
}

#[test]
fn protected_storage_failure_during_delete_preserves_application_recovery() {
    wallet_lifecycle_scenario("metadata can be retained when protected-secret deletion fails")
        .when(import_wallet(
            "import",
            "delete-failure",
            Network::Testnet,
            test_wallet().recovery_words(),
        ))
        .when(fail_next_protected_secret_delete())
        .when(delete_wallet("delete", "import"))
        .then(lifecycle_error(
            "delete",
            WalletLifecycleError::ProtectedSecretHost {
                kind: ProtectedSecretHostErrorKind::Other,
                diagnostic: "scripted protected secret delete failure".to_owned(),
            },
        ))
        .then(protected_secret_is_stored("import"))
        .run();
}
