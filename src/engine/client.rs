//! Wallet client construction, snapshots, waiting, and shutdown.

use std::sync::{Arc, Mutex};

use futures::channel::oneshot;

use crate::{ResourceState, SendPhase, WalletClientConfig, WalletClientError, WalletSnapshot};

use super::activity::mark_loading_cancelled;
use super::host::{WalletHttpHost, WalletPlatformHost};
use super::state::State;
use super::validation::validate_config;

#[derive(uniffi::Object)]
/// Coordinates state and operations for one wallet record.
///
/// The client owns no transport or platform resources. Call [`Self::shutdown`]
/// before the host releases callback objects or application services.
pub struct WalletClient {
    pub(super) http_host: Arc<dyn WalletHttpHost>,
    pub(super) platform_host: Arc<dyn WalletPlatformHost>,
    state: Mutex<State>,
}

#[uniffi::export]
impl WalletClient {
    #[uniffi::constructor]
    /// Creates a client after validation of identifiers, URLs, and credential origin.
    ///
    /// The initial snapshot has revision zero and idle resource states.
    pub fn new(
        config: WalletClientConfig,
        http_host: Arc<dyn WalletHttpHost>,
        platform_host: Arc<dyn WalletPlatformHost>,
    ) -> Result<Arc<Self>, WalletClientError> {
        validate_config(&config)?;

        let snapshot = WalletSnapshot::empty(&config);

        Ok(Arc::new(Self {
            http_host,
            platform_host,
            state: Mutex::new(State {
                config,
                snapshot,
                activity: Vec::new(),
                activity_cursor: None,
                activity_has_more: false,
                nft_offset: 0,
                nfts_has_more: false,
                next_id: 1,
                refresh_generation: 0,
                pagination_generation: 0,
                nft_refresh_generation: 0,
                nft_pagination_generation: 0,
                preview_generation: 0,
                send_generation: 0,
                resolution_generation: 0,
                active_refresh: None,
                active_pagination: None,
                active_nft_refresh: None,
                active_nft_pagination: None,
                active_preview: None,
                active_send: None,
                active_resolution: None,
                send_commit_started: false,
                send_workflow: None,
                waiters: Vec::new(),
                shutdown: false,
                closing: false,
            }),
        }))
    }

    /// Returns a clone of the current immutable snapshot.
    ///
    /// A returned snapshot never changes. Read a newer snapshot to observe a
    /// higher revision.
    pub fn snapshot(&self) -> Result<WalletSnapshot, WalletClientError> {
        Ok(self.lock()?.snapshot.clone())
    }

    /// Waits until the snapshot revision is greater than `after_revision`.
    ///
    /// This method returns immediately when a newer revision already exists.
    /// Shutdown releases all waiters and returns [`WalletClientError::Shutdown`].
    pub async fn wait_for_change(
        &self,
        after_revision: u64,
    ) -> Result<WalletSnapshot, WalletClientError> {
        // Hosts commonly start their observation loop immediately after
        // construction. Use that first async poll as the runtime-neutral
        // startup sweep for a durable send left by a previous process.
        let _ = self.resolve_pending().await;

        let receiver = {
            let mut state = self.lock()?;
            if state.closing || state.shutdown {
                return Err(WalletClientError::Shutdown);
            }

            if state.snapshot.revision > after_revision {
                return Ok(state.snapshot.clone());
            }

            let (sender, receiver) = oneshot::channel();
            state.waiters.push((after_revision, sender));

            receiver
        };

        receiver.await.map_err(|_| WalletClientError::Shutdown)?;

        let state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientError::Shutdown);
        }

        Ok(state.snapshot.clone())
    }

    /// Stops new work, cancels active host requests, and releases snapshot waiters.
    ///
    /// The operation is idempotent. It cancels reversible work immediately.
    /// If a send crossed its durable commit boundary, shutdown rejects new
    /// operations and waits for that send to reach a terminal journal state.
    pub async fn shutdown(&self) -> Result<(), WalletClientError> {
        loop {
            let shutdown = {
                let mut state = self.lock()?;
                if state.shutdown {
                    return Ok(());
                }

                state.closing = true;
                if state.active_send.is_some() && state.send_commit_started {
                    let revision = state.snapshot.revision;
                    let (sender, receiver) = oneshot::channel();
                    state.waiters.push((revision, sender));
                    ShutdownAction::Wait(receiver)
                } else {
                    ShutdownAction::Finish(prepare_shutdown(&mut state)?)
                }
            };

            match shutdown {
                ShutdownAction::Wait(receiver) => {
                    let _ = receiver.await;
                }
                ShutdownAction::Finish((request_ids, waiters)) => {
                    for request_id in request_ids {
                        self.http_host.cancel_http(request_id).await;
                    }

                    drop(waiters);
                    return Ok(());
                }
            }
        }
    }
}

type SnapshotWaiter = (u64, oneshot::Sender<()>);

enum ShutdownAction {
    Wait(oneshot::Receiver<()>),
    Finish((Vec<crate::HttpRequestId>, Vec<SnapshotWaiter>)),
}

