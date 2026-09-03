use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use futures::executor::block_on;
use wallet_engine::{
    CreateWalletRequest, CreatedWallet, ImportWalletRequest, KeyRotationMessageKind, Network,
    NonEmptyString, PrepareKeyRotationRequest, PreparedKeyRotation, ProviderConfig, RecoveryPhrase,
    SecretAccessReason, SendAmount, SendBocRequest, SendExpiration, SendIntent, SendMessage,
    SendMessageBody, SendPhase, SendRequest, UnsignedDecimalString, WalletClient,
    WalletClientConfig, WalletClientError, WalletDescriptor, WalletLifecycle, WalletLifecycleError,
};

use super::host::{MemoryPlatformHost, RequestKind, ScenarioHttpHost};
use super::localnet::LocalnetHttpHost;
use super::scenario::wallet;
use super::test_wallet::test_wallet;

pub(crate) fn wallet_lifecycle_scenario(name: impl Into<String>) -> WalletLifecycleScenario {
    WalletLifecycleScenario {
        name: name.into(),
        steps: Vec::new(),
    }
}

pub(crate) fn execute_repeated_key_rotation_on_localnet() -> Result<(), String> {
    let platform_host = Arc::new(MemoryPlatformHost::default());
    let lifecycle = WalletLifecycle::new(platform_host.clone());
    let fixture = test_wallet();
    let initial_descriptor = block_on(lifecycle.import_wallet(ImportWalletRequest {
        record_id: "localnet-key-rotation-initial".to_owned(),
        network: Network::Testnet,
        recovery_words: fixture.recovery_words(),
    }))
    .map_err(|error| error.to_string())?;
    let wallet_address = initial_descriptor.address.clone();
    let localnet = Arc::new(LocalnetHttpHost::start(
        wallet_address.as_str(),
        "5000000000",
    )?);

    localnet.spam_transfers(1)?;
    let first_client =
        localnet_wallet_client(initial_descriptor, localnet.clone(), platform_host.clone())?;
    let first = block_on(
        first_client.prepare_key_rotation(PrepareKeyRotationRequest {
            valid_until: u64::from(u32::MAX),
            message_kind: KeyRotationMessageKind::External,
        }),
    )
    .map_err(|error| error.to_string())?;
    if first.seqno != 1 {
        return Err(format!(
            "expected first provider seqno 1, got {}",
            first.seqno
        ));
    }
    let first_request = SendBocRequest {
        operation_id: NonEmptyString::try_from("localnet-key-rotation-first".to_owned())
            .map_err(|error| error.to_string())?,
        force: false,
        signed_boc: first.signed_boc.clone(),
        seqno: first.seqno,
        valid_until: first.valid_until,
    };
    let first_preview = block_on(first_client.preview_send_boc(first_request.clone()))
        .map_err(|error| error.to_string())?;
    if first_preview.message_boc_base64 != first.signed_boc {
        return Err("prepared rotation preview changed the signed BOC".to_owned());
    }
    if first_preview.valid_until != first.valid_until {
        return Err("prepared rotation preview changed the expiration".to_owned());
    }
    if !first_preview.messages.is_empty() {
        return Err("prepared rotation preview unexpectedly exposed decoded messages".to_owned());
    }
    let first_send =
        block_on(first_client.send_boc(first_request)).map_err(|error| error.to_string())?;
    if first_send.phase != SendPhase::Submitted {
        return Err(format!(
            "expected first rotation submission, got {:?}",
            first_send.phase
        ));
    }
    localnet.wait_for_seqno(2)?;
    let first_resolution =
        block_on(first_client.resolve_pending()).map_err(|error| error.to_string())?;
    if first_resolution.phase != SendPhase::Confirmed {
        return Err(format!(
            "expected first rotation confirmation, got {:?}",
            first_resolution.phase
        ));
    }
    assert_localnet_public_key(&localnet, &first.new_public_key, "first")?;

    let second_descriptor = block_on(
        lifecycle.import_wallet(ImportWalletRequest {
            record_id: "localnet-key-rotation-second".to_owned(),
            network: Network::Testnet,
            recovery_words: first
                .replacement_recovery_phrase
                .phrase
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect(),
        }),
    )
    .map_err(|error| error.to_string())?;
    if second_descriptor.address != wallet_address {
        return Err(format!(
            "re-importing the first replacement phrase changed the wallet address from {} to {}",
            wallet_address.as_str(),
            second_descriptor.address.as_str()
        ));
    }

    let second_client = localnet_wallet_client(second_descriptor, localnet.clone(), platform_host)?;
    let second = block_on(
        second_client.prepare_key_rotation(PrepareKeyRotationRequest {
            valid_until: u64::from(u32::MAX),
            message_kind: KeyRotationMessageKind::External,
        }),
    )
    .map_err(|error| error.to_string())?;
    if second.seqno != 2 {
        return Err(format!(
            "expected second provider seqno 2, got {}",
            second.seqno
        ));
    }
    if second.new_public_key == first.new_public_key {
        return Err("the second rotation reused the current signing key".to_owned());
    }
    let second_send = block_on(
        second_client.send_boc(SendBocRequest {
            operation_id: NonEmptyString::try_from("localnet-key-rotation-second".to_owned())
                .map_err(|error| error.to_string())?,
            force: false,
            signed_boc: second.signed_boc.clone(),
            seqno: second.seqno,
            valid_until: second.valid_until,
        }),
    )
    .map_err(|error| error.to_string())?;
    if second_send.phase != SendPhase::Submitted {
        return Err(format!(
            "expected second rotation submission, got {:?}",
            second_send.phase
        ));
    }
    localnet.wait_for_seqno(3)?;
    let second_resolution =
        block_on(second_client.resolve_pending()).map_err(|error| error.to_string())?;
    if second_resolution.phase != SendPhase::Confirmed {
        return Err(format!(
            "expected second rotation confirmation, got {:?}",
            second_resolution.phase
        ));
    }
    assert_localnet_public_key(&localnet, &second.new_public_key, "second")?;

    Ok(())
}

