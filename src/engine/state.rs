//! Mutable state protected by one wallet-client mutex.

use futures::channel::oneshot;

use crate::wallet::send::SendWorkflow;
use crate::{
    HttpRequestId, WalletClientConfig, WalletClientError, WalletOperationOutcome, WalletSnapshot,
    WalletUpdate,
};

use super::provider::{ActivityPageCursor, ActivityRecord};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OperationFamily {
    Refresh,
    Pagination,
    Preview,
    Send,
}

/// All mutable state for one wallet client.
///
/// [`WalletClient`](super::WalletClient) protects this value with one mutex.
/// Engine code releases the mutex before it awaits a host callback.
pub(super) struct State {
    /// The validated identity, network, provider URL, and send lifetime for this client.
    /// These values do not change after client construction.
    pub(super) config: WalletClientConfig,

    /// The last public state published to host applications.
    /// Resource failures keep last-good domain data and update the related resource state.
    /// Its revision is the single change sequence for all operation families.
    pub(super) snapshot: WalletSnapshot,

    /// The authoritative activity list with numeric amounts and logical times.
    /// The public snapshot uses decimal strings for FFI portability.
    /// [`Self::sync_activity_snapshot`] publishes this list after each committed merge.
    pub(super) activity: Vec<ActivityRecord>,

    /// The provider cursor for the next older page.
    /// It comes from the oldest raw transaction, which can produce no visible activity item.
    /// A refresh replaces this cursor. A page request advances it only after a successful merge.
    pub(super) activity_cursor: Option<ActivityPageCursor>,

    /// Reports whether the provider can have an older activity page.
    /// A false value makes later pagination calls no-ops.
    pub(super) activity_has_more: bool,

    /// The next HTTP request number for this client instance.
    /// Allocation never reuses a number, even when request construction later fails.
    /// Host cancellation registries must remain client-scoped because replacement clients start from the initial number.
    pub(super) next_id: u64,

    /// The generation of the newest refresh operation.
    /// A newer generation makes late results from an older refresh no-ops.
    pub(super) refresh_generation: u64,

    /// The generation of the newest pagination operation.
    /// This counter separates a cancelled page response from a later page request.
    pub(super) pagination_generation: u64,

    /// The generation of the newest send preview.
    /// It prevents a cancelled preview response from completing a later preview.
    pub(super) preview_generation: u64,

    /// The generation of the newest send operation.
    /// A send result can mutate state only while this generation is active.
    pub(super) send_generation: u64,

    /// The active refresh as `(generation, request_ids)`.
    /// The request list contains the concurrent account and activity requests.
    /// A new refresh replaces this entry and cancels all request identifiers from the old entry.
    pub(super) active_refresh: Option<(u64, Vec<HttpRequestId>)>,

    /// The active older-page load as `(generation, request_id)`.
    /// Only one page request can run. A refresh removes and cancels this entry.
    pub(super) active_pagination: Option<(u64, HttpRequestId)>,

    /// The active send preview as `(generation, active_request_ids)`.
    /// Preview requests never read protected secrets or change the send snapshot.
    pub(super) active_preview: Option<(u64, Vec<HttpRequestId>)>,

    /// The active send as `(generation, active_request_ids)`.
    /// The list contains only HTTP requests that the host currently owns.
    /// Send steps are sequential, so the list usually contains zero or one identifier.
    pub(super) active_send: Option<(u64, Vec<HttpRequestId>)>,

    /// Marks the irreversible send boundary before the prepared journal CAS starts.
    /// Cancellation returns `SendCancellationTooLate` while this value is true.
    /// This rule prevents cancellation from abandoning a BOC that can already reach the provider.
    pub(super) send_commit_started: bool,

    /// The most recently published send reducer, including its terminal state.
    /// It contains prepared message data after authorization, but it never contains the recovery phrase.
    /// The active send and this workflow change under the same mutex.
    pub(super) send_workflow: Option<SendWorkflow>,

    /// Snapshot listeners stored as `(after_revision, sender)`.
    /// A revision wakes listeners whose requested revision is older than the new revision.
    /// If a listener drops its receiver, the next revision discards its failed sender.
    /// Shutdown drops all remaining senders and wakes their receivers with an error.
    pub(super) waiters: Vec<(u64, oneshot::Sender<()>)>,

    /// The permanent terminal flag for this client instance.
    /// New operations fail after shutdown. Late host results cannot publish state.
    /// Graceful shutdown sets this flag after any durable send becomes terminal.
    pub(super) shutdown: bool,

    /// A graceful shutdown has started and rejects all new operations.
    ///
    /// An already durable send can continue while this flag is set. Shutdown
    /// waits for that send to publish a terminal journal state before it sets
    /// [`Self::shutdown`].
    pub(super) closing: bool,
}

impl State {
    pub(super) fn allocate_request_id(&mut self) -> Result<HttpRequestId, WalletClientError> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(WalletClientError::IdentifierExhausted)?;
        Ok(HttpRequestId { value: id })
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
        let network = self.config.network;
        self.snapshot.activity = self
            .activity
            .iter()
            .map(|record| record.snapshot(network))
            .collect();
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
            OperationFamily::Preview => self
                .active_preview
                .as_ref()
                .is_some_and(|active| active.0 == generation),
            OperationFamily::Send => self
                .active_send
                .as_ref()
                .is_some_and(|active| active.0 == generation),
        }
    }
}

pub(super) const fn ensure_running(state: &State) -> Result<(), WalletClientError> {
    if state.closing || state.shutdown {
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
