//! Send-operation tracking and protected-byte lifetime handling.

use crate::domain::bounded_diagnostic;
use crate::wallet::send::{SendWorkflow, SendWorkflowError};
use crate::{DomainError, HttpRequest, HttpRequestId, SendPhase, SendSnapshot, WalletClientError};
use zeroize::Zeroizing;

use super::WalletClient;
use super::http::process_response;
use super::state::OperationFamily;

pub(super) struct SensitiveBytes(Zeroizing<Vec<u8>>);

impl SensitiveBytes {
    pub(super) fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    pub(super) fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl WalletClient {
    /// Publishes the recovered fate of the previous operation while the current
    /// send still owns the single-flight slot.
    pub(super) fn publish_prior_send_resolution(
        &self,
        generation: u64,
        snapshot: SendSnapshot,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.snapshot.send = snapshot;
        state.next_revision()?;
        Ok(())
    }

    /// Ends the current attempt without changing the previous operation to
    /// `Failed`; the unresolved snapshot must remain visible to explain why a
    /// replacement signature was refused.
    pub(super) fn block_send_for_pending(
        &self,
        generation: u64,
        snapshot: SendSnapshot,
    ) -> Result<WalletClientError, WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.active_send = None;
        state.send_commit_started = false;
        state.send_workflow = None;
        state.snapshot.send = snapshot;
        state.next_revision()?;
        Ok(WalletClientError::PreviousSubmissionUnresolved)
    }

    pub(super) fn fail_send(
        &self,
        generation: u64,
        message: String,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.is_current(OperationFamily::Send, generation) {
            state.active_send = None;
            state.send_commit_started = false;
            state.snapshot.send.phase = SendPhase::Failed;
            state.snapshot.send.error_message = Some(bounded_diagnostic(message));
            state.next_revision()?;
        }

        Ok(())
    }

    pub(super) fn send_failed_error(
        &self,
        generation: u64,
        message: impl Into<String>,
    ) -> WalletClientError {
        let diagnostic = bounded_diagnostic(message.into());

        match self.fail_send(generation, diagnostic.clone()) {
            Ok(()) => WalletClientError::SendFailed { diagnostic },
            Err(error) => error,
        }
    }

    pub(super) fn send_workflow_error(
        &self,
        generation: u64,
        error: SendWorkflowError,
    ) -> WalletClientError {
        let diagnostic = bounded_diagnostic(error.to_string());
        let public_error = match error {
            SendWorkflowError::JournalConflict => WalletClientError::SendAlreadyInProgress,
            SendWorkflowError::PreviousSubmissionUnresolved => {
                WalletClientError::PreviousSubmissionUnresolved
            }
            SendWorkflowError::AccountUnavailable { status } => {
                WalletClientError::SendAccountUnavailable { status }
            }
            _ => WalletClientError::SendFailed {
                diagnostic: diagnostic.clone(),
            },
        };

        match self.fail_send(generation, diagnostic) {
            Ok(()) => public_error,
            Err(error) => error,
        }
    }

    fn mark_send_unknown(&self, generation: u64, message: String) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.is_current(OperationFamily::Send, generation) {
            state.active_send = None;
            state.send_commit_started = false;
            state.snapshot.send.phase = SendPhase::SubmissionUnknown;
            state.snapshot.send.error_message = Some(bounded_diagnostic(message));
            state.next_revision()?;
        }

        Ok(())
    }

    pub(super) fn submission_unknown_error(
        &self,
        generation: u64,
        message: impl Into<String>,
    ) -> WalletClientError {
        let diagnostic = bounded_diagnostic(message.into());

        match self.mark_send_unknown(generation, diagnostic.clone()) {
            Ok(()) => WalletClientError::SubmissionUnknown { diagnostic },
            Err(error) => error,
        }
    }

    pub(super) fn begin_send_commit(&self, generation: u64) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientError::Shutdown);
        }

        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientError::StateUnavailable);
        }

        state.send_commit_started = true;

        Ok(())
    }

    pub(super) fn ensure_current_send(&self, generation: u64) -> Result<(), WalletClientError> {
        let state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientError::Shutdown);
        }

        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientError::StateUnavailable);
        }

        Ok(())
    }

    pub(super) fn publish_send_workflow(
        &self,
        generation: u64,
        workflow: &SendWorkflow,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Send, generation) {
            return Err(WalletClientError::StateUnavailable);
        }

        state.snapshot.send = workflow.snapshot();
        state.send_workflow = Some(workflow.clone());
        state.next_revision()?;

        Ok(())
    }

    fn start_send_http_request(
        &self,
        generation: u64,
        request_id: HttpRequestId,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientError::Shutdown);
        }

        let Some((active_generation, request_ids)) = state.active_send.as_mut() else {
            return Err(WalletClientError::StateUnavailable);
        };

        if *active_generation != generation {
            return Err(WalletClientError::StateUnavailable);
        }

        request_ids.push(request_id);

        Ok(())
    }

    /// Executes a send-owned HTTP request and returns its engine-checked body.
    ///
    /// Tracking is finished before response processing so a rejected response
    /// cannot leave its request ID registered as active.
    pub(super) async fn execute_tracked_send_request(
        &self,
        generation: u64,
        request: &HttpRequest,
    ) -> Result<Result<Vec<u8>, DomainError>, WalletClientError> {
        self.start_send_http_request(generation, request.id)?;

        // Do not hold the state lock while the foreign host performs I/O.
        let result = self.http_host.execute_http(request.clone()).await;

        self.finish_send_http_request(generation, request.id)?;

        Ok(process_response(request, result))
    }

    fn finish_send_http_request(
        &self,
        generation: u64,
        request_id: HttpRequestId,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.shutdown {
            return Err(WalletClientError::Shutdown);
        }

        let Some((active_generation, request_ids)) = state.active_send.as_mut() else {
            return Err(WalletClientError::StateUnavailable);
        };

        if *active_generation != generation {
            return Err(WalletClientError::StateUnavailable);
        }

        request_ids.retain(|active| *active != request_id);

        Ok(())
    }
}