pub(crate) fn execute_uninitialized_key_rotation_deploys_with_zero_seqno_on_localnet()
-> Result<(), String> {
    let platform_host = Arc::new(MemoryPlatformHost::default());
    let lifecycle = WalletLifecycle::new(platform_host.clone());
    let descriptor = block_on(lifecycle.import_wallet(ImportWalletRequest {
        record_id: "localnet-uninitialized-key-rotation".to_owned(),
        network: Network::Testnet,
        recovery_words: test_wallet().recovery_words(),
    }))
    .map_err(|error| error.to_string())?;
    let localnet = Arc::new(LocalnetHttpHost::start(
        descriptor.address.as_str(),
        "5000000000",
    )?);
    let client = localnet_wallet_client(descriptor, localnet.clone(), platform_host)?;

    let prepared = prepare_external_rotation(&client)?;
    if prepared.seqno != 0 {
        return Err(format!(
            "expected uninitialized wallet seqno 0, got {}",
            prepared.seqno
        ));
    }

    let submitted = block_on(client.send_boc(key_rotation_send_request(
        "localnet-uninitialized-key-rotation",
        &prepared,
    )?))
    .map_err(|error| error.to_string())?;
    if submitted.phase != SendPhase::Submitted {
        return Err(format!(
            "expected uninitialized rotation submission, got {:?}",
            submitted.phase
        ));
    }
    localnet.wait_for_seqno(1)?;
    let resolution = block_on(client.resolve_pending()).map_err(|error| error.to_string())?;
    if resolution.phase != SendPhase::Confirmed {
        return Err(format!(
            "expected uninitialized rotation confirmation, got {:?}",
            resolution.phase
        ));
    }
    assert_localnet_public_key(&localnet, &prepared.new_public_key, "initial deployment")?;

    Ok(())
}

pub(crate) fn execute_key_rotation_confirmation_after_restart_on_localnet() -> Result<(), String> {
    let LocalnetKeyRotationFixture {
        platform_host,
        descriptor,
        localnet,
        client,
    } = localnet_key_rotation_fixture("localnet-key-rotation-restart")?;
    let prepared = prepare_external_rotation(&client)?;
    let request = key_rotation_send_request("localnet-key-rotation-restart", &prepared)?;

    let submitted = block_on(client.send_boc(request)).map_err(|error| error.to_string())?;
    if submitted.phase != SendPhase::Submitted {
        return Err(format!(
            "expected rotation submission before restart, got {:?}",
            submitted.phase
        ));
    }
    drop(client);

    localnet.wait_for_seqno(prepared.seqno + 1)?;
    let restarted = localnet_wallet_client(descriptor, localnet.clone(), platform_host)?;
    let resolution = block_on(restarted.resolve_pending()).map_err(|error| error.to_string())?;
    if resolution.phase != SendPhase::Confirmed {
        return Err(format!(
            "expected restarted client to confirm the journaled rotation, got {:?}",
            resolution.phase
        ));
    }
    assert_localnet_public_key(&localnet, &prepared.new_public_key, "restarted")
}

