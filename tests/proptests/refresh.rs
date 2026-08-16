use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use wallet_engine::ResourcePhase;

use super::support::*;

#[derive(Clone, Copy, Debug)]
struct RefreshScheduleStep {
    account_completes_first: bool,
    one_response_completes_before_cancel: bool,
}

fn refresh_schedule() -> impl Strategy<Value = Vec<RefreshScheduleStep>> {
    prop::collection::vec(
        (any::<bool>(), any::<bool>()).prop_map(
            |(account_completes_first, one_response_completes_before_cancel)| RefreshScheduleStep {
                account_completes_first,
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
    fn cancelled_refreshes_ignore_generated_late_response_schedules(
        schedule in refresh_schedule(),
    ) {
        let mut test = scenario("cancelled refresh ignores generated late response schedules");

        for (index, step) in schedule.into_iter().enumerate() {
            let refresh = format!("refresh-{index}");
            let cancel = format!("cancel-{index}");
            let account = format!("account-{index}");
            let activity = format!("activity-{index}");
            let after_cancel = format!("after-cancel-{index}");
            let (first, second) = if step.account_completes_first {
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
                .then(
                    snapshot()
                        .account_phase(ResourcePhase::Idle)
                        .activity_count(0)
                        .activity_phase(ResourcePhase::Idle),
                );

            if !step.one_response_completes_before_cancel {
                test = test.when(release_request(first));
            }

            test = test
                .when(release_request(second))
                .then(update(refresh).superseded())
                .then(revision_is(after_cancel))
                .then(
                    snapshot()
                        .account_phase(ResourcePhase::Idle)
                        .activity_count(0)
                        .activity_phase(ResourcePhase::Idle),
                );
        }

        test.when(call("retry", refresh_wallet()))
            .then(update("retry").completed())
            .then(
                snapshot()
                    .account_phase(ResourcePhase::Ready)
                    .activity_phase(ResourcePhase::Ready),
            )
            .run();
    }
}
