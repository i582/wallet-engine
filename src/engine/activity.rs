//! Activity refresh and pagination helpers.

use std::collections::HashMap;

use crate::provider::{ActivityPage, ActivityPageCursor, activity_record_order};
use crate::{
    ErrorCode, HttpCall, HttpCallId, ResourcePhase, ResourceState, WalletClientConfig,
    WalletClientError, WalletOperationOutcome, WalletUpdate,
};

use super::WalletClient;
use super::http::build_toncenter_request;
use super::http::evaluate_for_call;
use super::state::{OperationFamily, State, ensure_running, update};

#[uniffi::export]
impl WalletClient {
    /// Loads the next older activity page and merges unique items by item ID.
    ///
    /// The method returns `Skipped` during refresh, during another page load,
    /// or when no advancing cursor exists. A page must move to an older logical
    /// time. Otherwise pagination stops and adds no items.
    pub async fn load_more_activity(&self) -> Result<WalletUpdate, WalletClientError> {
        let (generation, call) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;

            if state.active_refresh.is_some()
                || state.active_pagination.is_some()
                || !state.activity_has_more
            {
                return Ok(update(WalletOperationOutcome::Skipped, 0, &state));
            }

            let Some(cursor) = state.activity_cursor.clone() else {
                return Ok(update(WalletOperationOutcome::Skipped, 0, &state));
            };

            let id = HttpCallId {
                value: state.allocate_id()?,
            };
            let call = build_activity_page_request(&state.config, &cursor, id)?;

            state.pagination_generation = state
                .pagination_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.pagination_generation;
            state.active_pagination = Some((generation, id));
            state.snapshot.activity_pagination_resource = ResourceState::loading();
            state.next_revision()?;

            (generation, call)
        };

        let result = evaluate_for_call(
            &call,
            self.http_host.execute_http(call.clone()).await,
            |body| crate::provider::parse_activity(body, PAGE_SIZE),
        );

        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Pagination, generation) {
            return Ok(update(WalletOperationOutcome::Superseded, 0, &state));
        }

        state.active_pagination = None;

        let (outcome, added) = match result {
            Ok(page) => {
                let added = apply_activity_page(&mut state, page);
                state.snapshot.activity_pagination_resource = ResourceState::ready();
                (WalletOperationOutcome::Completed, added)
            }
            Err(error) if error.code == ErrorCode::HostCancelled => {
                state.snapshot.activity_pagination_resource = ResourceState::idle();
                (WalletOperationOutcome::Cancelled, 0)
            }
            Err(error) => {
                state.snapshot.activity_pagination_resource = ResourceState::failed(error);
                (WalletOperationOutcome::Failed, 0)
            }
        };

        state.next_revision()?;

        Ok(update(outcome, added, &state))
    }

    /// Cancels the active activity page load.
    ///
    /// This method has no effect when no page load is active.
    pub async fn cancel_load_more_activity(&self) -> Result<(), WalletClientError> {
        let call = {
            let mut state = self.lock()?;
            let call = state.active_pagination.take().map(|active| active.1);
            if call.is_some() {
                state.snapshot.activity_pagination_resource = ResourceState::idle();
                state.next_revision()?;
            }

            call
        };

        if let Some(call_id) = call {
            self.http_host.cancel_http(call_id).await;
        }

        Ok(())
    }
}

pub(super) const PAGE_SIZE: u32 = 10;

pub(super) fn mark_loading_cancelled(resource: &mut ResourceState) {
    if resource.phase == ResourcePhase::Loading {
        *resource = ResourceState::idle();
    }
}

pub(super) fn apply_activity_page(state: &mut State, page: ActivityPage) -> u64 {
    // Toncenter pages move from newer to older logical times. Comparing the
    // retained BigUint values makes this check exact for every valid LT size.
    let advanced = match (state.activity_cursor.as_ref(), page.cursor.as_ref()) {
        (Some(previous), Some(next)) => next.logical_time < previous.logical_time,
        _ => false,
    };
    if !advanced {
        state.activity_has_more = false;
        state.sync_activity_snapshot();
        return 0;
    }

    let previous_len = state.activity.len();
    let mut by_id: HashMap<_, _> = state
        .activity
        .drain(..)
        .map(|item| (item.id.clone(), item))
        .collect();

    for item in page.items {
        by_id.insert(item.id.clone(), item);
    }

    state.activity = by_id.into_values().collect();
    state.activity.sort_by(activity_record_order);
    state.activity_cursor = page.cursor;
    state.activity_has_more = page.has_more;
    state.sync_activity_snapshot();

    u64::try_from(state.activity.len().saturating_sub(previous_len)).unwrap_or(u64::MAX)
}

pub(super) fn build_refresh_requests(
    config: &WalletClientConfig,
    account_id: HttpCallId,
    activity_id: HttpCallId,
) -> Result<(HttpCall, HttpCall), WalletClientError> {
    Ok((
        build_toncenter_request(
            config,
            account_id,
            "getAddressInformation",
            &[("address", config.address.as_str())],
        )?,
        build_toncenter_request(
            config,
            activity_id,
            "getTransactions",
            &[("address", config.address.as_str()), ("limit", "10")],
        )?,
    ))
}

pub(super) fn build_activity_page_request(
    config: &WalletClientConfig,
    cursor: &ActivityPageCursor,
    id: HttpCallId,
) -> Result<HttpCall, WalletClientError> {
    if cursor.hash.is_empty() {
        return Err(WalletClientError::InvalidConfig);
    }

    let logical_time = cursor.logical_time.to_string();

    build_toncenter_request(
        config,
        id,
        "getTransactions",
        &[
            ("address", config.address.as_str()),
            ("limit", "10"),
            ("lt", logical_time.as_str()),
            ("hash", cursor.hash.as_str()),
        ],
    )
}
