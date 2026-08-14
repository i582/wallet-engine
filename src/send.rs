//! The private reducer for durable transfer submission.
//!
//! The reducer produces directives for the wallet client. It never performs
//! callbacks itself. Its journal record prevents a second signature after an
//! ambiguous submission result.

#![allow(dead_code)]

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};

use crate::domain::{
    AccountStatus, JournalCompareExchange, JournalCompareExchangeResult, JournalKey, JournalRecord,
    PreparedSend, ProtectedSecretRead, SecretAccessReason, SendPhase, SendRequest, SendSnapshot,
};

const JOURNAL_SCHEMA_VERSION: u32 = 1;
const FIRST_JOURNAL_VERSION: u64 = 1;
pub(crate) const SEND_SLOT: &str = "outgoing-transfer";

/// Fresh chain state used to build a wallet transfer.
///
/// It is deliberately supplied to this reducer by the engine. Fetching and
/// parsing provider responses belongs to the HTTP workflow, not to the send
/// state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FreshSendAccount {
    pub status: AccountStatus,
    pub seqno: u32,
    pub observed_at: u64,
}

impl FreshSendAccount {
    pub(crate) fn needs_state_init(&self) -> bool {
        self.status != AccountStatus::Active
    }
}

/// Signed material produced inside Rust after the host authorizes access to
/// the protected mnemonic. Secret bytes must not be retained in this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedTransfer {
    pub operation_id: String,
    pub record_id: String,
    pub source: String,
    pub destination: String,
    pub amount_nanograms: String,
    pub seqno: u32,
    pub needs_state_init: bool,
    pub valid_until: u32,
    pub signed_boc: Vec<u8>,
    /// The normalized external-message hash in standard padded Base64.
    pub message_hash: String,
}

impl PreparedTransfer {
    pub(crate) fn public_summary(&self) -> PreparedSend {
        PreparedSend {
            operation_id: self.operation_id.clone(),
            valid_until: self.valid_until,
            destination: self.destination.clone(),
            amount_nanograms: self.amount_nanograms.clone(),
        }
    }
}

/// Full internal state. Public `SendPhase` intentionally remains a compact
/// UI projection while the engine coordinates host work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SendStage {
    Validating,
    LoadingJournal,
    FetchingFreshAccount,
    Authorizing,
    Preparing,
    PersistingPrepared,
    ReadyToSubmit,
    Submitting,
    SubmissionUnknown,
    Submitted,
    Failed,
    Cancelled,
}

impl SendStage {
    const fn public_phase(self) -> SendPhase {
        match self {
            Self::Validating | Self::LoadingJournal | Self::FetchingFreshAccount => {
                SendPhase::Validating
            }
            Self::Authorizing => SendPhase::Authorizing,
            Self::Preparing => SendPhase::Preparing,
            Self::PersistingPrepared => SendPhase::Persisting,
            Self::ReadyToSubmit => SendPhase::ReadyToSubmit,
            Self::Submitting => SendPhase::Submitting,
            Self::SubmissionUnknown => SendPhase::SubmissionUnknown,
            Self::Submitted => SendPhase::Submitted,
            Self::Failed => SendPhase::Failed,
            Self::Cancelled => SendPhase::Cancelled,
        }
    }

    const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::SubmissionUnknown | Self::Submitted | Self::Failed | Self::Cancelled
        )
    }
}

/// The next capability the coordinator must perform without holding the
/// wallet-state mutex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SendDirective {
    LoadJournal(JournalKey),
    FetchFreshAccount,
    ReadProtectedSecret(ProtectedSecretRead),
    PrepareTransfer {
        request: SendRequest,
        account: FreshSendAccount,
    },
    PersistJournal(JournalCompareExchange),
    Submit {
        signed_boc: Vec<u8>,
        /// The normalized external-message hash in standard padded Base64.
        message_hash: String,
    },
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SendWorkflowError {
    #[error("invalid send request: {0}")]
    InvalidRequest(String),
    #[error("invalid send transition from {from:?}: {event}")]
    InvalidTransition {
        from: SendStage,
        event: &'static str,
    },
    #[error("prepared transfer does not match the active send request")]
    PreparedTransferMismatch,
    #[error("send journal was changed by another operation")]
    JournalConflict,
    #[error("send journal slot is occupied by an unresolved operation")]
    JournalBusy,
    #[error("wallet account state does not permit sending")]
    AccountUnavailable,
    #[error("send journal record is invalid: {0}")]
    InvalidJournal(String),
}

