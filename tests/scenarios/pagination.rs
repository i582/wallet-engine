use super::support::*;

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
fn loads_older_real_transactions_from_localnet() {
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
fn keeps_the_original_cursor_when_new_transactions_arrive() {
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
fn preserves_multiple_older_pages_when_new_transactions_are_refreshed() {
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