fn prepare_shutdown(
    state: &mut State,
) -> Result<(Vec<crate::HttpRequestId>, Vec<SnapshotWaiter>), WalletClientError> {
    let has_active_work = state.active_refresh.is_some()
        || state.active_pagination.is_some()
        || state.active_nft_refresh.is_some()
        || state.active_nft_pagination.is_some()
        || state.active_preview.is_some()
        || state.active_send.is_some();
    let has_active_work = has_active_work || state.active_resolution.is_some();
    if has_active_work && state.snapshot.revision == u64::MAX {
        return Err(WalletClientError::IdentifierExhausted);
    }

    let active_refresh = state.active_refresh.take();
    let mut request_ids = active_refresh
        .as_ref()
        .map(|active| active.1.clone())
        .unwrap_or_default();
    if active_refresh.is_some() {
        mark_loading_cancelled(&mut state.snapshot.account_resource);
        mark_loading_cancelled(&mut state.snapshot.activity.resource);
    }

    if let Some((_, request_id)) = state.active_pagination.take() {
        request_ids.push(request_id);
        state.snapshot.activity.pagination_resource = ResourceState::idle();
    }

    if let Some((_, request_id)) = state.active_nft_refresh.take() {
        request_ids.push(request_id);
        mark_loading_cancelled(&mut state.snapshot.nfts.resource);
    }

    if let Some((_, request_id)) = state.active_nft_pagination.take() {
        request_ids.push(request_id);
        state.snapshot.nfts.pagination_resource = ResourceState::idle();
    }

    if let Some((_, preview_request_ids)) = state.active_preview.take() {
        request_ids.extend(preview_request_ids);
    }

    if let Some((_, send_request_ids)) = state.active_send.take() {
        request_ids.extend(send_request_ids);
        if let Some(mut workflow) = state.send_workflow.take() {
            let _ = workflow.cancel();
            state.snapshot.send = workflow.snapshot();
            state.send_workflow = Some(workflow);
        }
        state.snapshot.send.phase = SendPhase::Cancelled;
    }

    if let Some((_, resolution_request_ids)) = state.active_resolution.take() {
        request_ids.extend(resolution_request_ids);
    }

    state.send_commit_started = false;
    if has_active_work {
        state.next_revision()?;
    }
    state.shutdown = true;

    Ok((request_ids, std::mem::take(&mut state.waiters)))
}

impl WalletClient {
    pub(super) fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>, WalletClientError> {
        self.state
            .lock()
            .map_err(|_| WalletClientError::StateUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::state::ensure_running;
    use crate::{HttpRequestId, Network, NonEmptyString, ProviderConfig, TonAddressString};

    const ADDRESS: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn shutdown_detects_revision_exhaustion_for_each_active_operation_family() {
        for activate in [
            activate_refresh as fn(&mut State),
            activate_nft_refresh,
            activate_nft_pagination,
            activate_preview,
            activate_send,
            activate_resolution,
        ] {
            let mut state = state();
            state.snapshot.revision = u64::MAX;
            activate(&mut state);

            assert!(matches!(
                prepare_shutdown(&mut state),
                Err(WalletClientError::IdentifierExhausted)
            ));
        }
    }

    #[test]
    fn closing_and_shutdown_each_reject_new_work_independently() {
        let mut closing = state();
        closing.closing = true;
        assert_eq!(ensure_running(&closing), Err(WalletClientError::Shutdown));

        let mut shutdown = state();
        shutdown.shutdown = true;
        assert_eq!(ensure_running(&shutdown), Err(WalletClientError::Shutdown));
    }

    fn activate_refresh(state: &mut State) {
        state.active_refresh = Some((1, vec![request_id()]));
    }

    fn activate_preview(state: &mut State) {
        state.active_preview = Some((1, vec![request_id()]));
    }

    fn activate_nft_refresh(state: &mut State) {
        state.active_nft_refresh = Some((1, request_id()));
    }

    fn activate_nft_pagination(state: &mut State) {
        state.active_nft_pagination = Some((1, request_id()));
    }

    fn activate_send(state: &mut State) {
        state.active_send = Some((1, vec![request_id()]));
    }

    fn activate_resolution(state: &mut State) {
        state.active_resolution = Some((1, vec![request_id()]));
    }

    const fn request_id() -> HttpRequestId {
        HttpRequestId { value: 7 }
    }

    fn state() -> State {
        let config = WalletClientConfig {
            record_id: NonEmptyString::try_from("client-tests").expect("valid record identifier"),
            address: TonAddressString::try_from(ADDRESS).expect("valid TON address"),
            public_key: vec![0; 32],
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig::standard(Network::Testnet),
        };
        State {
            snapshot: WalletSnapshot::empty(&config),
            config,
            activity: Vec::new(),
            activity_cursor: None,
            activity_has_more: false,
            nft_offset: 0,
            nfts_has_more: false,
            next_id: 1,
            refresh_generation: 0,
            pagination_generation: 0,
            nft_refresh_generation: 0,
            nft_pagination_generation: 0,
            preview_generation: 0,
            send_generation: 0,
            resolution_generation: 0,
            active_refresh: None,
            active_pagination: None,
            active_nft_refresh: None,
            active_nft_pagination: None,
            active_preview: None,
            active_send: None,
            active_resolution: None,
            send_commit_started: false,
            send_workflow: None,
            waiters: Vec::new(),
            shutdown: false,
            closing: false,
        }
    }
}