/// No secret material is ever serialized into this record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DurableSendRecord {
    schema_version: u32,
    operation_id: String,
    record_id: String,
    source: String,
    destination: String,
    amount_nanograms: String,
    seqno: u32,
    needs_state_init: bool,
    valid_until: u32,
    /// The signed BOC encoded as standard padded Base64 for durable storage.
    signed_boc_base64: String,
    /// The normalized external-message hash in standard padded Base64.
    message_hash: String,
    stage: SendStage,
    provider_reference: Option<String>,
    diagnostic: Option<String>,
}

/// Pure send reducer. The owning coordinator invokes callbacks between reducer
/// calls. It invokes no callback while the reducer is borrowed.
#[derive(Debug, Clone)]
pub(crate) struct SendWorkflow {
    record_id: String,
    source: String,
    request: SendRequest,
    stage: SendStage,
    fresh_account: Option<FreshSendAccount>,
    prepared: Option<PreparedTransfer>,
    journal_version: Option<u64>,
    prior_submitted_seqno: Option<u32>,
    provider_reference: Option<String>,
    diagnostic: Option<String>,
}

impl SendWorkflow {
    pub(crate) const fn new(record_id: String, source: String, request: SendRequest) -> Self {
        Self {
            record_id,
            source,
            request,
            stage: SendStage::Validating,
            fresh_account: None,
            prepared: None,
            journal_version: None,
            prior_submitted_seqno: None,
            provider_reference: None,
            diagnostic: None,
        }
    }

    pub(crate) fn operation_id(&self) -> &str {
        &self.request.operation_id
    }

    pub(crate) const fn stage(&self) -> SendStage {
        self.stage
    }

    pub(crate) fn snapshot(&self) -> SendSnapshot {
        SendSnapshot {
            operation_id: Some(self.request.operation_id.clone()),
            phase: self.stage.public_phase(),
            error_message: self.diagnostic.clone(),
        }
    }

    pub(crate) fn begin(&mut self) -> Result<SendDirective, SendWorkflowError> {
        self.expect(SendStage::Validating, "begin")?;
        validate_request(&self.record_id, &self.source, &self.request)?;

        self.stage = SendStage::LoadingJournal;

        Ok(SendDirective::LoadJournal(self.journal_key()))
    }

    /// Seeds the compare-and-swap version of the wallet-level send slot.
    ///
    /// Only an absent slot or a safely terminal prior operation can be
    /// replaced. In particular, `SubmissionUnknown` blocks a new signature:
    /// the persisted BOC can already be on the network.
    pub(crate) fn journal_loaded(
        &mut self,
        record: Option<JournalRecord>,
    ) -> Result<SendDirective, SendWorkflowError> {
        self.expect(SendStage::LoadingJournal, "journal_loaded")?;

        self.journal_version = match record {
            None => None,
            Some(record) => {
                let durable = decode_durable_record(&record)?;
                if durable.record_id != self.record_id || durable.source != self.source {
                    return Err(SendWorkflowError::InvalidJournal(
                        "record belongs to another wallet".to_owned(),
                    ));
                }

                match durable.stage {
                    SendStage::Submitted => {
                        self.prior_submitted_seqno = Some(durable.seqno);
                        Some(record.version)
                    }
                    SendStage::Failed | SendStage::Cancelled => {
                        self.prior_submitted_seqno = None;
                        Some(record.version)
                    }
                    SendStage::SubmissionUnknown => {
                        return Err(SendWorkflowError::JournalBusy);
                    }
                    _ => return Err(SendWorkflowError::JournalBusy),
                }
            }
        };

        self.stage = SendStage::FetchingFreshAccount;

        Ok(SendDirective::FetchFreshAccount)
    }

