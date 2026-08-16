use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use wallet_engine::ResourcePhase;

use super::support::*;

#[derive(Clone, Copy, Debug)]
struct RefreshScheduleStep {
    release_account_first: bool,
    one_response_completes_before_cancel: bool,
}

fn refresh_schedule() -> impl Strategy<Value = Vec<RefreshScheduleStep>> {
    prop::collection::vec(
        (any::<bool>(), any::<bool>()).prop_map(
            |(release_account_first, one_response_completes_before_cancel)| RefreshScheduleStep {
                release_account_first,
                one_response_completes_before_cancel,
            },
        ),
        1..=4,
    )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
            "proptest-regressions",
        ))),
        ..ProptestConfig::default()
    })]

    #[test]
    #[ignore = "run explicitly with `just proptest-rust`"]
    fn cancelled_refreshes_restore_stable_snapshot_across_generated_schedules(
        schedule in refresh_schedule(),
    ) {
        let mut test = scenario("cancelled refresh restores stable data across late schedules")
            .given(wallet().active().balance(grams(10)).seqno(7))
            .given(activity_pages(&[3, 3]))
            .when(call("baseline", refresh_wallet()))
            .then(update("baseline").completed())
            .then(
                snapshot()
                    .account_phase(ResourcePhase::Ready)
                    .activity_count(3)
                    .activity_phase(ResourcePhase::Ready),
            )
            .then(remember_snapshot("stable"));

        for (index, step) in schedule.into_iter().enumerate() {
            let refresh = format!("refresh-{index}");
            let cancel = format!("cancel-{index}");
            let account = format!("account-{index}");
            let activity = format!("activity-{index}");
            let after_cancel = format!("after-cancel-{index}");
            let (first, second) = if step.release_account_first {
                (&account, &activity)
            } else {
                (&activity, &account)
            };

            test = test
                .when(pause_next_account_request(account.clone()))
                .when(pause_next_activity_request(activity.clone()))
                .when(start(refresh.clone(), refresh_wallet()))
                .when(wait_for_request(account.clone()))
                .when(wait_for_request(activity.clone()));

            if step.one_response_completes_before_cancel {
                test = test.when(release_request(first));
            }

            test = test
                .when(call(cancel.clone(), cancel_refresh()))
                .then(succeeds(cancel))
                .then(request_was_cancelled(account.clone()))
                .then(request_was_cancelled(activity.clone()))
                .then(remember_revision(after_cancel.clone()))
                .then(snapshot_is_except_revision("stable"));

            if !step.one_response_completes_before_cancel {
                test = test.when(release_request(first));
            }

            test = test
                .when(release_request(second))
                .then(update(refresh).superseded())
                .then(revision_is(after_cancel))
                .then(snapshot_is_except_revision("stable"));
        }

        let result = test.when(call("retry", refresh_wallet()))
            .then(update("retry").completed())
            .then(
                snapshot()
                    .account_phase(ResourcePhase::Ready)
                    .activity_phase(ResourcePhase::Ready),
            )
            .run_result();
        if let Err(failure) = result {
            prop_assert!(false, "{failure}");
        }
    }
}