pub(crate) fn execute_stale_key_rotation_rejection_on_localnet() -> Result<(), String> {
    let LocalnetKeyRotationFixture {
        localnet, client, ..
    } = localnet_key_rotation_fixture("localnet-key-rotation-stale")?;
    let stale = prepare_external_rotation(&client)?;
    localnet.spam_transfers(1)?;
    localnet.wait_for_seqno(stale.seqno + 1)?;

    let error = block_on(client.send_boc(key_rotation_send_request(
        "localnet-key-rotation-stale",
        &stale,
    )?))
    .expect_err("a stale prepared key rotation must not be submitted");
    let WalletClientError::SendFailed { diagnostic } = error else {
        return Err(format!("expected a stale-seqno send failure, got {error}"));
    };
    if !diagnostic.contains("does not match current wallet seqno") {
        return Err(format!(
            "expected a stale-seqno diagnostic, got {diagnostic}"
        ));
    }
    if localnet.submitted_boc().is_some() {
        return Err("the stale key-rotation BOC reached sendBoc".to_owned());
    }

    let fresh = prepare_external_rotation(&client)?;
    if fresh.seqno != stale.seqno + 1 {
        return Err(format!(
            "expected fresh rotation seqno {}, got {}",
            stale.seqno + 1,
            fresh.seqno
        ));
    }
    let submitted = block_on(client.send_boc(key_rotation_send_request(
        "localnet-key-rotation-fresh-after-stale",
        &fresh,
    )?))
    .map_err(|error| error.to_string())?;
    if submitted.phase != SendPhase::Submitted {
        return Err(format!(
            "expected fresh rotation submission after stale rejection, got {:?}",
            submitted.phase
        ));
    }
    localnet.wait_for_seqno(fresh.seqno + 1)?;
    let resolution = block_on(client.resolve_pending()).map_err(|error| error.to_string())?;
    if resolution.phase != SendPhase::Confirmed {
        return Err(format!(
            "expected fresh rotation confirmation after stale rejection, got {:?}",
            resolution.phase
        ));
    }
    assert_localnet_public_key(&localnet, &fresh.new_public_key, "fresh after stale")
}

pub(crate) fn execute_key_rotation_shares_send_slot_on_localnet() -> Result<(), String> {
    let LocalnetKeyRotationFixture {
        descriptor,
        localnet,
        client,
        ..
    } = localnet_key_rotation_fixture("localnet-key-rotation-single-flight")?;
    let prepared = prepare_external_rotation(&client)?;
    let request = key_rotation_send_request("localnet-key-rotation-paused", &prepared)?;

    localnet.pause_next_request("rotation-submit".to_owned(), RequestKind::Submission);
    let send_client = client.clone();
    let send_thread = thread::spawn(move || block_on(send_client.send_boc(request)));
    localnet.wait_for_request("rotation-submit")?;

    let ordinary_send = SendRequest {
        operation_id: NonEmptyString::try_from("ordinary-send-during-rotation".to_owned())
            .map_err(|error| error.to_string())?,
        force: false,
        intent: SendIntent {
            expiration: SendExpiration::EngineDefault,
            messages: vec![SendMessage {
                destination: descriptor.address,
                amount: SendAmount::Exact {
                    nanograms: UnsignedDecimalString::try_from("1".to_owned())
                        .map_err(|error| error.to_string())?,
                },
                body: SendMessageBody::Empty,
                bounce: false,
                state_init: None,
            }],
        },
    };
    let ordinary_result = block_on(client.send(ordinary_send));

    localnet.release_request("rotation-submit")?;
    let submitted = send_thread
        .join()
        .map_err(|_| "the paused key-rotation send thread panicked".to_owned())?
        .map_err(|error| error.to_string())?;
    let error = ordinary_result.expect_err("ordinary send must share the active sendBoc slot");
    if error != WalletClientError::SendAlreadyInProgress {
        return Err(format!("expected shared send slot rejection, got {error}"));
    }
    if submitted.phase != SendPhase::Submitted {
        return Err(format!(
            "expected paused rotation to submit after release, got {:?}",
            submitted.phase
        ));
    }
    localnet.wait_for_seqno(prepared.seqno + 1)?;
    let resolution = block_on(client.resolve_pending()).map_err(|error| error.to_string())?;
    if resolution.phase != SendPhase::Confirmed {
        return Err(format!(
            "expected paused rotation confirmation, got {:?}",
            resolution.phase
        ));
    }
    assert_localnet_public_key(&localnet, &prepared.new_public_key, "single-flight")
}

