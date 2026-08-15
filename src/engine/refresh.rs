//! Account, first-page activity, and jetton refresh operation.

use futures::future::join3;

use crate::{
    AccountSnapshot, DomainError, HttpRequest, HttpRequestId, ResourcePhase, ResourceState,
    WalletClientConfig, WalletClientError, WalletOperationOutcome, WalletUpdate,
};

use super::WalletClient;
use super::activity::{PAGE_SIZE, apply_refreshed_activity_page, mark_loading_cancelled};
use super::http::{build_toncenter_v2_request, build_toncenter_v3_request, evaluate_response};
use super::provider::{ActivityPage, JettonPage, parse_account, parse_activity, parse_jettons};
use super::state::{OperationFamily, ensure_running, update};

const JETTON_PAGE_SIZE: u32 = 1_000;
const JETTON_PAGE_SIZE_QUERY: &str = "1000";

struct RefreshRequests {
    account: HttpRequest,
    activity: HttpRequest,
    jettons: HttpRequest,
}

#[uniffi::export]
impl WalletClient {
    /// Refreshes account, first-page activity, and jetton data concurrently.
    ///
    /// Each resource has independent state. One request can succeed while another
    /// fails, which produces [`WalletOperationOutcome::PartiallyCompleted`].
    /// A newer refresh supersedes the older refresh and cancels its host requests.
    pub async fn refresh(&self) -> Result<WalletUpdate, WalletClientError> {
        let (generation, requests, owner, network, previous_request_ids) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;

            let config = state.config.clone();
            let owner = config.parsed_address()?;
            let account_id = state.allocate_request_id()?;
            let activity_id = state.allocate_request_id()?;
            let jettons_id = state.allocate_request_id()?;

            let requests = build_refresh_requests(&config, account_id, activity_id, jettons_id)?;

            state.refresh_generation = state
                .refresh_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.refresh_generation;

            let mut previous_request_ids = state
                .active_refresh
                .replace((generation, vec![account_id, activity_id, jettons_id]))
                .map(|active| active.1)
                .unwrap_or_default();
            if let Some((_, page_request_id)) = state.active_pagination.take() {
                previous_request_ids.push(page_request_id);
            }

            state.snapshot.account_resource = ResourceState::loading();
            state.snapshot.activity_resource = ResourceState::loading();
            state.snapshot.jettons_resource = ResourceState::loading();
            state.snapshot.activity_pagination_resource = ResourceState::idle();
            state.next_revision()?;

            (
                generation,
                requests,
                owner,
                config.network,
                previous_request_ids,
            )
        };

        for request_id in previous_request_ids {
            self.http_host.cancel_http(request_id).await;
        }

        let (account, activity, jettons) = join3(
            self.http_host.execute_http(requests.account.clone()),
            self.http_host.execute_http(requests.activity.clone()),
            self.http_host.execute_http(requests.jettons.clone()),
        )
        .await;

        let account = evaluate_response(&requests.account, account, parse_account);
        self.publish_refresh_component(generation, RefreshValue::Account(account))?;

        let activity = evaluate_response(&requests.activity, activity, |body| {
            parse_activity(body, PAGE_SIZE)
        });
        self.publish_refresh_component(generation, RefreshValue::Activity(activity))?;

        let jettons = evaluate_response(&requests.jettons, jettons, |body| {
            parse_jettons(body, &owner, network, JETTON_PAGE_SIZE)
        });
        self.publish_refresh_component(generation, RefreshValue::Jettons(jettons))?;

        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Refresh, generation) {
            return Ok(update(WalletOperationOutcome::Superseded, 0, &state));
        }

        state.active_refresh = None;

        let failed = [
            &state.snapshot.account_resource,
            &state.snapshot.activity_resource,
            &state.snapshot.jettons_resource,
        ]
        .into_iter()
        .filter(|resource| resource.phase == ResourcePhase::Failed)
        .count();

        let outcome = match failed {
            0 => WalletOperationOutcome::Completed,
            3 => WalletOperationOutcome::Failed,
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
                mark_loading_cancelled(&mut state.snapshot.jettons_resource);
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
                    apply_refreshed_activity_page(&mut state, page);
                    state.snapshot.activity_resource = ResourceState::ready();
                }
                Err(error) => state.snapshot.activity_resource = ResourceState::failed(error),
            },
            RefreshValue::Jettons(result) => match result {
                Ok(page) => {
                    state.snapshot.jettons = page.items;
                    state.snapshot.jettons_has_more = page.has_more;
                    state.snapshot.jettons_resource = ResourceState::ready();
                }
                Err(error) => state.snapshot.jettons_resource = ResourceState::failed(error),
            },
        }

        state.next_revision()?;

        Ok(())
    }
}

enum RefreshValue {
    Account(Result<AccountSnapshot, DomainError>),
    Activity(Result<ActivityPage, DomainError>),
    Jettons(Result<JettonPage, DomainError>),
}

fn build_refresh_requests(
    config: &WalletClientConfig,
    account_id: HttpRequestId,
    activity_id: HttpRequestId,
    jettons_id: HttpRequestId,
) -> Result<RefreshRequests, WalletClientError> {
    Ok(RefreshRequests {
        account: build_toncenter_v2_request(
            config,
            account_id,
            "getAddressInformation",
            &[("address", config.address.as_str())],
        )?,
        activity: build_toncenter_v2_request(
            config,
            activity_id,
            "getTransactions",
            &[("address", config.address.as_str()), ("limit", "10")],
        )?,
        jettons: build_toncenter_v3_request(
            config,
            jettons_id,
            &["jetton", "wallets"],
            &[
                ("owner_address", config.address.as_str()),
                ("exclude_zero_balance", "true"),
                ("limit", JETTON_PAGE_SIZE_QUERY),
                ("sort", "desc"),
            ],
        )?,
    })
}

#[cfg(test)]
mod tests {
    use super::build_refresh_requests;
    use crate::{HttpRequestId, Network, ProviderConfig, WalletClientConfig};

    const ADDRESS: &str = "0:0000000000000000000000000000000000000000000000000000000000000001";

    #[test]
    fn jetton_refresh_uses_the_v3_owner_query_on_the_provider_base() {
        let config = WalletClientConfig {
            record_id: "record".to_owned(),
            address: ADDRESS.to_owned(),
            public_key: vec![0; 32],
            network: Network::Testnet,
            send_validity_seconds: 300,
            providers: ProviderConfig {
                toncenter_base_url: "https://provider.example/custom".to_owned(),
            },
        };

        let requests = build_refresh_requests(
            &config,
            HttpRequestId { value: 1 },
            HttpRequestId { value: 2 },
            HttpRequestId { value: 3 },
        )
        .expect("refresh requests must build");

        assert_eq!(
            requests.jettons.url,
            format!(
                "https://provider.example/custom/api/v3/jetton/wallets?owner_address={ADDRESS_ENCODED}&exclude_zero_balance=true&limit=1000&sort=desc",
                ADDRESS_ENCODED = ADDRESS.replace(':', "%3A")
            )
        );
    }
}
