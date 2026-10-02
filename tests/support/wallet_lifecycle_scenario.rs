use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

use futures::executor::block_on;
use sha2::{Digest as _, Sha256};
use ton::block_tlb::CommonMsgInfo;
use wallet_engine::{
    Boc, CreateEncryptedCommentRequest, CreateWalletRequest, CreatedWallet, DecryptCommentRequest,
    ImportWalletRequest, KeyRotationMessageKind, Network, NonEmptyString,
    PrepareKeyRotationRequest, PreparedKeyRotation, ProviderConfig, RecoveryPhrase,
    SecretAccessReason, SendAmount, SendBocRequest, SendExpiration, SendIntent, SendMessage,
    SendMessageBody, SendPhase, SendRequest, TonAddressString, UnsignedDecimalString, WalletClient,
    WalletClientConfig, WalletClientError, WalletDescriptor, WalletLifecycle, WalletLifecycleError,
};

use super::host::{MemoryPlatformHost, RequestKind, ScenarioHttpHost};
use super::localnet::{
    LocalnetHttpHost, change_key_request_public_key, parse_key_changed_log, transaction_aborted,
};
use super::scenario::wallet;
use super::test_wallet::{rotation_anchor_key_pair, test_wallet};

/// Domain-separation salt of Wallet rev00's encrypted old private key.
const KEY_CHANGE_SALT: &[u8] = b"keyChangeSaltV1";

pub(crate) fn wallet_lifecycle_scenario(name: impl Into<String>) -> WalletLifecycleScenario {
    WalletLifecycleScenario {
        name: name.into(),
        steps: Vec::new(),
    }
}

pub(crate) fn execute_repeated_key_rotation_on_localnet() -> Result<(), String> {
    rotate_twice_on_localnet(|_| Ok(()), |_, ()| Ok(())).map(|_| ())
}