struct LocalnetKeyRotationFixture {
    platform_host: Arc<MemoryPlatformHost>,
    descriptor: WalletDescriptor,
    localnet: Arc<LocalnetHttpHost>,
    client: Arc<WalletClient>,
}

fn localnet_key_rotation_fixture(record_id: &str) -> Result<LocalnetKeyRotationFixture, String> {
    let platform_host = Arc::new(MemoryPlatformHost::default());
    let lifecycle = WalletLifecycle::new(platform_host.clone());
    let descriptor = block_on(lifecycle.import_wallet(ImportWalletRequest {
        record_id: record_id.to_owned(),
        network: Network::Testnet,
        recovery_words: test_wallet().recovery_words(),
    }))
    .map_err(|error| error.to_string())?;
    let localnet = Arc::new(LocalnetHttpHost::start(
        descriptor.address.as_str(),
        "5000000000",
    )?);
    localnet.spam_transfers(1)?;
    let client =
        localnet_wallet_client(descriptor.clone(), localnet.clone(), platform_host.clone())?;
    Ok(LocalnetKeyRotationFixture {
        platform_host,
        descriptor,
        localnet,
        client,
    })
}

fn prepare_external_rotation(client: &WalletClient) -> Result<PreparedKeyRotation, String> {
    block_on(client.prepare_key_rotation(PrepareKeyRotationRequest {
        valid_until: u64::from(u32::MAX),
        message_kind: KeyRotationMessageKind::External,
    }))
    .map_err(|error| error.to_string())
}

fn key_rotation_send_request(
    operation_id: &str,
    prepared: &PreparedKeyRotation,
) -> Result<SendBocRequest, String> {
    Ok(SendBocRequest {
        operation_id: NonEmptyString::try_from(operation_id.to_owned())
            .map_err(|error| error.to_string())?,
        force: false,
        signed_boc: prepared.signed_boc.clone(),
        seqno: prepared.seqno,
        valid_until: prepared.valid_until,
    })
}

fn localnet_wallet_client(
    descriptor: WalletDescriptor,
    localnet: Arc<LocalnetHttpHost>,
    platform_host: Arc<MemoryPlatformHost>,
) -> Result<Arc<WalletClient>, String> {
    let record_id = NonEmptyString::try_from(descriptor.record_id.as_str())
        .map_err(|error| error.to_string())?;
    WalletClient::new(
        WalletClientConfig {
            record_id,
            address: descriptor.address,
            public_key: descriptor.public_key,
            local_secret_ref: Some(descriptor.secret_ref),
            network: descriptor.network,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig {
                toncenter_base_url: localnet.provider_base_url(),
                dns_root_address: None,
                request_timeout_ms: 15_000,
            },
        },
        localnet,
        platform_host,
    )
    .map_err(|error| error.to_string())
}

fn assert_localnet_public_key(
    localnet: &LocalnetHttpHost,
    expected_public_key: &[u8],
    rotation: &str,
) -> Result<(), String> {
    let expected_public_key = expected_public_key
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let actual_public_key = localnet.public_key_hex()?;
    if actual_public_key != expected_public_key {
        return Err(format!(
            "expected {rotation} rotated public key {expected_public_key}, got {actual_public_key}"
        ));
    }

    Ok(())
}

pub(crate) fn create_wallet(
    operation: impl Into<String>,
    record_id: impl Into<String>,
    network: Network,
) -> LifecycleAction {
    LifecycleAction::Create {
        operation: operation.into(),
        request: CreateWalletRequest {
            record_id: record_id.into(),
            network,
        },
    }
}

pub(crate) fn import_wallet(
    operation: impl Into<String>,
    record_id: impl Into<String>,
    network: Network,
    recovery_words: Vec<String>,
) -> LifecycleAction {
    LifecycleAction::Import {
        operation: operation.into(),
        request: ImportWalletRequest {
            record_id: record_id.into(),
            network,
            recovery_words,
        },
    }
}

pub(crate) fn reveal_wallet(
    operation: impl Into<String>,
    descriptor_from: impl Into<String>,
) -> LifecycleAction {
    LifecycleAction::Reveal {
        operation: operation.into(),
        descriptor_from: descriptor_from.into(),
    }
}

pub(crate) fn delete_wallet(
    operation: impl Into<String>,
    descriptor_from: impl Into<String>,
) -> LifecycleAction {
    LifecycleAction::Delete {
        operation: operation.into(),
        descriptor_from: descriptor_from.into(),
    }
}

