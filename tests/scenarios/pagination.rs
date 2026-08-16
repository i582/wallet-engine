use super::support::*;
use wallet_engine::ResourcePhase;

#[test]
fn skips_when_activity_has_not_been_loaded() {
    scenario("pagination needs a cursor from the first activity page")
        .when(call("older", load_more_activity()))
        .then(update("older").skipped())
        .run();
}

#[test]
fn skips_after_an_empty_first_page() {
    scenario("an empty activity page has no older page")
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .when(call("older", load_more_activity()))
        .then(update("older").skipped())
        .run();
}

#[test]
fn appends_an_older_page_and_stops_after_a_short_page() {
    scenario("pagination appends unique older activity")
        .given(activity_pages(&[10, 3]))
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .then(snapshot().has_more(true))
        .when(call("older", load_more_activity()))
        .then(update("older").completed())
        .then(update("older").added_items(3))
        .then(snapshot().activity_count(13).has_more(false))
        .when(call("finished", load_more_activity()))
        .then(update("finished").skipped())
        .run();
}

#[test]
fn refresh_supersedes_an_in_flight_older_page() {
    scenario("refresh owns the activity head and supersedes older-page work")
        .given(activity_pages(&[10, 3, 10]))
        .when(call("initial", refresh_wallet()))
        .then(update("initial").completed())
        .then(remember_activity_as("A"))
        // Hold the older-page response after the provider has produced it.
        .when(pause_next_activity_request("older-response"))
        .when(start("older", load_more_activity()))
        .when(wait_for_request("older-response"))
        // Refresh cancels that exact request and starts a new head request.
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .then(request_was_cancelled("older-response"))
        .then(remember_new_activity_as("C"))
        .then(remember_revision("after-refresh"))
        // Simulate a transport that still delivers the cancelled successful response.
        .when(release_request("older-response"))
        .then(update("older").superseded())
        .then(revision_is("after-refresh"))
        .then(activity_is(&["A", "C"]))
        .run();
}

#[test]
fn cancelling_an_older_page_preserves_loaded_activity() {
    scenario("cancelled pagination cannot publish its late provider response")
        .given(activity_pages(&[10, 3]))
        .when(call("initial", refresh_wallet()))
        .then(update("initial").completed())
        .then(remember_activity_as("A"))
        .when(pause_next_activity_request("older-response"))
        .when(start("older", load_more_activity()))
        .when(wait_for_request("older-response"))
        .when(call("cancel", cancel_load_more_activity()))
        .then(succeeds("cancel"))
        .then(request_was_cancelled("older-response"))
        .then(remember_revision("after-cancel"))
        // The provider response succeeds after cancellation and must still be ignored.
        .when(release_request("older-response"))
        .then(update("older").superseded())
        .then(revision_is("after-cancel"))
        .then(activity_is(&["A"]))
        .run();
}

#[test]
fn provider_failure_keeps_the_loaded_page_and_exposes_a_retryable_footer() {
    scenario("a failed older-page request preserves the loaded pagination lineage")
        .given(activity_pages(&[10, 3]))
        .when(call("initial", refresh_wallet()))
        .then(update("initial").completed())
        .then(remember_activity_as("A"))
        .when(fail_next_activity_request(503))
        .when(call("older", load_more_activity()))
        .then(update("older").failed())
        .then(
            snapshot()
                .pagination_phase(ResourcePhase::Failed)
                .activity_count(10)
                .has_more(true),
        )
        .then(activity_is(&["A"]))
        .run();
}

#[test]
fn host_cancellation_returns_the_pagination_resource_to_idle() {
    scenario("transport cancellation does not look like a provider failure")
        .given(activity_pages(&[10, 3]))
        .when(call("initial", refresh_wallet()))
        .then(update("initial").completed())
        .then(remember_activity_as("A"))
        .when(cancel_next_activity_request_at_host())
        .when(call("older", load_more_activity()))
        .then(update("older").cancelled())
        .then(
            snapshot()
                .pagination_phase(ResourcePhase::Idle)
                .activity_count(10)
                .has_more(true),
        )
        .then(activity_is(&["A"]))
        .run();
}

#[test]
fn shutdown_cancels_pagination_without_discarding_loaded_activity() {
    scenario("shutdown cancels only the in-flight older page")
        .given(activity_pages(&[10, 5]))
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .then(remember_activity_as("head"))
        .when(pause_next_activity_request("older-page"))
        .when(start("older", load_more_activity()))
        .when(wait_for_request("older-page"))
        .when(call("shutdown", shutdown_client()))
        .then(succeeds("shutdown"))
        .then(request_was_cancelled("older-page"))
        .then(
            snapshot()
                .pagination_phase(ResourcePhase::Idle)
                .activity_count(10)
                .has_more(true),
        )
        .then(activity_is(&["head"]))
        .then(remember_revision("after-shutdown"))
        .when(release_request("older-page"))
        .then(update("older").superseded())
        .then(revision_is("after-shutdown"))
        .then(activity_is(&["head"]))
        .run();
}

#[test]
fn localnet_loads_older_real_transactions() {
    scenario("pagination loads older transactions from Acton localnet")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("deploy", send().to(own_address()).nanograms(1)))
        .then(result("deploy").submitted())
        .when(call("spam", spam_transfers(12)))
        .then(succeeds("spam"))
        .then(on_chain_wallet().active().seqno(13))
        // The first provider page must be full before the client exposes an older-page cursor.
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .then(snapshot().has_more(true))
        .when(call("older", load_more_activity()))
        .then(update("older").completed())
        .then(update("older").added_any_items())
        .run();
}