/// Decrypts comments sent to every signing key of a twice-rotated wallet.
///
/// The wallet rotates K0 (anchor) -> K1 -> K2 on localnet. While each key is
/// current, an independent sender encrypts a comment to the key the wallet's
/// `get_public_key` reports, exactly as another wallet would. After the second
/// rotation the recovery phrase holds only K0 and K2, so the engine must read
/// the key-change history (served by the localnet indexer shim from the real
/// contract logs) and recover K1 from the encrypted old key K2's rotation
/// published. Comments to K2 and K0 decrypt without any history request, and
/// the history read for K1 is cached.
pub(crate) fn execute_comment_to_replaced_signing_key_decrypts_on_localnet() -> Result<(), String> {
    const LOST_KEY_COMMENT: &str = "sent while K1 was the signing key";
    const CURRENT_KEY_COMMENT: &str = "sent to the current key K2";
    const ANCHOR_KEY_COMMENT: &str = "sent to the anchor key K0";

    let rotation = rotate_twice_on_localnet(
        |context| {
            let sender = comment_sender(context)?;
            let anchor_key_comment = sender.encrypt_to_on_chain_key(context, ANCHOR_KEY_COMMENT)?;
            Ok((sender, anchor_key_comment))
        },
        |context, (sender, anchor_key_comment)| {
            let lost_key_comment = sender.encrypt_to_on_chain_key(context, LOST_KEY_COMMENT)?;
            Ok((sender, anchor_key_comment, lost_key_comment))
        },
    )?;
    let RepeatedKeyRotation {
        context,
        second,
        between_rotations: (sender, anchor_key_comment, lost_key_comment),
    } = rotation;
    let current_key_comment = sender.encrypt_to_on_chain_key(&context, CURRENT_KEY_COMMENT)?;

    // Only the latest phrase remains: anchor K0 plus signing K2. K1 is gone.
    let rotated_descriptor = block_on(
        context.lifecycle.import_wallet(ImportWalletRequest {
            record_id: "localnet-key-rotation-latest".to_owned(),
            network: Network::Testnet,
            recovery_words: second
                .replacement_recovery_phrase
                .phrase
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect(),
        }),
    )
    .map_err(|error| error.to_string())?;
    if rotated_descriptor.address != context.wallet_address {
        return Err("the latest replacement phrase changed the wallet address".to_owned());
    }
    let rotated_client = localnet_wallet_client(
        rotated_descriptor,
        context.localnet.clone(),
        context.platform_host.clone(),
    )?;
    let decrypt = |body: &Boc| {
        block_on(rotated_client.decrypt_comment(DecryptCommentRequest {
            sender: sender.address.clone(),
            body: body.clone(),
        }))
        .map_err(|error| error.to_string())
    };
    let expect_comment = |body: &Boc, expected: &str, key: &str| {
        let actual = decrypt(body)?;
        if actual == expected {
            Ok(())
        } else {
            Err(format!(
                "the comment to {key} decrypted to {actual:?}, expected {expected:?}"
            ))
        }
    };
    let expect_history_requests = |expected: usize, step: &str| {
        let actual = context.localnet.key_change_action_request_count();
        if actual == expected {
            Ok(())
        } else {
            Err(format!(
                "expected {expected} key-change history requests {step}, got {actual}"
            ))
        }
    };

    expect_comment(&current_key_comment, CURRENT_KEY_COMMENT, "K2")?;
    expect_comment(&anchor_key_comment, ANCHOR_KEY_COMMENT, "K0")?;
    expect_history_requests(0, "after decrypting with keys the phrase holds")?;

    expect_comment(&lost_key_comment, LOST_KEY_COMMENT, "the replaced key K1")?;
    expect_history_requests(1, "after recovering the replaced key")?;
    expect_comment(&lost_key_comment, LOST_KEY_COMMENT, "K1 again")?;
    expect_history_requests(1, "after reusing the cached history")?;

    let other_sender_error = block_on(rotated_client.decrypt_comment(DecryptCommentRequest {
        sender: context.wallet_address.clone(),
        body: lost_key_comment,
    }))
    .expect_err("a comment authenticated for another sender must not decrypt");
    if !matches!(
        other_sender_error,
        WalletClientError::EncryptedCommentUnavailable { .. }
    ) {
        return Err(format!(
            "expected no key to decrypt a comment for another sender, got {other_sender_error}"
        ));
    }
    expect_history_requests(1, "after a failed decryption with the cached history")
}

/// Shared localnet state of one rotating wallet.
struct LocalnetRotationContext {
    platform_host: Arc<MemoryPlatformHost>,
    lifecycle: Arc<WalletLifecycle>,
    localnet: Arc<LocalnetHttpHost>,
    wallet_address: TonAddressString,
}

/// A wallet that rotated K0 (anchor) -> K1 -> K2 on localnet.
struct RepeatedKeyRotation<T> {
    context: LocalnetRotationContext,
    second: PreparedKeyRotation,
    between_rotations: T,
}

/// Rotates the fixture wallet twice with external requests on localnet.
///
/// Each rotation must confirm, install its key on-chain, and emit the
/// contract's key-changed log with the old key encrypted under the new one.
/// `before_rotations` runs once the wallet is deployed with K0, and
/// `between_rotations` receives its result while K1 is the current signing key.
fn rotate_twice_on_localnet<B, T>(
    before_rotations: impl FnOnce(&LocalnetRotationContext) -> Result<B, String>,
    between_rotations: impl FnOnce(&LocalnetRotationContext, B) -> Result<T, String>,
) -> Result<RepeatedKeyRotation<T>, String> {
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
    let initial_phrase =
        std::str::from_utf8(fixture.recovery_phrase_bytes()).map_err(|error| error.to_string())?;

    localnet.spam_transfers(1)?;
    let context = LocalnetRotationContext {
        platform_host: platform_host.clone(),
        lifecycle: lifecycle.clone(),
        localnet: localnet.clone(),
        wallet_address: wallet_address.clone(),
    };
    let before_rotations = before_rotations(&context)?;

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
    assert_key_changed_log(&localnet, &first, initial_phrase, "first")?;

    let between_rotations = between_rotations(&context, before_rotations)?;

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
    assert_key_changed_log(
        &localnet,
        &second,
        &first.replacement_recovery_phrase.phrase,
        "second",
    )?;

    Ok(RepeatedKeyRotation {
        context,
        second,
        between_rotations,
    })
}