pub(crate) fn prepare_key_rotation(
    operation: impl Into<String>,
    descriptor_from: impl Into<String>,
    message_kind: KeyRotationMessageKind,
) -> LifecycleAction {
    LifecycleAction::PrepareKeyRotation {
        operation: operation.into(),
        descriptor_from: descriptor_from.into(),
        message_kind,
    }
}

pub(crate) fn replace_protected_secret(
    target_descriptor: impl Into<String>,
    source_descriptor: impl Into<String>,
) -> LifecycleAction {
    LifecycleAction::ReplaceProtectedSecret {
        target_descriptor: target_descriptor.into(),
        source_descriptor: source_descriptor.into(),
    }
}

pub(crate) const fn fail_next_protected_secret_store() -> LifecycleAction {
    LifecycleAction::FailNextProtectedSecretStore
}

pub(crate) const fn fail_next_protected_secret_read() -> LifecycleAction {
    LifecycleAction::FailNextProtectedSecretRead
}

pub(crate) const fn fail_next_protected_secret_delete() -> LifecycleAction {
    LifecycleAction::FailNextProtectedSecretDelete
}

pub(crate) fn descriptor_is(
    operation: impl Into<String>,
    record_id: impl Into<String>,
    network: Network,
) -> LifecycleExpectation {
    LifecycleExpectation::DescriptorIs {
        operation: operation.into(),
        record_id: record_id.into(),
        network,
    }
}

pub(crate) fn descriptor_address_is(
    operation: impl Into<String>,
    address: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::DescriptorAddressIs {
        operation: operation.into(),
        address: address.into(),
    }
}

pub(crate) fn descriptor_addresses_differ(
    left: impl Into<String>,
    right: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::DescriptorAddressesDiffer {
        left: left.into(),
        right: right.into(),
    }
}

pub(crate) fn phrase_has_words(operation: impl Into<String>, count: usize) -> LifecycleExpectation {
    LifecycleExpectation::PhraseHasWords {
        operation: operation.into(),
        count,
    }
}

pub(crate) fn phrase_is(operation: impl Into<String>, words: Vec<String>) -> LifecycleExpectation {
    LifecycleExpectation::PhraseIs {
        operation: operation.into(),
        words,
    }
}

pub(crate) fn phrases_match(
    left: impl Into<String>,
    right: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::PhrasesMatch {
        left: left.into(),
        right: right.into(),
    }
}

pub(crate) fn protected_secret_is_stored(
    descriptor_from: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::ProtectedSecretIsStored {
        descriptor_from: descriptor_from.into(),
    }
}

pub(crate) fn protected_secret_was_revealed(
    descriptor_from: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::ProtectedSecretWasRevealed {
        descriptor_from: descriptor_from.into(),
    }
}

pub(crate) fn key_rotation_material_is(
    operation: impl Into<String>,
    message_kind: KeyRotationMessageKind,
) -> LifecycleExpectation {
    LifecycleExpectation::KeyRotationMaterialIs {
        operation: operation.into(),
        message_kind,
    }
}

pub(crate) fn protected_secret_was_read_for_key_rotation(
    descriptor_from: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::ProtectedSecretWasReadForKeyRotation {
        descriptor_from: descriptor_from.into(),
    }
}

pub(crate) fn protected_secret_is_deleted(
    descriptor_from: impl Into<String>,
) -> LifecycleExpectation {
    LifecycleExpectation::ProtectedSecretIsDeleted {
        descriptor_from: descriptor_from.into(),
    }
}

pub(crate) const fn no_protected_secrets_were_stored() -> LifecycleExpectation {
    LifecycleExpectation::StoredSecretCount(0)
}

pub(crate) fn lifecycle_succeeds(operation: impl Into<String>) -> LifecycleExpectation {
    LifecycleExpectation::Success {
        operation: operation.into(),
    }
}

pub(crate) fn lifecycle_error(
    operation: impl Into<String>,
    expected: WalletLifecycleError,
) -> LifecycleExpectation {
    LifecycleExpectation::Error {
        operation: operation.into(),
        expected,
    }
}

pub(crate) enum LifecycleAction {
    Create {
        operation: String,
        request: CreateWalletRequest,
    },
    Import {
        operation: String,
        request: ImportWalletRequest,
    },
    Reveal {
        operation: String,
        descriptor_from: String,
    },
    Delete {
        operation: String,
        descriptor_from: String,
    },
    PrepareKeyRotation {
        operation: String,
        descriptor_from: String,
        message_kind: KeyRotationMessageKind,
    },
    ReplaceProtectedSecret {
        target_descriptor: String,
        source_descriptor: String,
    },
    FailNextProtectedSecretStore,
    FailNextProtectedSecretRead,
    FailNextProtectedSecretDelete,
}

