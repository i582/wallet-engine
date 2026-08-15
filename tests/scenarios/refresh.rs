use super::support::*;
use wallet_engine::{AccountStatus, ResourcePhase};

#[test]
fn publishes_account_and_activity_together() {
    scenario("refresh loads independent wallet resources")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .then(account_status(AccountStatus::Active))
        .then(
            snapshot()
                .account_phase(ResourcePhase::Ready)
                .activity_phase(ResourcePhase::Ready),
        )
        .run();
}

#[test]
fn keeps_account_when_activity_fails() {
    scenario("activity failure does not discard refreshed account")
        .given(wallet().active().balance(grams(10)).seqno(7))
        .given(provider().activity_fails(503))
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").partially_completed())
        .then(account_status(AccountStatus::Active))
        .then(
            snapshot()
                .account_phase(ResourcePhase::Ready)
                .activity_phase(ResourcePhase::Failed),
        )
        .run();
}

#[test]
fn keeps_activity_resource_when_account_fails() {
    scenario("account failure does not discard refreshed activity")
        .given(provider().account_fails(503))
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").partially_completed())
        .then(
            snapshot()
                .account_phase(ResourcePhase::Failed)
                .activity_phase(ResourcePhase::Ready),
        )
        .run();
}

#[test]
fn reports_failure_when_both_resources_fail() {
    scenario("refresh fails when every requested resource fails")
        .given(provider().account_fails(503).activity_fails(503))
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").failed())
        .then(
            snapshot()
                .account_phase(ResourcePhase::Failed)
                .activity_phase(ResourcePhase::Failed),
        )
        .run();
}

#[test]
fn cancellation_discards_late_refresh_responses_and_allows_retry() {
    scenario("cancelled refresh responses cannot publish after cancellation")
        .when(pause_next_account_request("account-response"))
        .when(start("refresh", refresh_wallet()))
        .when(wait_for_request("account-response"))
        .when(call("cancel", cancel_refresh()))
        .then(succeeds("cancel"))
        .then(request_was_cancelled("account-response"))
        .then(
            snapshot()
                .account_phase(ResourcePhase::Idle)
                .activity_count(0)
                .activity_phase(ResourcePhase::Idle),
        )
        .then(remember_revision("after-cancel"))
        // Deliver a successful response after cancellation. It must not publish.
        .when(release_request("account-response"))
        .then(update("refresh").superseded())
        .then(revision_is("after-cancel"))
        .then(
            snapshot()
                .account_phase(ResourcePhase::Idle)
                .activity_count(0)
                .activity_phase(ResourcePhase::Idle),
        )
        // A cancelled generation must not poison the next refresh.
        .when(call("retry", refresh_wallet()))
        .then(update("retry").completed())
        .then(
            snapshot()
                .account_phase(ResourcePhase::Ready)
                .activity_phase(ResourcePhase::Ready),
        )
        .run();
}

#[test]
fn refresh_stays_loading_until_both_provider_responses_finish() {
    scenario("refresh completes only after both resource requests finish")
        .when(pause_next_account_request("account-response"))
        .when(start("refresh", refresh_wallet()))
        .when(wait_for_request("account-response"))
        .then(
            snapshot()
                .account_phase(ResourcePhase::Loading)
                .activity_phase(ResourcePhase::Loading),
        )
        .when(release_request("account-response"))
        .then(update("refresh").completed())
        .then(
            snapshot()
                .account_phase(ResourcePhase::Ready)
                .activity_phase(ResourcePhase::Ready),
        )
        .run();
}

#[test]
fn loads_newly_confirmed_transactions_from_localnet() {
    scenario("refresh adds transactions confirmed after the previous snapshot")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("deploy", send().to(own_address()).nanograms(1)))
        .then(result("deploy").submitted())
        .when(call("initial-refresh", refresh_wallet()))
        .then(update("initial-refresh").completed())
        .then(remember_activity_as("A"))
        // Confirm new transfers only after the first immutable snapshot was captured.
        .when(call("new-transactions", spam_transfers(3)))
        .then(succeeds("new-transactions"))
        .then(on_chain_wallet().active().seqno(4))
        .when(call("latest-refresh", refresh_wallet()))
        .then(update("latest-refresh").completed())
        .then(remember_new_activity_as("B"))
        .then(activity_is(&["A", "B"]))
        .run();
}