    pub(crate) fn fresh_account_loaded(
        &mut self,
        account: FreshSendAccount,
    ) -> Result<SendDirective, SendWorkflowError> {
        self.expect(SendStage::FetchingFreshAccount, "fresh_account_loaded")?;

        if let Some(prior_seqno) = self.prior_submitted_seqno
            && (account.status != AccountStatus::Active || account.seqno <= prior_seqno)
        {
            return Err(SendWorkflowError::JournalBusy);
        }

        match account.status {
            AccountStatus::Active => {}
            AccountStatus::Nonexistent | AccountStatus::Uninitialized if account.seqno == 0 => {}
            AccountStatus::Nonexistent
            | AccountStatus::Uninitialized
            | AccountStatus::Frozen
            | AccountStatus::Unknown => {
                return Err(SendWorkflowError::AccountUnavailable);
            }
        }

        self.fresh_account = Some(account);
        self.stage = SendStage::Authorizing;

        Ok(SendDirective::ReadProtectedSecret(ProtectedSecretRead {
            secret_ref: self.request.secret_ref.clone(),
            reason: SecretAccessReason::SignTransfer,
            prompt: "Authenticate to sign this GRAM transfer".to_owned(),
        }))
    }

    /// Marks successful host authorization.
    ///
    /// The reducer does not accept or retain the secret. The coordinator passes
    /// it directly to the Rust signer and zeroizes its temporary buffer.
    pub(crate) fn authorization_succeeded(&mut self) -> Result<SendDirective, SendWorkflowError> {
        self.expect(SendStage::Authorizing, "authorization_succeeded")?;
        let account = self
            .fresh_account
            .clone()
            .ok_or(SendWorkflowError::InvalidTransition {
                from: self.stage,
                event: "authorization_without_fresh_account",
            })?;

        self.stage = SendStage::Preparing;

        Ok(SendDirective::PrepareTransfer {
            request: self.request.clone(),
            account,
        })
    }

    pub(crate) fn transfer_prepared(
        &mut self,
        prepared: PreparedTransfer,
    ) -> Result<SendDirective, SendWorkflowError> {
        self.expect(SendStage::Preparing, "transfer_prepared")?;
        self.validate_prepared(&prepared)?;

        self.prepared = Some(prepared);
        self.stage = SendStage::PersistingPrepared;

        self.persist_directive()
    }

    pub(crate) fn journal_persisted(
        &mut self,
        result: JournalCompareExchangeResult,
    ) -> Result<SendDirective, SendWorkflowError> {
        if !result.applied {
            self.fail("Another send operation changed the journal");
            return Err(SendWorkflowError::JournalConflict);
        }

        self.journal_version = Some(next_journal_version(self.journal_version)?);

        match self.stage {
            SendStage::PersistingPrepared => {
                self.stage = SendStage::ReadyToSubmit;
                let prepared = self.prepared_ref()?;
                Ok(SendDirective::Submit {
                    signed_boc: prepared.signed_boc.clone(),
                    message_hash: prepared.message_hash.clone(),
                })
            }
            SendStage::SubmissionUnknown
            | SendStage::Submitted
            | SendStage::Failed
            | SendStage::Cancelled => Ok(SendDirective::Finished),
            stage => Err(SendWorkflowError::InvalidTransition {
                from: stage,
                event: "journal_persisted",
            }),
        }
    }

    pub(crate) fn submission_started(&mut self) -> Result<(), SendWorkflowError> {
        self.expect(SendStage::ReadyToSubmit, "submission_started")?;

        self.stage = SendStage::Submitting;

        Ok(())
    }

