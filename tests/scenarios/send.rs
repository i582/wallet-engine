use super::support::*;
use wallet_engine::{AccountStatus, SendPhase, WalletClientError};

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
        .when(call("send", send().to(own_address()).grams(1)))
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
