use super::support::*;
use wallet_engine::{AccountStatus, SendPhase, WalletClientError};

const EMPTY_DESTINATION: &str =
    "0:2222222222222222222222222222222222222222222222222222222222222222";

#[test]
fn second_send_is_rejected_while_first_is_in_progress() {
    scenario("second send while first is in progress")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(submission().paused("first-submit"))
        .when(start("first", send().to(own_address()).grams(1)))
        .then(send_phase("first", SendPhase::Submitting))
        .when(call("second", send().to(own_address()).grams(2)))
        .then(error("second", WalletClientError::SendAlreadyInProgress))
        .then(send_phase("first", SendPhase::Submitting))
        .when(resume("first-submit", submission_accepted()))
        .then(result("first").submitted())
        .then(snapshot().send_phase(SendPhase::Submitted))
        .run();
}

#[test]
fn emulation_completes_before_secret_authorization_and_persistence() {
    scenario("send preview uses only public wallet metadata")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(submission().paused("submit"))
        .when(pause_next_emulation_request("emulation"))
        .when(start(
            "preview",
            preview_send(send().to(own_address()).grams(1)),
        ))
        .when(wait_for_request("emulation"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .when(release_request("emulation"))
        .then(result("preview").emulated_action("ton_transfer"))
        .when(start("send", send().to(own_address()).grams(1)))
        .then(send_phase("send", SendPhase::Submitting))
        .when(resume("submit", submission_accepted()))
        .then(result("send").submitted())
        .run();
}

#[test]
fn public_key_only_wallet_can_preview_but_cannot_sign_locally() {
    scenario("public-key-only identity remains useful without a local mnemonic")
        .given(
            wallet()
                .active()
                .balance(grams(10))
                .seqno(7)
                .public_key_only(),
        )
        .when(call(
            "preview",
            preview_send(send().to(own_address()).grams(1)),
        ))
        .then(result("preview").previewed())
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(remember_revision("before-send"))
        .when(call("send", send().to(own_address()).grams(1)))
        .then(error("send", WalletClientError::LocalSigningUnavailable))
        .then(revision_is("before-send"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .run();
}

#[test]
fn cancelling_emulation_discards_its_late_result_before_authorization() {
    scenario("cancelled preview cannot unlock or revive a send")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(pause_next_emulation_request("emulation"))
        .when(start(
            "preview",
            preview_send(send().to(own_address()).grams(1)),
        ))
        .when(wait_for_request("emulation"))
        .when(call("cancel", cancel_send_preview()))
        .then(succeeds("cancel"))
        .then(request_was_cancelled("emulation"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .then(remember_revision("after-cancel"))
        .when(release_request("emulation"))
        .then(error("preview", WalletClientError::StateUnavailable))
        .then(revision_is("after-cancel"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .run();
}

#[test]
fn provider_emulation_failure_stops_before_secret_and_journal() {
    scenario("emulation transport failure keeps the send retryable")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(provider().emulation_fails(503))
        .when(call(
            "preview",
            preview_send(send().to(own_address()).grams(1)),
        ))
        .then(emulation_failed("preview"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .run();
}

#[test]
fn failed_preview_does_not_block_the_independent_send_workflow() {
    scenario("preview failure warns the client but does not gate confirmation")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(provider().emulation_fails(503))
        .when(call(
            "preview",
            preview_send(send().to(own_address()).grams(1)),
        ))
        .then(emulation_failed("preview"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        // The real workflow does not reuse or retry the failed preview. It loads
        // fresh account state and seqno before requesting authorization.
        .when(call("send", send().to(own_address()).grams(1)))
        .then(result("send").submitted())
        .then(snapshot().send_phase(SendPhase::Submitted))
        .run();
}

#[test]
fn rejected_emulation_exposes_tvm_phase_codes_without_unlocking_secret() {
    scenario("failed emulation is a typed transfer rejection")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(provider().emulation_rejects_transfer())
        .when(call(
            "preview",
            preview_send(send().to(own_address()).grams(1)),
        ))
        .then(error(
            "preview",
            WalletClientError::EmulationRejected {
                diagnostic: "emulated transfer did not complete successfully".to_owned(),
                compute_exit_code: Some(33),
                action_result_code: Some(34),
            },
        ))
        .then(snapshot().send_phase(SendPhase::Idle))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .run();
}

#[test]
fn exact_preview_rejects_a_value_that_leaves_no_balance_for_fees() {
    scenario("an exact transfer must leave enough balance for its emulated wallet fee")
        .given(wallet().active().balance(grams(1)).seqno(7))
        // The amount itself fits exactly, but the scripted emulation reports a
        // 1_000_000 nanogram source-wallet fee.
        .when(call(
            "preview",
            preview_send(send().to(own_address()).grams(1)),
        ))
        .then(error(
            "preview",
            WalletClientError::InsufficientBalanceForFees {
                available_nanograms: "1000000000".to_owned(),
                requested_nanograms: "1000000000".to_owned(),
                estimated_fee_nanograms: "1000000".to_owned(),
            },
        ))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .run();
}

#[test]
fn exact_preview_requires_a_positive_remainder_after_the_estimated_fee() {
    scenario("an exact transfer cannot consume the balance exactly after fees")
        .given(wallet().active().balance(grams(1)).seqno(7))
        // The scripted wallet fee is 1_000_000 nanograms. Together with the
        // requested 999_000_000 nanograms it consumes the complete balance.
        // Send-all mode is the explicit policy for that intent.
        .when(call(
            "preview",
            preview_send(send().to(own_address()).nanograms(999_000_000)),
        ))
        .then(error(
            "preview",
            WalletClientError::InsufficientBalanceForFees {
                available_nanograms: "1000000000".to_owned(),
                requested_nanograms: "999000000".to_owned(),
                estimated_fee_nanograms: "1000000".to_owned(),
            },
        ))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .run();
}

#[test]
fn exact_transfer_keeps_mode_3() {
    scenario("an exact transfer preserves its value and pays fees separately")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(call("send", send().to(own_address()).grams(1)))
        .then(result("send").submitted())
        .then(submitted_message().uses_send_mode(EXACT_AMOUNT_SEND_MODE))
        .run();
}

#[test]
fn plaintext_comment_is_submitted_and_executed_on_localnet() {
    let comment = "Привет, TON — localnet!";

    scenario("a plaintext comment survives signing and localnet submission")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call(
            "send",
            send().to(own_address()).nanograms(1).comment(comment),
        ))
        .then(result("send").submitted())
        .then(submitted_message().has_comment(comment))
        .then(on_chain_wallet().active().seqno(1))
        .run();
}

#[test]
fn all_balance_transfer_uses_mode_130() {
    scenario("send all delegates the remaining-balance calculation to Wallet V5")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(call("send", send().to(own_address()).all()))
        .then(result("send").submitted())
        .then(submitted_message().uses_send_mode(ALL_BALANCE_SEND_MODE))
        .run();
}

#[test]
fn all_balance_transfer_executes_on_localnet() {
    scenario("send all deploys the wallet and transfers its remaining balance")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call(
            "preview",
            preview_send(send().to(address(EMPTY_DESTINATION)).all()),
        ))
        // Localnet can omit the high-level action list for a carry-all message.
        // A decoded successful trace is still required before confirmation.
        .then(result("preview").previewed())
        .when(call("send", send().to(address(EMPTY_DESTINATION)).all()))
        .then(result("send").submitted())
        .then(submitted_message().contains_state_init())
        .then(submitted_message().uses_send_mode(ALL_BALANCE_SEND_MODE))
        .then(on_chain_wallet().active().seqno(1))
        .run();
}

#[test]
fn send_cannot_be_cancelled_after_the_durable_commit_boundary() {
    scenario("cancel after the signed message becomes durable")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(submission().paused("submit"))
        .when(start("send", send().to(own_address()).grams(1)))
        .then(send_phase("send", SendPhase::Submitting))
        .when(call("cancel", cancel_send()))
        .then(error("cancel", WalletClientError::SendCancellationTooLate))
        .then(send_phase("send", SendPhase::Submitting))
        .when(resume("submit", submission_accepted()))
        .then(result("send").submitted())
        .run();
}

#[test]
fn cancel_before_the_durable_boundary_stops_the_request_and_releases_the_slot() {
    scenario("cancel before signing stops the send without durable state")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(pause_next_account_request("fresh-account"))
        .when(start("cancelled-send", send().to(own_address()).grams(1)))
        .when(wait_for_request("fresh-account"))
        .when(call("cancel", cancel_send()))
        .then(succeeds("cancel"))
        .then(request_was_cancelled("fresh-account"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        // A successful late response belongs to the cancelled generation and
        // must not revive authorization, persistence, or submission.
        .then(remember_revision("after-cancel"))
        .when(release_request("fresh-account"))
        .then(error("cancelled-send", WalletClientError::StateUnavailable))
        .then(revision_is("after-cancel"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        // Cancellation before commit releases the wallet-local send slot.
        .given(submission().paused("retry-submit"))
        .when(start("retry", send().to(own_address()).grams(1)))
        .then(send_phase("retry", SendPhase::Submitting))
        .when(resume("retry-submit", submission_accepted()))
        .then(result("retry").submitted())
        .run();
}

#[test]
fn cancelling_while_the_journal_load_is_pending_discards_its_late_result() {
    scenario("cancel during journal load cannot revive a send")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(pause_next_journal_load("journal-load"))
        .when(start("send", send().to(own_address()).grams(1)))
        .when(wait_for_platform_call("journal-load"))
        .when(call("cancel", cancel_send()))
        .then(succeeds("cancel"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .then(remember_revision("after-cancel"))
        .when(release_platform_call("journal-load"))
        .then(error("send", WalletClientError::StateUnavailable))
        .then(revision_is("after-cancel"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .run();
}

#[test]
fn cancelling_while_seqno_is_pending_cancels_only_that_request() {
    scenario("cancel during seqno fetch cannot continue to authorization")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(pause_next_seqno_request("seqno"))
        .when(start("send", send().to(own_address()).grams(1)))
        .when(wait_for_request("seqno"))
        .when(call("cancel", cancel_send()))
        .then(succeeds("cancel"))
        .then(request_was_cancelled("seqno"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .then(remember_revision("after-cancel"))
        .when(release_request("seqno"))
        .then(error("send", WalletClientError::StateUnavailable))
        .then(revision_is("after-cancel"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .run();
}

#[test]
fn cancelling_while_secret_authorization_is_pending_discards_the_secret() {
    scenario("cancel during protected-secret read cannot sign or persist")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(pause_next_secret_read("secret-read"))
        .when(start("send", send().to(own_address()).grams(1)))
        .when(wait_for_platform_call("secret-read"))
        .when(call("cancel", cancel_send()))
        .then(succeeds("cancel"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .then(remember_revision("after-cancel"))
        .when(release_platform_call("secret-read"))
        .then(error("send", WalletClientError::StateUnavailable))
        .then(revision_is("after-cancel"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .run();
}

#[test]
fn cancelling_without_an_active_send_is_idempotent() {
    scenario("cancel send is safe when no send exists")
        .when(call("first-cancel", cancel_send()))
        .then(succeeds("first-cancel"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .when(call("second-cancel", cancel_send()))
        .then(succeeds("second-cancel"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .run();
}

#[test]
fn unknown_submission_blocks_a_replacement_send() {
    scenario("unknown submission blocks a replacement send")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(submission().paused("first-submit"))
        .when(start("first", send().to(own_address()).grams(1)))
        .then(send_phase("first", SendPhase::Submitting))
        .when(resume("first-submit", submission_timeout()))
        .then(result("first").submission_unknown())
        .when(call("replacement", send().to(own_address()).grams(2)))
        .then(error(
            "replacement",
            WalletClientError::PreviousSubmissionUnresolved,
        ))
        .then(snapshot().send_phase(SendPhase::Failed))
        .run();
}

#[test]
fn explicit_rejection_releases_the_wallet_send_slot() {
    scenario("explicit provider rejection permits a new send")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(submission().paused("rejected-submit"))
        .when(start("rejected", send().to(own_address()).grams(1)))
        .then(send_phase("rejected", SendPhase::Submitting))
        .when(resume(
            "rejected-submit",
            submission_rejected("provider rejected the message"),
        ))
        .then(result("rejected").failed())
        .given(submission().paused("accepted-submit"))
        .when(start("accepted", send().to(own_address()).grams(2)))
        .then(send_phase("accepted", SendPhase::Submitting))
        .when(resume("accepted-submit", submission_accepted()))
        .then(result("accepted").submitted())
        .run();
}

#[test]
fn uninitialized_wallet_can_deploy_and_send() {
    scenario("first transfer deploys an uninitialized wallet")
        .given(wallet().uninitialized().balance(grams(10)))
        .given(submission().paused("submit"))
        .when(start("send", send().to(own_address()).grams(1)))
        .then(send_phase("send", SendPhase::Submitting))
        .then(submitted_message().contains_state_init())
        .when(resume("submit", submission_accepted()))
        .then(result("send").submitted())
        .then(snapshot().send_phase(SendPhase::Submitted))
        .run();
}

#[test]
fn first_transfer_deploys_the_wallet_on_localnet_and_appears_in_history() {
    scenario("first transfer activates an uninitialized wallet on localnet")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(pause_next_emulation_request("deploy-emulation"))
        .when(start(
            "preview",
            preview_send(send().to(own_address()).grams(1)),
        ))
        .when(wait_for_request("deploy-emulation"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .when(release_request("deploy-emulation"))
        // Localnet can omit the high-level action list, but the trace and fees
        // must still form a valid preview before the user confirms the send.
        .then(result("preview").previewed())
        .when(start("send", send().to(own_address()).grams(1)))
        .then(submitted_message().contains_state_init())
        .then(result("send").submitted())
        .then(on_chain_wallet().active().seqno(1))
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .then(account_status(AccountStatus::Active))
        .then(activity_present())
        .run();
}

#[test]
fn second_transfer_uses_the_advanced_wallet_seqno_on_localnet() {
    scenario("confirmed transfer advances seqno for the next send")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("first", send().to(own_address()).grams(1)))
        .then(result("first").submitted())
        .then(on_chain_wallet().active().seqno(1))
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .when(call("second", send().to(own_address()).grams(1)))
        .then(result("second").submitted())
        .then(on_chain_wallet().active().seqno(2))
        .when(call("final-refresh", refresh_wallet()))
        .then(update("final-refresh").completed())
        .then(activity_present())
        .run();
}

#[test]
fn send_uses_fresh_seqno_after_an_external_same_key_transfer() {
    scenario("send reads seqno after another client spends with the same key")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("deploy", send().to(own_address()).nanograms(1)))
        .then(result("deploy").submitted())
        .then(on_chain_wallet().active().seqno(1))
        .when(call("baseline", refresh_wallet()))
        .then(update("baseline").completed())
        .then(remember_activity_as("A"))
        // Stop this send before its fresh account HTTP request reaches Acton.
        .when(pause_next_account_request("fresh-account"))
        .when(start("engine-send", send().to(own_address()).nanograms(1)))
        .when(wait_for_request("fresh-account"))
        // A different client with the same mnemonic consumes seqno 1.
        .when(call("external-send", spam_transfers(1)))
        .then(succeeds("external-send"))
        .then(on_chain_wallet().active().seqno(2))
        // The engine must observe seqno 2, sign with it, and execute as seqno 3.
        .when(release_request("fresh-account"))
        .then(result("engine-send").submitted())
        .then(on_chain_wallet().active().seqno(3))
        .when(call("after-both", refresh_wallet()))
        .then(update("after-both").completed())
        .then(remember_new_activity_as("B"))
        .then(activity_is(&["A", "B"]))
        .run();
}

#[test]
fn send_rereads_seqno_after_an_external_transfer_follows_fresh_account() {
    scenario("send reads seqno after fresh account when another client spends")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("deploy", send().to(own_address()).nanograms(1)))
        .then(result("deploy").submitted())
        .then(on_chain_wallet().active().seqno(1))
        // The engine has fetched active account state before this seqno request waits.
        .when(pause_next_seqno_request("fresh-seqno"))
        .when(start("engine-send", send().to(own_address()).nanograms(1)))
        .when(wait_for_request("fresh-seqno"))
        // Another client consumes the seqno that existed at account-fetch time.
        .when(call("external-send", spam_transfers(1)))
        .then(succeeds("external-send"))
        .then(on_chain_wallet().active().seqno(2))
        .when(release_request("fresh-seqno"))
        .then(result("engine-send").submitted())
        .then(on_chain_wallet().active().seqno(3))
        .run();
}

#[test]
fn send_ignores_a_successful_preview_seqno_after_an_external_transfer() {
    scenario("confirmed send rebuilds a preview after another client spends")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("external-deploy", spam_transfers(1)))
        .then(succeeds("external-deploy"))
        .then(on_chain_wallet().active().seqno(1))
        // The preview is valid for the current seqno, but it is only information
        // for the confirmation screen. It does not reserve seqno 1.
        .when(call(
            "preview",
            preview_send(send().to(own_address()).nanograms(1)),
        ))
        .then(result("preview").previewed())
        .then(snapshot().send_phase(SendPhase::Idle))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        // Another client invalidates the preview by consuming seqno 1.
        .when(call("external-spend", spam_transfers(1)))
        .then(succeeds("external-spend"))
        .then(on_chain_wallet().active().seqno(2))
        // Confirmation starts an independent send. It must fetch seqno 2,
        // construct a new signed message, and execute as seqno 3.
        .when(call("send", send().to(own_address()).nanograms(1)))
        .then(result("send").submitted())
        .then(on_chain_wallet().active().seqno(3))
        .run();
}

#[test]
fn external_transfer_while_active_wallet_emulation_is_pending_is_not_misclassified() {
    scenario("an external spend makes a pending active-wallet emulation stale")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        // A different client deploys the wallet without touching this engine's
        // protected-secret or durable-journal hosts.
        .when(call("external-deploy", spam_transfers(1)))
        .then(succeeds("external-deploy"))
        .then(on_chain_wallet().active().seqno(1))
        // This send reads seqno 1 and constructs its fake-signed message before
        // the emulation HTTP request is allowed to reach Acton.
        .when(pause_next_emulation_request("stale-emulation"))
        .when(start(
            "stale-preview",
            preview_send(send().to(own_address()).nanograms(1)),
        ))
        .when(wait_for_request("stale-emulation"))
        .then(snapshot().send_phase(SendPhase::Idle))
        // Another same-key client consumes seqno 1 while the emulation waits.
        .when(call("external-spend", spam_transfers(1)))
        .then(succeeds("external-spend"))
        .then(on_chain_wallet().active().seqno(2))
        .when(release_request("stale-emulation"))
        // Acton rejects the stale external message. This is a chain-state race,
        // not an emulator outage or a successfully created failed transaction.
        .then(emulation_message_not_accepted("stale-preview"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        // A real send fetches seqno 2 independently and executes exactly once.
        .when(call("retry", send().to(own_address()).nanograms(1)))
        .then(result("retry").submitted())
        .then(on_chain_wallet().active().seqno(3))
        .run();
}

#[test]
fn external_deployment_while_first_send_emulation_is_pending_is_not_misclassified() {
    scenario("an external deployment invalidates a pending StateInit emulation")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        // The engine prepares a seqno-zero message with StateInit, then waits.
        .when(pause_next_emulation_request("deployment-emulation"))
        .when(start(
            "stale-preview",
            preview_send(send().to(own_address()).nanograms(1)),
        ))
        .when(wait_for_request("deployment-emulation"))
        .then(snapshot().send_phase(SendPhase::Idle))
        // A different client deploys the same V5R1 wallet first.
        .when(call("external-deploy", spam_transfers(1)))
        .then(succeeds("external-deploy"))
        .then(on_chain_wallet().active().seqno(1))
        .when(release_request("deployment-emulation"))
        .then(emulation_message_not_accepted("stale-preview"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        // The retry observes an active wallet, omits StateInit, and uses seqno 1.
        .when(call("retry", send().to(own_address()).nanograms(1)))
        .then(result("retry").submitted())
        .then(on_chain_wallet().active().seqno(2))
        .run();
}

#[test]
fn replaying_the_exact_external_message_executes_only_once_on_localnet() {
    scenario("the chain deduplicates an exact signed external message replay")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("original", send().to(own_address()).nanograms(1)))
        .then(result("original").submitted())
        .then(on_chain_wallet().active().seqno(1))
        .when(call("initial-refresh", refresh_wallet()))
        .then(update("initial-refresh").completed())
        .then(remember_activity_as("A"))
        // Replay the same BOC with the same external-message hash and seqno.
        // Toncenter can accept the POST, but the wallet must execute it once.
        .when(call("replay", replay_last_submission()))
        .then(succeeds("replay"))
        .then(on_chain_wallet().active().seqno(1))
        .when(call("after-replay", refresh_wallet()))
        .then(update("after-replay").completed())
        .then(activity_is(&["A"]))
        .run();
}

#[test]
fn expired_external_message_is_rejected_before_authorization() {
    scenario("an already expired message cannot pass preflight emulation")
        .given(network().localnet())
        .given(client().send_validity_seconds(1))
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("baseline-refresh", refresh_wallet()))
        .then(update("baseline-refresh").completed())
        .then(remember_activity_as("funding"))
        // The local emulator evaluates valid_until before the engine unlocks
        // the secret or persists a signed message.
        .when(call(
            "expired",
            preview_send(send().to(own_address()).nanograms(1)),
        ))
        .then(emulation_message_not_accepted("expired"))
        .then(snapshot().send_phase(SendPhase::Idle))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .then(on_chain_wallet().uninitialized())
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .then(activity_is(&["funding"]))
        .run();
}

#[test]
fn submitted_send_blocks_replacement_until_seqno_advances() {
    scenario("submitted send waits for the next wallet seqno")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(submission().paused("first-submit"))
        .when(start("first", send().to(own_address()).grams(1)))
        .then(send_phase("first", SendPhase::Submitting))
        .when(resume("first-submit", submission_accepted()))
        .then(result("first").submitted())
        .when(call("too-early", send().to(own_address()).grams(2)))
        .then(error(
            "too-early",
            WalletClientError::WalletSeqnoNotAdvanced,
        ))
        .given(wallet().active().balance(grams(9)).seqno(8))
        .given(submission().paused("next-submit"))
        .when(start("next", send().to(own_address()).grams(2)))
        .then(send_phase("next", SendPhase::Submitting))
        .when(resume("next-submit", submission_accepted()))
        .then(result("next").submitted())
        .run();
}

#[test]
fn zero_amount_is_rejected_before_send_state_changes() {
    scenario("zero amount is not a valid transfer")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(call("send", send().to(own_address()).nanograms(0)))
        .then(error("send", WalletClientError::InvalidSendRequest))
        .then(snapshot().send_phase(SendPhase::Idle))
        .run();
}

#[test]
fn malformed_destination_is_rejected_before_send_state_changes() {
    scenario("malformed destination is not a valid transfer")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(call("send", send().to(invalid_address()).grams(1)))
        .then(error("send", WalletClientError::InvalidSendRequest))
        .then(snapshot().send_phase(SendPhase::Idle))
        .run();
}

#[test]
fn frozen_wallet_stops_before_secret_authorization() {
    scenario("a frozen wallet cannot send")
        .given(wallet().frozen().balance(grams(10)))
        .when(call("send", send().to(own_address()).grams(1)))
        .then(error(
            "send",
            WalletClientError::SendAccountUnavailable {
                status: AccountStatus::Frozen,
            },
        ))
        .then(snapshot().send_phase(SendPhase::Failed))
        .run();
}

#[test]
fn unknown_wallet_state_stops_before_secret_authorization() {
    scenario("an unknown wallet state cannot send")
        .given(wallet().unknown().balance(grams(10)))
        .when(call("send", send().to(own_address()).grams(1)))
        .then(error(
            "send",
            WalletClientError::SendAccountUnavailable {
                status: AccountStatus::Unknown,
            },
        ))
        .then(snapshot().send_phase(SendPhase::Failed))
        .run();
}

#[test]
fn invalid_protected_secret_fails_without_submission() {
    scenario("an invalid protected recovery phrase cannot sign")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(secret().invalid())
        .when(call("send", send().to(own_address()).grams(1)))
        .then(error("send", WalletClientError::InvalidProtectedSecret))
        .then(snapshot().send_phase(SendPhase::Failed))
        .run();
}

#[test]
fn valid_secret_from_another_wallet_cannot_sign_the_selected_wallet() {
    scenario("a valid mnemonic for another wallet cannot authorize this wallet")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(secret().belongs_to_another_wallet())
        .when(call("send", send().to(own_address()).grams(1)))
        .then(error(
            "send",
            send_failed("protected mnemonic does not belong to this wallet"),
        ))
        .then(snapshot().send_phase(SendPhase::Failed))
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .run();
}

#[test]
fn protected_secret_host_failure_releases_the_send_slot() {
    scenario("protected storage failure stops before persistence")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(secret().host_fails())
        .when(call("failed", send().to(own_address()).grams(1)))
        .then(error(
            "failed",
            send_failed("protected-secret host failure (Other): scripted protected secret failure"),
        ))
        .then(snapshot().send_phase(SendPhase::Failed))
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .given(submission().paused("retry-submit"))
        .when(start("retry", send().to(own_address()).grams(1)))
        .then(send_phase("retry", SendPhase::Submitting))
        .when(resume("retry-submit", submission_accepted()))
        .then(result("retry").submitted())
        .run();
}

#[test]
fn amount_larger_than_the_fresh_balance_fails_before_signing() {
    scenario("a transfer cannot exceed the fresh wallet balance")
        .given(wallet().active().balance(grams(1)).seqno(7))
        .when(call("too-large", send().to(own_address()).grams(100)))
        .then(error(
            "too-large",
            WalletClientError::InsufficientBalance {
                available_nanograms: grams(1).as_nanograms(),
                requested_nanograms: grams(100).as_nanograms(),
            },
        ))
        .then(snapshot().send_phase(SendPhase::Failed))
        // The failure happens before authorization, signing, persistence, and submission.
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        // The rejected attempt must release the in-memory send slot.
        .given(wallet().active().balance(grams(2)).seqno(7))
        .given(submission().paused("retry-submit"))
        .when(start("retry", send().to(own_address()).grams(1)))
        .then(send_phase("retry", SendPhase::Submitting))
        .when(resume("retry-submit", submission_accepted()))
        .then(result("retry").submitted())
        .run();
}

#[test]
fn missing_provider_time_prevents_transfer_expiration() {
    scenario("fresh account state must include provider time")
        .given(
            wallet()
                .active()
                .balance(grams(10))
                .seqno(7)
                .without_sync_time(),
        )
        .when(call("send", send().to(own_address()).grams(1)))
        .then(error(
            "send",
            send_failed("fresh account state did not include provider synchronization time"),
        ))
        .then(snapshot().send_phase(SendPhase::Failed))
        .run();
}

#[test]
fn provider_time_must_fit_the_wallet_timestamp_field() {
    scenario("provider time outside u32 cannot be signed")
        .given(
            wallet()
                .active()
                .balance(grams(10))
                .seqno(7)
                .sync_time(u64::from(u32::MAX) + 1),
        )
        .when(call("send", send().to(own_address()).grams(1)))
        .then(error(
            "send",
            send_failed("provider synchronization time does not fit the wallet timestamp field"),
        ))
        .then(snapshot().send_phase(SendPhase::Failed))
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .run();
}

#[test]
fn transfer_expiration_overflow_fails_before_persistence() {
    scenario("provider time plus validity must fit u32")
        .given(
            wallet()
                .active()
                .balance(grams(10))
                .seqno(7)
                .sync_time(u64::from(u32::MAX)),
        )
        .when(call("send", send().to(own_address()).grams(1)))
        .then(error(
            "send",
            send_failed("transfer expiration timestamp overflow"),
        ))
        .then(snapshot().send_phase(SendPhase::Failed))
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .run();
}

#[test]
fn journal_conflict_releases_the_in_memory_send_slot() {
    scenario("a journal CAS conflict does not leave the client busy")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(journal().conflicts_on_next_write())
        .when(call("conflict", send().to(own_address()).grams(1)))
        .then(error("conflict", WalletClientError::SendAlreadyInProgress))
        .then(snapshot().send_phase(SendPhase::Failed))
        .given(submission().paused("retry-submit"))
        .when(start("retry", send().to(own_address()).grams(1)))
        .then(send_phase("retry", SendPhase::Submitting))
        .when(resume("retry-submit", submission_accepted()))
        .then(result("retry").submitted())
        .run();
}

#[test]
fn journal_load_failure_stops_before_network_or_secret_access() {
    scenario("send cannot start when its durable wallet slot cannot be read")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(journal().load_fails())
        .when(call("send", send().to(own_address()).grams(1)))
        .then(error(
            "send",
            send_failed("journal host failure (Unavailable): scripted journal load failure"),
        ))
        .then(snapshot().send_phase(SendPhase::Failed))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .run();
}

#[test]
fn prepared_journal_write_failure_is_submission_unknown_without_http_submit() {
    scenario("failure at the durable boundary preserves an unknown outcome")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(journal().write_fails(1))
        .when(call("send", send().to(own_address()).grams(1)))
        .then(error(
            "send",
            WalletClientError::SubmissionUnknown {
                diagnostic: "journal host failure (Unavailable): scripted journal write 1 failure"
                    .to_owned(),
            },
        ))
        .then(snapshot().send_phase(SendPhase::SubmissionUnknown))
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .run();
}

#[test]
fn terminal_journal_write_failure_is_unknown_after_http_submit() {
    scenario("provider acceptance without terminal persistence remains unknown")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(journal().write_fails(2))
        .when(call("send", send().to(own_address()).grams(1)))
        .then(error(
            "send",
            WalletClientError::SubmissionUnknown {
                diagnostic: "journal host failure (Unavailable): scripted journal write 2 failure"
                    .to_owned(),
            },
        ))
        .then(snapshot().send_phase(SendPhase::SubmissionUnknown))
        .then(message_was_submitted())
        .run();
}

#[test]
fn malformed_success_response_is_submission_unknown() {
    scenario("a malformed success response cannot prove whether submission happened")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(submission().paused("submit"))
        .when(start("send", send().to(own_address()).grams(1)))
        .then(send_phase("send", SendPhase::Submitting))
        .when(resume("submit", submission_malformed()))
        .then(result("send").submission_unknown())
        .then(snapshot().send_error_message("invalid sendBoc success response"))
        // The exact BOC is already durable. A new signature could double-spend
        // if the provider accepted the malformed-response submission.
        .when(call("replacement", send().to(own_address()).grams(2)))
        .then(error(
            "replacement",
            WalletClientError::PreviousSubmissionUnresolved,
        ))
        .run();
}

#[test]
fn provider_rate_limit_is_a_definite_rejection_and_allows_retry() {
    scenario("an HTTP 429 means the provider definitely rejected sendBoc")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(submission().paused("rate-limited-submit"))
        .when(start("rate-limited", send().to(own_address()).grams(1)))
        .then(send_phase("rate-limited", SendPhase::Submitting))
        .when(resume(
            "rate-limited-submit",
            submission_http_failure(429, "send rate limited"),
        ))
        .then(result("rate-limited").failed())
        .then(snapshot().send_error_message("send rate limited"))
        // A definite rejection releases both the in-memory and durable slots.
        .given(submission().paused("retry-submit"))
        .when(start("retry", send().to(own_address()).grams(2)))
        .then(send_phase("retry", SendPhase::Submitting))
        .when(resume("retry-submit", submission_accepted()))
        .then(result("retry").submitted())
        .run();
}

#[test]
fn provider_server_failure_is_submission_unknown() {
    scenario("a provider server failure after POST has an ambiguous outcome")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(submission().paused("submit"))
        .when(start("send", send().to(own_address()).grams(1)))
        .then(send_phase("send", SendPhase::Submitting))
        .when(resume(
            "submit",
            submission_http_failure(503, "provider unavailable"),
        ))
        .then(result("send").submission_unknown())
        .then(snapshot().send_error_message("provider unavailable"))
        .when(call("replacement", send().to(own_address()).grams(2)))
        .then(error(
            "replacement",
            WalletClientError::PreviousSubmissionUnresolved,
        ))
        .run();
}