/// An independent wallet that encrypts comments to the rotating wallet.
struct CommentSender {
    address: TonAddressString,
    client: Arc<WalletClient>,
}

impl CommentSender {
    /// Encrypts a comment to the key the recipient's `get_public_key` reports now.
    fn encrypt_to_on_chain_key(
        &self,
        context: &LocalnetRotationContext,
        comment: &str,
    ) -> Result<Boc, String> {
        block_on(
            self.client
                .create_encrypted_comment(CreateEncryptedCommentRequest {
                    recipient: context.wallet_address.clone(),
                    comment: comment.to_owned(),
                    recipient_public_key: None,
                }),
        )
        .map_err(|error| format!("encrypting {comment:?} to the on-chain key failed: {error}"))
    }
}

/// Imports the fixture's other wallet as a comment sender on the same localnet.
///
/// Encrypting needs no deployment or funds: the sender only resolves the
/// recipient's key through the localnet provider and signs locally.
fn comment_sender(context: &LocalnetRotationContext) -> Result<CommentSender, String> {
    let phrase = std::str::from_utf8(test_wallet().other_recovery_phrase_bytes())
        .map_err(|error| error.to_string())?;
    let descriptor = block_on(context.lifecycle.import_wallet(ImportWalletRequest {
        record_id: "localnet-comment-sender".to_owned(),
        network: Network::Testnet,
        recovery_words: phrase.split_whitespace().map(str::to_owned).collect(),
    }))
    .map_err(|error| error.to_string())?;
    let address = descriptor.address.clone();
    let client = localnet_wallet_client(
        descriptor,
        context.localnet.clone(),
        context.platform_host.clone(),
    )?;
    Ok(CommentSender { address, client })
}

/// Checks the key-changed log of the wallet transaction that executed `rotation`.
///
/// The transaction must emit exactly one external-out message: opcode
/// `0xEBA19948` followed by `sha256(new_seed ‖ "keyChangeSaltV1") XOR old_seed`.
/// The expected value is computed here from the recovery phrases, without the
/// engine's key-history code.
fn assert_key_changed_log(
    localnet: &LocalnetHttpHost,
    rotation: &PreparedKeyRotation,
    old_phrase: &str,
    label: &str,
) -> Result<(), String> {
    let transactions = localnet.wallet_transactions()?;
    let mut executed = transactions.iter().filter(|transaction| {
        transaction.msgs.in_msg.as_ref().is_some_and(|message| {
            change_key_request_public_key(&message.body.value)
                .is_some_and(|key| key.as_slice() == rotation.new_public_key.as_slice())
        })
    });
    let (Some(transaction), None) = (executed.next(), executed.next()) else {
        return Err(format!(
            "expected exactly one wallet transaction for the {label} rotation"
        ));
    };
    if transaction_aborted(transaction) {
        return Err(format!("the {label} rotation transaction was aborted"));
    }
    let logs = transaction
        .msgs
        .out_msgs
        .iter()
        .filter(|message| matches!(message.info, CommonMsgInfo::ExtOut(_)))
        .collect::<Vec<_>>();
    let [log] = logs.as_slice() else {
        return Err(format!(
            "expected the {label} rotation to emit exactly one external-out message, got {}",
            logs.len()
        ));
    };
    let logged =
        parse_key_changed_log(log).map_err(|error| format!("{label} rotation: {error}"))?;

    let (old_seed, _) = signing_seed(old_phrase)?;
    let (new_seed, new_public_key) = signing_seed(&rotation.replacement_recovery_phrase.phrase)?;
    if new_public_key.as_slice() != rotation.new_public_key.as_slice() {
        return Err(format!(
            "the {label} replacement phrase does not hold the installed signing key"
        ));
    }
    let mask = Sha256::new()
        .chain_update(new_seed)
        .chain_update(KEY_CHANGE_SALT)
        .finalize();
    let expected = mask
        .iter()
        .zip(old_seed)
        .map(|(mask, old)| mask ^ old)
        .collect::<Vec<_>>();
    if logged.as_slice() != expected.as_slice() {
        return Err(format!(
            "the {label} rotation logged an encrypted old key that is not sha256(new_seed ‖ salt) XOR old_seed"
        ));
    }
    if logged == old_seed {
        return Err(format!(
            "the {label} rotation published the old key in plaintext"
        ));
    }
    Ok(())
}