    pub(crate) fn submission_succeeded(
        &mut self,
        provider_reference: Option<String>,
    ) -> Result<SendDirective, SendWorkflowError> {
        self.expect(SendStage::Submitting, "submission_succeeded")?;

        self.provider_reference = provider_reference;
        self.diagnostic = None;
        self.stage = SendStage::Submitted;

        self.persist_directive()
    }

    /// A timeout or connection loss after submission is not a definite
    /// failure. The provider can have accepted the exact persisted BOC.
    /// With reconciliation intentionally absent, this state is a terminal
    /// pending result and never signs a replacement.
    pub(crate) fn submission_unknown(
        &mut self,
        diagnostic: String,
    ) -> Result<SendDirective, SendWorkflowError> {
        self.expect(SendStage::Submitting, "submission_unknown")?;

        self.diagnostic = Some(sanitize_diagnostic(diagnostic));
        self.stage = SendStage::SubmissionUnknown;

        self.persist_directive()
    }

    pub(crate) fn submission_rejected(
        &mut self,
        diagnostic: String,
    ) -> Result<SendDirective, SendWorkflowError> {
        self.expect(SendStage::Submitting, "submission_rejected")?;

        self.fail(diagnostic);

        self.persist_directive()
    }

    pub(crate) fn cancel(&mut self) -> Result<SendDirective, SendWorkflowError> {
        if self.stage.is_terminal() {
            return Ok(SendDirective::Finished);
        }

        self.stage = SendStage::Cancelled;

        if self.prepared.is_some() {
            self.persist_directive()
        } else {
            Ok(SendDirective::Finished)
        }
    }

    fn persist_directive(&self) -> Result<SendDirective, SendWorkflowError> {
        let record = self.journal_record()?;

        Ok(SendDirective::PersistJournal(JournalCompareExchange {
            key: self.journal_key(),
            expected_version: self.journal_version,
            replacement: JournalRecord {
                version: next_journal_version(self.journal_version)?,
                payload: serde_json::to_vec(&record)
                    .map_err(|error| SendWorkflowError::InvalidJournal(error.to_string()))?,
            },
        }))
    }

    fn journal_record(&self) -> Result<DurableSendRecord, SendWorkflowError> {
        let prepared = self.prepared_ref()?;

        Ok(DurableSendRecord {
            schema_version: JOURNAL_SCHEMA_VERSION,
            operation_id: prepared.operation_id.clone(),
            record_id: prepared.record_id.clone(),
            source: prepared.source.clone(),
            destination: prepared.destination.clone(),
            amount_nanograms: prepared.amount_nanograms.clone(),
            seqno: prepared.seqno,
            needs_state_init: prepared.needs_state_init,
            valid_until: prepared.valid_until,
            signed_boc_base64: BASE64.encode(&prepared.signed_boc),
            message_hash: prepared.message_hash.clone(),
            stage: self.stage,
            provider_reference: self.provider_reference.clone(),
            diagnostic: self.diagnostic.clone(),
        })
    }

    fn journal_key(&self) -> JournalKey {
        JournalKey {
            record_id: self.record_id.clone(),
            slot: SEND_SLOT.to_owned(),
        }
    }

    fn prepared_ref(&self) -> Result<&PreparedTransfer, SendWorkflowError> {
        self.prepared
            .as_ref()
            .ok_or(SendWorkflowError::InvalidTransition {
                from: self.stage,
                event: "prepared_transfer_required",
            })
    }

    fn validate_prepared(&self, prepared: &PreparedTransfer) -> Result<(), SendWorkflowError> {
        let account = self
            .fresh_account
            .as_ref()
            .ok_or(SendWorkflowError::PreparedTransferMismatch)?;

        if prepared.operation_id != self.request.operation_id
            || prepared.record_id != self.record_id
            || prepared.source != self.source
            || prepared.destination != self.request.destination
            || prepared.amount_nanograms != self.request.amount_nanograms
            || prepared.seqno != account.seqno
            || prepared.needs_state_init != account.needs_state_init()
            || prepared.signed_boc.is_empty()
            || prepared.message_hash.is_empty()
        {
            return Err(SendWorkflowError::PreparedTransferMismatch);
        }

        Ok(())
    }

