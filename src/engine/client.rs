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
                next_id: 1,
                refresh_generation: 0,
                pagination_generation: 0,
                preview_generation: 0,
                send_generation: 0,
                active_refresh: None,
                active_pagination: None,
                active_preview: None,
                active_send: None,
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
        || state.active_preview.is_some()
        || state.active_send.is_some();
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
        mark_loading_cancelled(&mut state.snapshot.activity_resource);
    }

    if let Some((_, request_id)) = state.active_pagination.take() {
        request_ids.push(request_id);
        state.snapshot.activity_pagination_resource = ResourceState::idle();
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
