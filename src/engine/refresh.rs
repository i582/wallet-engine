//! Account and first-page activity refresh operation.

use futures::future::join;

use crate::provider::{ActivityPage, parse_account, parse_activity};
use crate::{
    AccountSnapshot, DomainError, HttpRequestId, ResourcePhase, ResourceState, WalletClientError,
    WalletOperationOutcome, WalletUpdate,
};

use super::WalletClient;
use super::activity::{PAGE_SIZE, build_refresh_requests, mark_loading_cancelled};
use super::http::evaluate_response;
use super::state::{OperationFamily, ensure_running, update};

#[uniffi::export]
impl WalletClient {
    /// Refreshes account and first-page activity data concurrently.
    ///
    /// Each resource publishes independently. One request can succeed while
    /// the other fails, which produces [`WalletOperationOutcome::PartiallyCompleted`].
    /// A newer refresh supersedes the older refresh and cancels its host requests.
    pub async fn refresh(&self) -> Result<WalletUpdate, WalletClientError> {
        let (generation, requests, previous_request_ids) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;

            let config = state.config.clone();
            let account_id = HttpRequestId {
                value: state.allocate_id()?,
            };
            let activity_id = HttpRequestId {
                value: state.allocate_id()?,
            };

            let requests = build_refresh_requests(&config, account_id, activity_id)?;

            state.refresh_generation = state
                .refresh_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.refresh_generation;

            let mut previous_request_ids = state
                .active_refresh
                .replace((generation, vec![account_id, activity_id]))
                .map(|active| active.1)
                .unwrap_or_default();
            if let Some((_, page_request_id)) = state.active_pagination.take() {
                previous_request_ids.push(page_request_id);
            }

            state.snapshot.account_resource = ResourceState::loading();
            state.snapshot.activity_resource = ResourceState::loading();
            state.snapshot.activity_pagination_resource = ResourceState::idle();
            state.next_revision()?;

            (generation, requests, previous_request_ids)
        };

        for request_id in previous_request_ids {
            self.http_host.cancel_http(request_id).await;
        }

        let (account, activity) = join(
            self.http_host.execute_http(requests.0.clone()),
            self.http_host.execute_http(requests.1.clone()),
        )
        .await;

        let account = evaluate_response(&requests.0, account, parse_account);
        self.publish_refresh_component(generation, RefreshValue::Account(account))?;

        let activity = evaluate_response(&requests.1, activity, |body| {
            parse_activity(body, PAGE_SIZE)
        });
        self.publish_refresh_component(generation, RefreshValue::Activity(activity))?;

        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Refresh, generation) {
            return Ok(update(WalletOperationOutcome::Superseded, 0, &state));
        }

        state.active_refresh = None;

        let failed = [
            &state.snapshot.account_resource,
            &state.snapshot.activity_resource,
        ]
        .into_iter()
        .filter(|resource| resource.phase == ResourcePhase::Failed)
        .count();

        let outcome = match failed {
            0 => WalletOperationOutcome::Completed,
            2 => WalletOperationOutcome::Failed,
            _ => WalletOperationOutcome::PartiallyCompleted,
        };

        Ok(update(outcome, 0, &state))
    }

    /// Cancels the active refresh and requests cancellation of its HTTP requests.
    ///
    /// This method has no effect when no refresh is active.
    pub async fn cancel_refresh(&self) -> Result<(), WalletClientError> {
        let request_ids = {
            let mut state = self.lock()?;
            let request_ids = state
                .active_refresh
                .take()
                .map(|active| active.1)
                .unwrap_or_default();
            if !request_ids.is_empty() {
                mark_loading_cancelled(&mut state.snapshot.account_resource);
                mark_loading_cancelled(&mut state.snapshot.activity_resource);
                state.next_revision()?;
            }

            request_ids
        };

        for request_id in request_ids {
            self.http_host.cancel_http(request_id).await;
        }

        Ok(())
    }
}

impl WalletClient {
    fn publish_refresh_component(
        &self,
        generation: u64,
        value: RefreshValue,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Refresh, generation) {
            return Ok(());
        }

        match value {
            RefreshValue::Account(result) => match result {
                Ok(account) => {
                    state.snapshot.account = Some(account);
                    state.snapshot.account_resource = ResourceState::ready();
                }
                Err(error) => state.snapshot.account_resource = ResourceState::failed(error),
            },
            RefreshValue::Activity(result) => match result {
                Ok(page) => {
                    state.activity = page.items;
                    state.activity_cursor = page.cursor;
                    state.activity_has_more = page.has_more;
                    state.sync_activity_snapshot();
                    state.snapshot.activity_resource = ResourceState::ready();
                }
                Err(error) => state.snapshot.activity_resource = ResourceState::failed(error),
            },
        }

        state.next_revision()?;

        Ok(())
    }
}

enum RefreshValue {
    Account(Result<AccountSnapshot, DomainError>),
    Activity(Result<ActivityPage, DomainError>),
}
