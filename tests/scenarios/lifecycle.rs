use super::support::*;
use wallet_engine::{ResourcePhase, SendPhase, WalletClientError};

#[test]
fn shutdown_cancels_refresh_and_rejects_new_work() {
    scenario("shutdown cancels active HTTP and permanently closes the client")
        .when(pause_next_account_request("account-response"))
        .when(start("refresh", refresh_wallet()))
        .when(wait_for_request("account-response"))
        .when(call("shutdown", shutdown_client()))
        .then(succeeds("shutdown"))
        .then(request_was_cancelled("account-response"))
        .then(remember_revision("after-shutdown"))
        .when(release_request("account-response"))
        .then(update("refresh").superseded())
        .then(revision_is("after-shutdown"))
        .then(
            snapshot()
                .account_phase(ResourcePhase::Idle)
                .activity_count(0)
                .activity_phase(ResourcePhase::Idle),
        )
        .when(call("refresh-after-shutdown", refresh_wallet()))
        .then(error("refresh-after-shutdown", WalletClientError::Shutdown))
        // Shutdown is safe to repeat during host teardown.
        .when(call("shutdown-again", shutdown_client()))
        .then(succeeds("shutdown-again"))
        .run();
}

#[test]
fn shutdown_releases_snapshot_waiters() {
    scenario("shutdown releases clients waiting for a newer snapshot revision")
        .when(start("waiter", wait_for_change(0)))
        .when(call("shutdown", shutdown_client()))
        .then(succeeds("shutdown"))
        .then(error("waiter", WalletClientError::Shutdown))
        .run();
}

#[test]
fn wait_for_change_returns_an_already_published_snapshot_immediately() {
    scenario("a caller behind the current revision does not register a waiter")
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .then(remember_revision("refreshed"))
        .when(call("wait", wait_for_change(0)))
        .then(returned_snapshot_revision_is("wait", "refreshed"))
        .run();
}

#[test]
fn wait_for_change_is_released_by_a_newly_published_revision() {
    scenario("a pending snapshot waiter is released by the next state publication")
        .when(start("wait", wait_for_change(0)))
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .then(returned_snapshot_revision_is_greater_than("wait", 0))
        .run();
}

#[test]
fn a_future_snapshot_waiter_survives_insufficient_revisions() {
    scenario("a waiter completes only after revision advances past its requested value")
        .when(start("wait", wait_for_change(5)))
        // The first refresh ends at revision three. Returning here would violate
        // the caller's `after_revision` contract.
        .when(call("first-refresh", refresh_wallet()))
        .then(update("first-refresh").completed())
        // A later refresh advances beyond five and releases the retained waiter.
        .when(call("second-refresh", refresh_wallet()))
        .then(update("second-refresh").completed())
        .then(returned_snapshot_revision_is_greater_than("wait", 5))
        .run();
}

#[test]
fn wait_for_change_rejects_calls_after_shutdown_without_registering_a_waiter() {
    scenario("a closed client rejects new snapshot waiters immediately")
        .when(call("shutdown", shutdown_client()))
        .then(succeeds("shutdown"))
        .when(call("wait", wait_for_change(0)))
        .then(error("wait", WalletClientError::Shutdown))
        .run();
}

#[test]
fn shutdown_cancels_a_send_before_the_durable_boundary() {
    scenario("shutdown cancels a send that has not persisted a signed message")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(pause_next_account_request("fresh-account"))
        .when(start("send", send().to(own_address()).grams(1)))
        .when(wait_for_request("fresh-account"))
        .when(call("shutdown", shutdown_client()))
        .then(succeeds("shutdown"))
        .then(request_was_cancelled("fresh-account"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .then(remember_revision("after-shutdown"))
        // Even a successful late account response cannot restart signing.
        .when(release_request("fresh-account"))
        .then(error("send", WalletClientError::Shutdown))
        .then(revision_is("after-shutdown"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .run();
}

#[test]
fn shutdown_during_journal_load_discards_the_late_record() {
    scenario("shutdown while loading durable send state cannot resume the send")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(pause_next_journal_load("journal-load"))
        .when(start("send", send().to(own_address()).grams(1)))
        .when(wait_for_platform_call("journal-load"))
        .when(call("shutdown", shutdown_client()))
        .then(succeeds("shutdown"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .then(remember_revision("after-shutdown"))
        .when(release_platform_call("journal-load"))
        .then(error("send", WalletClientError::Shutdown))
        .then(revision_is("after-shutdown"))
        .run();
}

#[test]
fn shutdown_during_seqno_fetch_cancels_and_discards_the_request() {
    scenario("shutdown while fetching seqno cannot continue to signing")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(pause_next_seqno_request("seqno"))
        .when(start("send", send().to(own_address()).grams(1)))
        .when(wait_for_request("seqno"))
        .when(call("shutdown", shutdown_client()))
        .then(succeeds("shutdown"))
        .then(request_was_cancelled("seqno"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .then(protected_secret_was_not_read())
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .then(remember_revision("after-shutdown"))
        .when(release_request("seqno"))
        .then(error("send", WalletClientError::Shutdown))
        .then(revision_is("after-shutdown"))
        .run();
}

#[test]
fn shutdown_during_secret_read_discards_the_authorized_bytes() {
    scenario("shutdown while reading the secret cannot sign or persist")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(pause_next_secret_read("secret-read"))
        .when(start("send", send().to(own_address()).grams(1)))
        .when(wait_for_platform_call("secret-read"))
        .when(call("shutdown", shutdown_client()))
        .then(succeeds("shutdown"))
        .then(snapshot().send_phase(SendPhase::Cancelled))
        .then(journal_is_empty())
        .then(no_message_was_submitted())
        .then(remember_revision("after-shutdown"))
        .when(release_platform_call("secret-read"))
        .then(error("send", WalletClientError::Shutdown))
        .then(revision_is("after-shutdown"))
        .run();
}

#[test]
fn shutdown_waits_for_a_send_past_the_durable_boundary() {
    scenario("shutdown gracefully waits for a durable send")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(submission().paused("submit"))
        .when(start("send", send().to(own_address()).grams(1)))
        .then(send_phase("send", SendPhase::Submitting))
        .when(start("shutdown", shutdown_client()))
        .then(send_phase("send", SendPhase::Submitting))
        .when(resume("submit", submission_accepted()))
        .then(result("send").submitted())
        .then(succeeds("shutdown"))
        .when(call("refresh-after-shutdown", refresh_wallet()))
        .then(error("refresh-after-shutdown", WalletClientError::Shutdown))
        .run();
}