    fn expect(&self, expected: SendStage, event: &'static str) -> Result<(), SendWorkflowError> {
        if self.stage == expected {
            Ok(())
        } else {
            Err(SendWorkflowError::InvalidTransition {
                from: self.stage,
                event,
            })
        }
    }

    fn fail(&mut self, diagnostic: impl Into<String>) {
        self.stage = SendStage::Failed;
        self.diagnostic = Some(sanitize_diagnostic(diagnostic.into()));
    }
}

fn validate_request(
    record_id: &str,
    source: &str,
    request: &SendRequest,
) -> Result<(), SendWorkflowError> {
    if record_id.trim().is_empty() || source.trim().is_empty() {
        return Err(SendWorkflowError::InvalidRequest(
            "wallet record identity is empty".to_owned(),
        ));
    }

    if request.operation_id.trim().is_empty() || request.operation_id.len() > 128 {
        return Err(SendWorkflowError::InvalidRequest(
            "operation identifier is invalid".to_owned(),
        ));
    }

    if request.destination.trim().is_empty()
        || request.destination.len() > 128
        || request.destination.chars().any(char::is_whitespace)
    {
        return Err(SendWorkflowError::InvalidRequest(
            "destination is invalid".to_owned(),
        ));
    }

    if request.amount_nanograms.is_empty()
        || request.amount_nanograms == "0"
        || request.amount_nanograms.starts_with('0')
        || !request
            .amount_nanograms
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err(SendWorkflowError::InvalidRequest(
            "amount must be positive canonical nanograms".to_owned(),
        ));
    }

    if request.secret_ref.value.trim().is_empty() {
        return Err(SendWorkflowError::InvalidRequest(
            "protected secret reference is empty".to_owned(),
        ));
    }

    Ok(())
}

fn decode_durable_record(record: &JournalRecord) -> Result<DurableSendRecord, SendWorkflowError> {
    if record.version == 0 {
        return Err(SendWorkflowError::InvalidJournal(
            "version must be positive".to_owned(),
        ));
    }

    let durable: DurableSendRecord = serde_json::from_slice(&record.payload)
        .map_err(|error| SendWorkflowError::InvalidJournal(error.to_string()))?;

    if durable.schema_version != JOURNAL_SCHEMA_VERSION {
        return Err(SendWorkflowError::InvalidJournal(
            "unsupported schema version".to_owned(),
        ));
    }

    let boc_is_invalid = match BASE64.decode(&durable.signed_boc_base64) {
        Ok(boc) => boc.is_empty(),
        Err(_) => true,
    };

    if durable.operation_id.trim().is_empty()
        || durable.record_id.trim().is_empty()
        || durable.source.trim().is_empty()
        || durable.destination.trim().is_empty()
        || durable.message_hash.trim().is_empty()
        || durable.amount_nanograms.is_empty()
        || durable.amount_nanograms.bytes().all(|byte| byte == b'0')
        || !durable
            .amount_nanograms
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        || durable.signed_boc_base64.is_empty()
        || boc_is_invalid
    {
        return Err(SendWorkflowError::InvalidJournal(
            "record fields are invalid".to_owned(),
        ));
    }

    Ok(durable)
}

fn next_journal_version(current: Option<u64>) -> Result<u64, SendWorkflowError> {
    match current {
        Some(version) => version
            .checked_add(1)
            .ok_or_else(|| SendWorkflowError::InvalidJournal("version exhausted".to_owned())),
        None => Ok(FIRST_JOURNAL_VERSION),
    }
}

fn sanitize_diagnostic(value: String) -> String {
    value
        .chars()
        .filter(|character| !character.is_control() || *character == ' ')
        .take(256)
        .collect()
}
