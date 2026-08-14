//! Mutable state protected by one wallet-client mutex.

use futures::channel::oneshot;

use crate::provider::{ActivityPageCursor, ActivityRecord};
use crate::send::SendWorkflow;
use crate::{
    HttpCallId, WalletClientConfig, WalletClientError, WalletOperationOutcome, WalletSnapshot,
    WalletUpdate,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OperationFamily {
    Refresh,
    Pagination,
    Send,
}

pub(super) struct State {
    pub(super) config: WalletClientConfig,
    pub(super) snapshot: WalletSnapshot,
    // The public snapshot uses decimal strings for FFI portability. Keep the
    // authoritative activity values numeric so merge and pagination logic do
    // not need to parse those strings again.
    pub(super) activity: Vec<ActivityRecord>,
    pub(super) activity_cursor: Option<ActivityPageCursor>,
    pub(super) activity_has_more: bool,
    pub(super) next_id: u64,
    pub(super) refresh_generation: u64,
    pub(super) pagination_generation: u64,
    pub(super) send_generation: u64,
    pub(super) active_refresh: Option<(u64, Vec<HttpCallId>)>,
    pub(super) active_pagination: Option<(u64, HttpCallId)>,
    pub(super) active_send: Option<(u64, Vec<HttpCallId>)>,
    pub(super) send_commit_started: bool,
    pub(super) send_workflow: Option<SendWorkflow>,
    pub(super) waiters: Vec<(u64, oneshot::Sender<()>)>,
    pub(super) shutdown: bool,
}

impl State {
    pub(super) fn allocate_id(&mut self) -> Result<u64, WalletClientError> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(WalletClientError::IdentifierExhausted)?;
        Ok(id)
    }

    pub(super) fn next_revision(&mut self) -> Result<(), WalletClientError> {
        self.snapshot.revision = self
            .snapshot
            .revision
            .checked_add(1)
            .ok_or(WalletClientError::IdentifierExhausted)?;
        let revision = self.snapshot.revision;
        let mut pending = Vec::new();
        for (after, waiter) in self.waiters.drain(..) {
            if revision > after {
                let _ = waiter.send(());
            } else {
                pending.push((after, waiter));
            }
        }
        self.waiters = pending;
        Ok(())
    }

    /// Publishes the internal numeric activity model through portable DTOs.
    pub(super) fn sync_activity_snapshot(&mut self) {
        self.snapshot.activity = self.activity.iter().map(ActivityRecord::snapshot).collect();
        self.snapshot.activity_cursor = self
            .activity_cursor
            .as_ref()
            .map(ActivityPageCursor::snapshot);
        self.snapshot.activity_has_more = self.activity_has_more;
    }

    pub(super) fn is_current(&self, family: OperationFamily, generation: u64) -> bool {
        match family {
            OperationFamily::Refresh => self
                .active_refresh
                .as_ref()
                .is_some_and(|active| active.0 == generation),
            OperationFamily::Pagination => self
                .active_pagination
                .is_some_and(|active| active.0 == generation),
            OperationFamily::Send => self
                .active_send
                .as_ref()
                .is_some_and(|active| active.0 == generation),
        }
    }
}

pub(super) const fn ensure_running(state: &State) -> Result<(), WalletClientError> {
    if state.shutdown {
        Err(WalletClientError::Shutdown)
    } else {
        Ok(())
    }
}

pub(super) fn update(outcome: WalletOperationOutcome, added: u64, state: &State) -> WalletUpdate {
    WalletUpdate {
        outcome,
        activity_items_added: added,
        snapshot: state.snapshot.clone(),
    }
}