pub(crate) enum LifecycleExpectation {
    DescriptorIs {
        operation: String,
        record_id: String,
        network: Network,
    },
    DescriptorAddressIs {
        operation: String,
        address: String,
    },
    DescriptorAddressesDiffer {
        left: String,
        right: String,
    },
    PhraseHasWords {
        operation: String,
        count: usize,
    },
    PhraseIs {
        operation: String,
        words: Vec<String>,
    },
    PhrasesMatch {
        left: String,
        right: String,
    },
    ProtectedSecretIsStored {
        descriptor_from: String,
    },
    ProtectedSecretWasRevealed {
        descriptor_from: String,
    },
    KeyRotationMaterialIs {
        operation: String,
        message_kind: KeyRotationMessageKind,
    },
    ProtectedSecretWasReadForKeyRotation {
        descriptor_from: String,
    },
    ProtectedSecretIsDeleted {
        descriptor_from: String,
    },
    StoredSecretCount(usize),
    Success {
        operation: String,
    },
    Error {
        operation: String,
        expected: WalletLifecycleError,
    },
}

enum LifecycleStep {
    When(LifecycleAction),
    Then(LifecycleExpectation),
}

pub(crate) struct WalletLifecycleScenario {
    name: String,
    steps: Vec<LifecycleStep>,
}

impl WalletLifecycleScenario {
    #[must_use]
    pub(crate) fn when(mut self, action: LifecycleAction) -> Self {
        self.steps.push(LifecycleStep::When(action));
        self
    }

    #[must_use]
    pub(crate) fn then(mut self, expectation: LifecycleExpectation) -> Self {
        self.steps.push(LifecycleStep::Then(expectation));
        self
    }

    pub(crate) fn run(self) {
        let host = Arc::new(MemoryPlatformHost::default());
        let lifecycle = WalletLifecycle::new(host.clone());
        let mut runner = WalletLifecycleRunner {
            lifecycle,
            host,
            results: HashMap::new(),
        };

        for (index, step) in self.steps.into_iter().enumerate() {
            let result = match step {
                LifecycleStep::When(action) => runner.execute(action),
                LifecycleStep::Then(expectation) => runner.assert(expectation),
            };
            if let Err(message) = result {
                panic!(
                    "scenario: {}\nstep {} failed:\n{}",
                    self.name,
                    index + 1,
                    message
                );
            }
        }
    }
}

enum LifecycleResult {
    Created(Result<CreatedWallet, WalletLifecycleError>),
    Descriptor(Result<WalletDescriptor, WalletLifecycleError>),
    Phrase(Result<RecoveryPhrase, WalletLifecycleError>),
    KeyRotation(Result<PreparedKeyRotation, WalletClientError>),
    Unit(Result<(), WalletLifecycleError>),
}

struct WalletLifecycleRunner {
    lifecycle: Arc<WalletLifecycle>,
    host: Arc<MemoryPlatformHost>,
    results: HashMap<String, LifecycleResult>,
}