#[test]
fn localnet_keeps_the_original_cursor_when_new_transactions_arrive() {
    scenario("new transactions do not corrupt an in-progress pagination lineage")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("deploy", send().to(own_address()).nanograms(1)))
        .then(result("deploy").submitted())
        .when(call("initial-spam", spam_transfers(9)))
        .then(succeeds("initial-spam"))
        .then(on_chain_wallet().active().seqno(10))
        // Capture the cursor of the original first page before the network head changes.
        .when(call("initial-refresh", refresh_wallet()))
        .then(update("initial-refresh").completed())
        .then(snapshot().has_more(true))
        .then(remember_activity_cursor("original-page"))
        .then(remember_activity_as("A"))
        // These transactions are newer than the saved cursor and are intentionally not refreshed yet.
        .when(call("new-transactions", spam_transfers(3)))
        .then(succeeds("new-transactions"))
        .then(on_chain_wallet().active().seqno(13))
        // Loading older history must continue from the saved cursor, not from the new chain head.
        .when(call("older", load_more_activity()))
        .then(update("older").completed())
        .then(pagination_used_cursor("original-page"))
        .then(update("older").added_any_items())
        .then(remember_new_activity_as("B"))
        .then(activity_is(&["A", "B"]))
        // A normal refresh can now replace the head with the three newly confirmed transactions.
        .when(call("latest", refresh_wallet()))
        .then(update("latest").completed())
        .then(snapshot().has_more(true))
        .run();
}

#[test]
fn localnet_preserves_multiple_older_pages_when_new_transactions_are_refreshed() {
    scenario("refresh preserves multiple loaded pages while adding a new head")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("deploy", send().to(own_address()).nanograms(1)))
        .then(result("deploy").submitted())
        .when(call("history-spam", spam_transfers(14)))
        .then(succeeds("history-spam"))
        .then(on_chain_wallet().active().seqno(15))
        // Name the first page before extending the loaded history twice.
        .when(call("refresh-A", refresh_wallet()))
        .then(update("refresh-A").completed())
        .then(remember_activity_as("A"))
        .when(call("more-B", load_more_activity()))
        .then(update("more-B").completed())
        .then(update("more-B").added_any_items())
        .then(remember_new_activity_as("B"))
        .when(call("more-C", load_more_activity()))
        .then(update("more-C").completed())
        .then(update("more-C").added_any_items())
        .then(remember_new_activity_as("C"))
        .then(activity_is(&["A", "B", "C"]))
        // Refreshing the unchanged head must keep every older page already loaded in memory.
        .when(call("refresh-same-head", refresh_wallet()))
        .then(update("refresh-same-head").completed())
        .then(activity_is(&["A", "B", "C"]))
        // New head transactions must not discard either previously loaded older page.
        .when(call("new-head", spam_transfers(3)))
        .then(succeeds("new-head"))
        .then(on_chain_wallet().active().seqno(18))
        .when(call("refresh-D", refresh_wallet()))
        .then(update("refresh-D").completed())
        .then(remember_new_activity_as("D"))
        .then(activity_is(&["A", "B", "C", "D"]))
        .run();
}

#[test]
fn localnet_in_flight_older_page_keeps_its_cursor_when_the_chain_head_changes() {
    scenario("a localnet head change cannot retarget an in-flight older page")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("deploy", send().to(own_address()).nanograms(1)))
        .then(result("deploy").submitted())
        .when(call("history", spam_transfers(14)))
        .then(succeeds("history"))
        .then(on_chain_wallet().active().seqno(15))
        .when(call("head-A", refresh_wallet()))
        .then(update("head-A").completed())
        .then(remember_activity_cursor("cursor-A"))
        .then(remember_activity_as("A"))
        // Capture the actual older-page URL before changing the chain head.
        .when(pause_next_activity_request("older-page"))
        .when(start("more-B", load_more_activity()))
        .when(wait_for_request("older-page"))
        .when(call("new-head", spam_transfers(3)))
        .then(succeeds("new-head"))
        .then(on_chain_wallet().active().seqno(18))
        .when(release_request("older-page"))
        .then(update("more-B").completed())
        .then(pagination_used_cursor("cursor-A"))
        .then(update("more-B").added_any_items())
        .then(remember_new_activity_as("B"))
        .then(activity_is(&["A", "B"]))
        // A later head refresh adds the three new transactions and preserves
        // the older page returned by the request above.
        .when(call("head-C", refresh_wallet()))
        .then(update("head-C").completed())
        .then(remember_new_activity_as("C"))
        .then(activity_is(&["A", "B", "C"]))
        .run();
}

#[test]
fn localnet_refresh_supersedes_a_page_after_new_head_transactions_arrive() {
    scenario("new localnet head plus refresh supersedes an older-page request")
        .given(network().localnet())
        .given(wallet().uninitialized().balance(grams(10)))
        .when(call("deploy", send().to(own_address()).nanograms(1)))
        .then(result("deploy").submitted())
        .when(call("history", spam_transfers(14)))
        .then(succeeds("history"))
        .when(call("head-A", refresh_wallet()))
        .then(update("head-A").completed())
        .then(remember_activity_as("A"))
        .when(pause_next_activity_request("older-page"))
        .when(start("more", load_more_activity()))
        .when(wait_for_request("older-page"))
        // A different client changes the real chain while the old cursor request waits.
        .when(call("new-head", spam_transfers(3)))
        .then(succeeds("new-head"))
        .then(on_chain_wallet().active().seqno(18))
        // Refresh owns the new head and must cancel the exact older-page request.
        .when(call("refresh", refresh_wallet()))
        .then(update("refresh").completed())
        .then(request_was_cancelled("older-page"))
        .then(remember_new_activity_as("B"))
        .then(remember_revision("after-refresh"))
        .when(release_request("older-page"))
        .then(update("more").superseded())
        .then(revision_is("after-refresh"))
        .then(activity_is(&["A", "B"]))
        .run();
}
