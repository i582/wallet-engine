//! Activity refresh and pagination helpers.

use std::collections::HashMap;

use crate::{
    ErrorCode, HttpRequest, HttpRequestId, ResourcePhase, ResourceState, WalletClientConfig,
    WalletClientError, WalletOperationOutcome, WalletUpdate,
};

use super::WalletClient;
use super::http::build_toncenter_request;
use super::http::evaluate_response;
use super::provider::{ActivityPage, ActivityPageCursor, activity_record_order, parse_activity};
use super::state::{OperationFamily, State, ensure_running, update};

#[uniffi::export]
impl WalletClient {
    /// Loads the next older activity page and merges unique items by item ID.
    ///
    /// The method returns `Skipped` during refresh, during another page load,
    /// or when no advancing cursor exists. A page must move to an older logical
    /// time. Otherwise pagination stops and adds no items.
    pub async fn load_more_activity(&self) -> Result<WalletUpdate, WalletClientError> {
        let (generation, request) = {
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

            let id = state.allocate_request_id()?;
            let request = build_activity_page_request(&state.config, &cursor, id)?;

            state.pagination_generation = state
                .pagination_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.pagination_generation;
            state.active_pagination = Some((generation, id));
            state.snapshot.activity_pagination_resource = ResourceState::loading();
            state.next_revision()?;

            (generation, request)
        };

        let result = evaluate_response(
            &request,
            self.http_host.execute_http(request.clone()).await,
            |body| parse_activity(body, PAGE_SIZE),
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
        let request_id = {
            let mut state = self.lock()?;
            let request_id = state.active_pagination.take().map(|active| active.1);
            if request_id.is_some() {
                state.snapshot.activity_pagination_resource = ResourceState::idle();
                state.next_revision()?;
            }

            request_id
        };

        if let Some(request_id) = request_id {
            self.http_host.cancel_http(request_id).await;
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

pub(super) fn apply_refreshed_activity_page(state: &mut State, page: ActivityPage) {
    if state.activity.is_empty() {
        state.activity = page.items;
        state.activity_cursor = page.cursor;
        state.activity_has_more = page.has_more;
        state.sync_activity_snapshot();
        return;
    }

    // A refresh replaces the provider head, but it must not discard older pages
    // that the user already loaded. New rows replace matching IDs; the deepest
    // pagination cursor continues to describe the retained tail.
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
    state.sync_activity_snapshot();
}

pub(super) fn build_refresh_requests(
    config: &WalletClientConfig,
    account_id: HttpRequestId,
    activity_id: HttpRequestId,
) -> Result<(HttpRequest, HttpRequest), WalletClientError> {
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
    id: HttpRequestId,
) -> Result<HttpRequest, WalletClientError> {
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use num_bigint::BigUint;
    use ton::ton_core::types::TonAddress;

    use super::*;
    use crate::engine::provider::ActivityRecord;
    use crate::{
        ActivityDirection, Base64Hash, Network, ProviderConfig, SendPhase, SendSnapshot,
        WalletSnapshot,
    };

    const ADDRESS: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn a_nonadvancing_page_stops_pagination_without_mutating_loaded_rows() {
        let mut state = state();
        state.activity = vec![record("existing", 10, 1)];
        state.activity_cursor = Some(cursor(10, 1));
        state.activity_has_more = true;
        state.sync_activity_snapshot();
        let before = state.snapshot.activity.clone();

        let added = apply_activity_page(
            &mut state,
            ActivityPage {
                items: vec![record("unexpected", 11, 2)],
                cursor: Some(cursor(11, 2)),
                has_more: true,
            },
        );

        assert_eq!(added, 0);
        assert_eq!(state.snapshot.activity, before);
        assert_eq!(
            state.activity_cursor.as_ref().map(|value| &value.hash),
            Some(&hash(1))
        );
        assert!(!state.activity_has_more);
        assert!(!state.snapshot.activity_has_more);
    }

    #[test]
    fn the_first_refreshed_page_initializes_rows_cursor_and_has_more_together() {
        let mut state = state();
        let expected_cursor = cursor(20, 3);

        apply_refreshed_activity_page(
            &mut state,
            ActivityPage {
                items: vec![record("first", 20, 3)],
                cursor: Some(expected_cursor.clone()),
                has_more: true,
            },
        );

        assert_eq!(state.activity.len(), 1);
        assert_eq!(state.snapshot.activity[0].id, "first");
        assert_eq!(
            state.activity_cursor.as_ref().map(|value| &value.hash),
            Some(&expected_cursor.hash)
        );
        assert!(state.activity_has_more);
        assert!(state.snapshot.activity_has_more);
    }

    #[test]
    fn cancelling_loading_changes_only_a_loading_resource() {
        let mut loading = ResourceState::loading();
        mark_loading_cancelled(&mut loading);
        assert_eq!(loading, ResourceState::idle());

        let mut ready = ResourceState::ready();
        mark_loading_cancelled(&mut ready);
        assert_eq!(ready, ResourceState::ready());
    }

    fn state() -> State {
        let config = WalletClientConfig {
            record_id: "activity-tests".to_owned(),
            address: ADDRESS.to_owned(),
            network: Network::Testnet,
            send_validity_seconds: 300,
            providers: ProviderConfig::standard(Network::Testnet),
        };
        State {
            snapshot: WalletSnapshot {
                revision: 0,
                record_id: config.record_id.clone(),
                address: config.address.clone(),
                network: config.network,
                account: None,
                account_resource: ResourceState::idle(),
                activity: Vec::new(),
                activity_resource: ResourceState::idle(),
                activity_pagination_resource: ResourceState::idle(),
                activity_cursor: None,
                activity_has_more: false,
                send: SendSnapshot {
                    operation_id: None,
                    phase: SendPhase::Idle,
                    error_message: None,
                },
            },
            config,
            activity: Vec::new(),
            activity_cursor: None,
            activity_has_more: false,
            next_id: 1,
            refresh_generation: 0,
            pagination_generation: 0,
            send_generation: 0,
            active_refresh: None,
            active_pagination: None,
            active_send: None,
            send_commit_started: false,
            send_workflow: None,
            waiters: Vec::new(),
            shutdown: false,
            closing: false,
        }
    }

    fn record(id: &str, logical_time: u64, hash_byte: u8) -> ActivityRecord {
        ActivityRecord {
            id: id.to_owned(),
            transaction_hash: hash(hash_byte),
            logical_time: BigUint::from(logical_time),
            timestamp: logical_time,
            direction: ActivityDirection::Received,
            amount_nanograms: BigUint::from(1_u8),
            counterparty: Some(
                TonAddress::from_str(ADDRESS).expect("activity test address must be valid"),
            ),
        }
    }

    fn cursor(logical_time: u64, hash_byte: u8) -> ActivityPageCursor {
        ActivityPageCursor {
            logical_time: BigUint::from(logical_time),
            hash: hash(hash_byte),
        }
    }

    fn hash(byte: u8) -> Base64Hash {
        Base64Hash::from_bytes(&[byte; 32]).expect("activity test hash must be 256 bits")
    }
}