/// Independently derives the Ed25519 seed and public key of a phrase's signing key.
///
/// A 12-word phrase signs with its anchor; a 24-word rotation phrase signs
/// with words 13-24, which use the same derivation as the anchor half.
fn signing_seed(phrase: &str) -> Result<([u8; 32], [u8; 32]), String> {
    let words = phrase.split_whitespace().collect::<Vec<_>>();
    let signing_half = match words.len() {
        12 => words.join(" "),
        24 => words
            .get(12..)
            .ok_or_else(|| "a 24-word phrase has a signing half".to_owned())?
            .join(" "),
        count => {
            return Err(format!(
                "expected a 12- or 24-word phrase, got {count} words"
            ));
        }
    };
    let key_pair = rotation_anchor_key_pair(&signing_half)?;
    let seed = key_pair
        .secret_key
        .get(..32)
        .and_then(|seed| <[u8; 32]>::try_from(seed).ok())
        .ok_or_else(|| "an Ed25519 key pair starts with its 32-byte seed".to_owned())?;
    Ok((seed, key_pair.public_key))
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

/// A rotated 24-word phrase cannot deploy: the deployed contract would store
/// the anchor key while the request is signed with words 13-24.
pub(crate) fn execute_rotated_phrase_cannot_rotate_an_undeployed_wallet_on_localnet()
-> Result<(), String> {
    const REPLACEMENT_HALF: &str =
        "clump left year void clutch tool case burden fix income champion lounge";
    let platform_host = Arc::new(MemoryPlatformHost::default());
    let lifecycle = WalletLifecycle::new(platform_host.clone());
    let mut recovery_words = test_wallet().recovery_words();
    recovery_words.extend(REPLACEMENT_HALF.split_whitespace().map(str::to_owned));
    let descriptor = block_on(lifecycle.import_wallet(ImportWalletRequest {
        record_id: "localnet-rotated-undeployed-key-rotation".to_owned(),
        network: Network::Testnet,
        recovery_words,
    }))
    .map_err(|error| error.to_string())?;
    if descriptor.address.as_str() != test_wallet().testnet_address() {
        return Err("the rotated phrase must keep the anchor address".to_owned());
    }
    let localnet = Arc::new(LocalnetHttpHost::start(
        descriptor.address.as_str(),
        "5000000000",
    )?);
    let client = localnet_wallet_client(descriptor, localnet.clone(), platform_host)?;

    match prepare_external_rotation_result(&client).map(|_| "prepared rotation material") {
        Err(WalletClientError::KeyRotationUnavailable { diagnostic })
            if diagnostic.contains("requires an already deployed wallet") => {}
        other => {
            return Err(format!(
                "expected a rejected rotation of an undeployed wallet, got {other:?}"
            ));
        }
    }
    if localnet.submitted_boc().is_some() {
        return Err("a rejected rotation must not reach the provider".to_owned());
    }
    // The slot is released: the same client can still prepare other work.
    match prepare_external_rotation_result(&client).map(|_| "prepared rotation material") {
        Err(WalletClientError::KeyRotationUnavailable { .. }) => Ok(()),
        other => Err(format!("expected the same rejection again, got {other:?}")),
    }
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
    prepare_external_rotation_result(client).map_err(|error| error.to_string())
}

fn prepare_external_rotation_result(
    client: &WalletClient,
) -> Result<PreparedKeyRotation, WalletClientError> {
    block_on(client.prepare_key_rotation(PrepareKeyRotationRequest {
        valid_until: u64::from(u32::MAX),
        message_kind: KeyRotationMessageKind::External,
    }))
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
