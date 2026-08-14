//! Wallet client construction, snapshots, waiting, and shutdown.

use std::sync::{Arc, Mutex};

use futures::channel::oneshot;

use crate::{WalletClientConfig, WalletClientError, WalletSnapshot};

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
                send_generation: 0,
                active_refresh: None,
                active_pagination: None,
                active_send: None,
                send_commit_started: false,
                send_workflow: None,
                waiters: Vec::new(),
                shutdown: false,
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
            if state.shutdown {
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
    /// The operation is idempotent. It returns `SendCancellationTooLate` while
    /// a send is past its durable commit boundary. Call it again after that
    /// send reaches a terminal phase.
    pub async fn shutdown(&self) -> Result<(), WalletClientError> {
        let (request_ids, waiters) = {
            let mut state = self.lock()?;
            if state.shutdown {
                return Ok(());
            }

            if state.active_send.is_some() && state.send_commit_started {
                return Err(WalletClientError::SendCancellationTooLate);
            }

            state.shutdown = true;

            let mut request_ids = state
                .active_refresh
                .take()
                .map(|active| active.1)
                .unwrap_or_default();
            if let Some((_, request_id)) = state.active_pagination.take() {
                request_ids.push(request_id);
            }

            if let Some((_, send_request_ids)) = state.active_send.take() {
                request_ids.extend(send_request_ids);
            }

            state.send_commit_started = false;

            (request_ids, std::mem::take(&mut state.waiters))
        };

        for request_id in request_ids {
            self.http_host.cancel_http(request_id).await;
        }

        drop(waiters);

        Ok(())
    }
}

impl WalletClient {
    pub(super) fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>, WalletClientError> {
        self.state
            .lock()
            .map_err(|_| WalletClientError::StateUnavailable)
    }
}