impl WalletLifecycleRunner {
    fn execute(&mut self, action: LifecycleAction) -> Result<(), String> {
        let (operation, result) = match action {
            LifecycleAction::Create { operation, request } => {
                let result = block_on(self.lifecycle.create_wallet(request));
                (operation, LifecycleResult::Created(result))
            }
            LifecycleAction::Import { operation, request } => {
                let result = block_on(self.lifecycle.import_wallet(request));
                (operation, LifecycleResult::Descriptor(result))
            }
            LifecycleAction::Reveal {
                operation,
                descriptor_from,
            } => {
                let descriptor = self.descriptor(&descriptor_from)?.clone();
                let result = block_on(self.lifecycle.reveal_recovery_phrase(descriptor));
                (operation, LifecycleResult::Phrase(result))
            }
            LifecycleAction::Delete {
                operation,
                descriptor_from,
            } => {
                let descriptor = self.descriptor(&descriptor_from)?.clone();
                let result = block_on(self.lifecycle.delete_wallet(descriptor));
                (operation, LifecycleResult::Unit(result))
            }
            LifecycleAction::PrepareKeyRotation {
                operation,
                descriptor_from,
                message_kind,
            } => {
                let descriptor = self.descriptor(&descriptor_from)?.clone();
                let http_host = Arc::new(ScenarioHttpHost::new(wallet().seqno(7), None));
                let record_id = NonEmptyString::try_from(descriptor.record_id.as_str())
                    .map_err(|error| error.to_string())?;
                let client = WalletClient::new(
                    WalletClientConfig {
                        record_id,
                        address: descriptor.address,
                        public_key: descriptor.public_key,
                        local_secret_ref: Some(descriptor.secret_ref),
                        network: descriptor.network,
                        send_validity_seconds: 300,
                        resolution_margin_seconds: 60,
                        providers: ProviderConfig {
                            toncenter_base_url: "https://testnet.toncenter.com".to_owned(),
                            dns_root_address: None,
                            request_timeout_ms: 15_000,
                        },
                    },
                    http_host,
                    self.host.clone(),
                )
                .map_err(|error| error.to_string())?;
                let result = block_on(client.prepare_key_rotation(PrepareKeyRotationRequest {
                    valid_until: 1_900_000_000,
                    message_kind,
                }));
                (operation, LifecycleResult::KeyRotation(result))
            }
            LifecycleAction::ReplaceProtectedSecret {
                target_descriptor,
                source_descriptor,
            } => {
                let target = self.descriptor(&target_descriptor)?.secret_ref.clone();
                let source = self.descriptor(&source_descriptor)?.secret_ref.clone();
                self.host.replace_secret(&target, &source)?;
                return Ok(());
            }
            LifecycleAction::FailNextProtectedSecretStore => {
                self.host.fail_next_secret_store();
                return Ok(());
            }
            LifecycleAction::FailNextProtectedSecretRead => {
                self.host.fail_next_secret_read();
                return Ok(());
            }
            LifecycleAction::FailNextProtectedSecretDelete => {
                self.host.fail_next_secret_delete();
                return Ok(());
            }
        };

        if self.results.insert(operation.clone(), result).is_some() {
            return Err(format!("operation `{operation}` already exists"));
        }
        Ok(())
    }

