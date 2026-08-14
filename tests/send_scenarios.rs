mod support;

use support::*;
use wallet_engine::{SendPhase, WalletClientError};

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
