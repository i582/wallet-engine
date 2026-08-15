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