    fn assert(&self, expectation: LifecycleExpectation) -> Result<(), String> {
        match expectation {
            LifecycleExpectation::DescriptorIs {
                operation,
                record_id,
                network,
            } => {
                let descriptor = self.descriptor(&operation)?;
                let expected_ref = format!("wallet:{record_id}:mnemonic");
                if descriptor.record_id == record_id
                    && descriptor.network == network
                    && descriptor.secret_ref.value == expected_ref
                    && descriptor.public_key.len() == 32
                {
                    Ok(())
                } else {
                    Err(format!(
                        "descriptor `{operation}` did not preserve record, network, address, public key, and secret reference"
                    ))
                }
            }
            LifecycleExpectation::DescriptorAddressIs { operation, address } => {
                let actual = &self.descriptor(&operation)?.address;
                if actual.as_str() == address {
                    Ok(())
                } else {
                    Err(format!("expected address `{address}`, got `{actual}`"))
                }
            }
            LifecycleExpectation::DescriptorAddressesDiffer { left, right } => {
                let left_address = &self.descriptor(&left)?.address;
                let right_address = &self.descriptor(&right)?.address;
                if left_address != right_address {
                    Ok(())
                } else {
                    Err(format!(
                        "expected `{left}` and `{right}` to derive different addresses"
                    ))
                }
            }
            LifecycleExpectation::PhraseHasWords { operation, count } => {
                let actual = self.phrase(&operation)?.split_ascii_whitespace().count();
                if actual == count {
                    Ok(())
                } else {
                    Err(format!("expected {count} words, got {actual}"))
                }
            }
            LifecycleExpectation::PhraseIs { operation, words } => {
                let actual = self.phrase(&operation)?;
                if actual == words.join(" ") {
                    Ok(())
                } else {
                    Err(format!(
                        "phrase `{operation}` did not preserve the imported words"
                    ))
                }
            }
            LifecycleExpectation::PhrasesMatch { left, right } => {
                if self.phrase(&left)? == self.phrase(&right)? {
                    Ok(())
                } else {
                    Err(format!("phrases `{left}` and `{right}` differ"))
                }
            }
            LifecycleExpectation::ProtectedSecretIsStored { descriptor_from } => {
                let secret_ref = &self.descriptor(&descriptor_from)?.secret_ref;
                if self.host.secret_exists(secret_ref)
                    && self.host.secret_requires_user_presence(secret_ref) == Some(true)
                {
                    Ok(())
                } else {
                    Err(format!(
                        "secret for `{descriptor_from}` was not stored with user presence"
                    ))
                }
            }
            LifecycleExpectation::ProtectedSecretWasRevealed { descriptor_from } => {
                let secret_ref = &self.descriptor(&descriptor_from)?.secret_ref;
                if self
                    .host
                    .secret_was_read_for(secret_ref, SecretAccessReason::RevealRecoveryPhrase)
                {
                    Ok(())
                } else {
                    Err(format!(
                        "secret for `{descriptor_from}` was not read for phrase reveal"
                    ))
                }
            }
            LifecycleExpectation::KeyRotationMaterialIs {
                operation,
                message_kind,
            } => match self.results.get(&operation) {
                Some(LifecycleResult::KeyRotation(Ok(prepared)))
                    if prepared
                        .replacement_recovery_phrase
                        .phrase
                        .split_ascii_whitespace()
                        .count()
                        == 24
                        && prepared.new_public_key.len() == 32
                        && prepared.seqno == 7
                        && prepared.valid_until == 1_900_000_000
                        && prepared.message_kind == message_kind =>
                {
                    Ok(())
                }
                Some(LifecycleResult::KeyRotation(Ok(_))) => {
                    Err(format!("rotation material `{operation}` is incomplete"))
                }
                Some(LifecycleResult::KeyRotation(Err(error))) => Err(format!(
                    "rotation preparation `{operation}` failed: {error}"
                )),
                Some(_) => Err(format!("operation `{operation}` is not a key rotation")),
                None => Err(format!("operation `{operation}` does not exist")),
            },
            LifecycleExpectation::ProtectedSecretWasReadForKeyRotation { descriptor_from } => {
                let secret_ref = &self.descriptor(&descriptor_from)?.secret_ref;
                if self
                    .host
                    .secret_was_read_for(secret_ref, SecretAccessReason::PrepareKeyRotation)
                {
                    Ok(())
                } else {
                    Err(format!(
                        "secret for `{descriptor_from}` was not read for key rotation"
                    ))
                }
            }
            LifecycleExpectation::ProtectedSecretIsDeleted { descriptor_from } => {
                let secret_ref = &self.descriptor(&descriptor_from)?.secret_ref;
                if self.host.secret_exists(secret_ref) {
                    Err(format!("secret for `{descriptor_from}` still exists"))
                } else {
                    Ok(())
                }
            }
            LifecycleExpectation::StoredSecretCount(expected) => {
                let actual = self.host.stored_secret_count();
                if actual == expected {
                    Ok(())
                } else {
                    Err(format!("expected {expected} stored secrets, got {actual}"))
                }
            }
            LifecycleExpectation::Success { operation } => match self.results.get(&operation) {
                Some(LifecycleResult::Unit(Ok(()))) => Ok(()),
                Some(_) => Err(format!("operation `{operation}` did not succeed")),
                None => Err(format!("operation `{operation}` does not exist")),
            },
            LifecycleExpectation::Error {
                operation,
                expected,
            } => {
                let actual = self.error(&operation)?;
                if actual == &expected {
                    Ok(())
                } else {
                    Err(format!("expected {expected:?}, got {actual:?}"))
                }
            }
        }
    }

    fn descriptor(&self, operation: &str) -> Result<&WalletDescriptor, String> {
        match self.results.get(operation) {
            Some(LifecycleResult::Created(Ok(created))) => Ok(&created.descriptor),
            Some(LifecycleResult::Descriptor(Ok(descriptor))) => Ok(descriptor),
            Some(_) => Err(format!("operation `{operation}` has no descriptor")),
            None => Err(format!("operation `{operation}` does not exist")),
        }
    }

    fn phrase(&self, operation: &str) -> Result<&str, String> {
        match self.results.get(operation) {
            Some(LifecycleResult::Created(Ok(created))) => Ok(&created.recovery_phrase.phrase),
            Some(LifecycleResult::Phrase(Ok(phrase))) => Ok(&phrase.phrase),
            Some(_) => Err(format!("operation `{operation}` has no recovery phrase")),
            None => Err(format!("operation `{operation}` does not exist")),
        }
    }

    fn error(&self, operation: &str) -> Result<&WalletLifecycleError, String> {
        let error = match self.results.get(operation) {
            Some(LifecycleResult::Created(Err(error)))
            | Some(LifecycleResult::Descriptor(Err(error)))
            | Some(LifecycleResult::Phrase(Err(error)))
            | Some(LifecycleResult::Unit(Err(error))) => error,
            Some(_) => return Err(format!("operation `{operation}` succeeded")),
            None => return Err(format!("operation `{operation}` does not exist")),
        };
        Ok(error)
    }
}
