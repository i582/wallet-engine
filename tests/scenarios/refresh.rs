use super::support::*;
use wallet_engine::{
    AccountStatus, DomainError, ErrorCategory, ErrorCode, HttpHostErrorKind, ResourcePhase,
    RetryAdvice,
};

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
fn account_timeout_keeps_the_independent_activity_result() {
    scenario("a timed-out account request does not discard activity")
        .given(provider().account_times_out())
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").partially_completed())
        .then(
            snapshot()
                .account_error(DomainError {
                    code: ErrorCode::TransportFailed,
                    category: ErrorCategory::Transport,
                    retry: RetryAdvice::Safe,
                    developer_message: "scripted account transport failure".to_owned(),
                    provider_status: None,
                    retry_after_ms: None,
                    host_kind: Some(HttpHostErrorKind::Timeout),
                })
                .activity_phase(ResourcePhase::Ready),
        )
        .run();
}

#[test]
fn refresh_fails_when_both_provider_requests_time_out() {
    scenario("refresh fails when Toncenter does not answer either request")
        .given(provider().account_times_out().activity_times_out())
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
fn localnet_loads_newly_confirmed_transactions() {
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

#[test]
fn localnet_includes_transactions_confirmed_between_refresh_resource_requests() {
    scenario("activity observes transactions confirmed while refresh is in flight")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("deploy", send().to(own_address()).nanograms(1)))
        .then(result("deploy").submitted())
        .when(call("baseline", refresh_wallet()))
        .then(update("baseline").completed())
        .then(remember_activity_as("A"))
        // The activity HTTP request exists, but the localnet has not served it.
        .when(pause_next_activity_request("activity-head"))
        .when(start("in-flight-refresh", refresh_wallet()))
        .when(wait_for_request("activity-head"))
        // Confirm real wallet transactions while refresh owns a Loading snapshot.
        .when(call("new-transactions", spam_transfers(3)))
        .then(succeeds("new-transactions"))
        .then(on_chain_wallet().active().seqno(4))
        .when(release_request("activity-head"))
        .then(update("in-flight-refresh").completed())
        .then(remember_new_activity_as("B"))
        .then(activity_is(&["A", "B"]))
        .run();
}

#[test]
fn localnet_cancelled_refresh_discards_a_real_head_change_until_retry() {
    scenario("cancelled refresh cannot publish localnet transactions from a late response")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("deploy", send().to(own_address()).nanograms(1)))
        .then(result("deploy").submitted())
        .when(call("baseline", refresh_wallet()))
        .then(update("baseline").completed())
        .then(remember_activity_as("A"))
        .when(pause_next_activity_request("late-head"))
        .when(start("cancelled-refresh", refresh_wallet()))
        .when(wait_for_request("late-head"))
        .when(call("new-transactions", spam_transfers(3)))
        .then(succeeds("new-transactions"))
        .then(on_chain_wallet().active().seqno(4))
        .when(call("cancel", cancel_refresh()))
        .then(succeeds("cancel"))
        .then(request_was_cancelled("late-head"))
        .then(remember_revision("after-cancel"))
        .when(release_request("late-head"))
        .then(update("cancelled-refresh").superseded())
        .then(revision_is("after-cancel"))
        .then(activity_is(&["A"]))
        // A new generation must load the chain change that the cancelled one ignored.
        .when(call("retry", refresh_wallet()))
        .then(update("retry").completed())
        .then(remember_new_activity_as("B"))
        .then(activity_is(&["A", "B"]))
        .run();
}

#[test]
fn preserves_provider_retry_advice_from_rate_limiting() {
    scenario("refresh exposes provider backoff as structured resource state")
        .given(provider().account_is_rate_limited(7))
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").partially_completed())
        .then(
            snapshot()
                .account_phase(ResourcePhase::Failed)
                .account_error(DomainError {
                    code: ErrorCode::RateLimited,
                    category: ErrorCategory::RateLimit,
                    retry: RetryAdvice::AfterDelay,
                    developer_message: "account rate limited".to_owned(),
                    provider_status: Some(429),
                    retry_after_ms: Some(7_000),
                    host_kind: None,
                })
                .activity_phase(ResourcePhase::Ready),
        )
        .run();
}

#[test]
fn malformed_activity_json_is_a_non_retryable_protocol_error() {
    scenario("malformed provider JSON cannot become an empty activity page")
        .given(provider().activity_returns_malformed_json())
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").partially_completed())
        .then(
            snapshot()
                .account_phase(ResourcePhase::Ready)
                .activity_error(DomainError {
                    code: ErrorCode::InvalidProviderResponse,
                    category: ErrorCategory::ProviderProtocol,
                    retry: RetryAdvice::None,
                    developer_message: "expected ident at line 1 column 2".to_owned(),
                    provider_status: None,
                    retry_after_ms: None,
                    host_kind: None,
                }),
        )
        .run();
}

#[test]
fn redirected_account_response_is_rejected_by_the_engine_boundary() {
    scenario("the host cannot redirect a wallet request to another URL")
        .given(provider().account_redirects())
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").partially_completed())
        .then(
            snapshot()
                .account_error(DomainError {
                    code: ErrorCode::HostPolicyViolation,
                    category: ErrorCategory::HostPolicy,
                    retry: RetryAdvice::None,
                    developer_message: "HTTP redirect or mismatched final URL".to_owned(),
                    provider_status: None,
                    retry_after_ms: None,
                    host_kind: Some(HttpHostErrorKind::PolicyViolation),
                })
                .activity_phase(ResourcePhase::Ready),
        )
        .run();
}
