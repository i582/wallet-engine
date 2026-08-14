#![allow(dead_code)]

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};

use crate::domain::{
    AccountStatusV3, JournalCompareExchangeResultV3, JournalCompareExchangeV3, JournalKeyV3,
    JournalRecordV3, PreparedSendV3, ProtectedSecretReadV3, SecretAccessReasonV3, SendPhaseV3,
    SendRequestV3, SendSnapshotV3,
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
pub(crate) struct FreshSendAccountV3 {
    pub status: AccountStatusV3,
    pub seqno: u32,
    pub observed_at: u64,
}

impl FreshSendAccountV3 {
    pub(crate) fn needs_state_init(&self) -> bool {
        self.status != AccountStatusV3::Active
    }
}

/// Signed material produced inside Rust after the host authorizes access to
/// the protected mnemonic. Secret bytes must not be retained in this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedTransferV3 {
    pub operation_id: String,
    pub wallet_id: String,
    pub source: String,
    pub destination: String,
    pub amount_nanograms: String,
    pub seqno: u32,
    pub needs_state_init: bool,
    pub valid_until: u64,
    pub signed_boc: Vec<u8>,
    pub message_hash: String,
}

impl PreparedTransferV3 {
    pub(crate) fn public_summary(&self) -> PreparedSendV3 {
        PreparedSendV3 {
            operation_id: self.operation_id.clone(),
            valid_until: self.valid_until,
            destination: self.destination.clone(),
            amount_nanograms: self.amount_nanograms.clone(),
        }
    }
}

/// Full internal state. Public `SendPhaseV3` intentionally remains a compact
/// UI projection while V3 is being integrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SendStageV3 {
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

impl SendStageV3 {
    const fn public_phase(self) -> SendPhaseV3 {
        match self {
            Self::Validating | Self::LoadingJournal | Self::FetchingFreshAccount => {
                SendPhaseV3::Validating
            }
            Self::Authorizing => SendPhaseV3::Authorizing,
            Self::Preparing => SendPhaseV3::Preparing,
            Self::PersistingPrepared => SendPhaseV3::Persisting,
            Self::ReadyToSubmit => SendPhaseV3::ReadyToSubmit,
            Self::Submitting => SendPhaseV3::Submitting,
            Self::SubmissionUnknown => SendPhaseV3::SubmissionUnknown,
            Self::Submitted => SendPhaseV3::Submitted,
            Self::Failed => SendPhaseV3::Failed,
            Self::Cancelled => SendPhaseV3::Cancelled,
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
pub(crate) enum SendDirectiveV3 {
    LoadJournal(JournalKeyV3),
    FetchFreshAccount,
    ReadProtectedSecret(ProtectedSecretReadV3),
    PrepareTransfer {
        request: SendRequestV3,
        account: FreshSendAccountV3,
    },
    PersistJournal(JournalCompareExchangeV3),
    Submit {
        signed_boc: Vec<u8>,
        message_hash: String,
    },
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SendWorkflowErrorV3 {
    #[error("invalid send request: {0}")]
    InvalidRequest(String),
    #[error("invalid send transition from {from:?}: {event}")]
    InvalidTransition {
        from: SendStageV3,
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
struct DurableSendRecordV3 {
    schema_version: u32,
    operation_id: String,
    wallet_id: String,
    source: String,
    destination: String,
    amount_nanograms: String,
    seqno: u32,
    needs_state_init: bool,
    valid_until: u64,
    signed_boc_base64: String,
    message_hash: String,
    stage: SendStageV3,
    provider_reference: Option<String>,
    diagnostic: Option<String>,
}

/// Pure send reducer. All callbacks are invoked by the owning coordinator
/// between calls to this type; none are invoked while the reducer is borrowed.
#[derive(Debug, Clone)]
pub(crate) struct SendWorkflowV3 {
    wallet_id: String,
    source: String,
    request: SendRequestV3,
    stage: SendStageV3,
    fresh_account: Option<FreshSendAccountV3>,
    prepared: Option<PreparedTransferV3>,
    journal_version: Option<u64>,
    prior_submitted_seqno: Option<u32>,
    provider_reference: Option<String>,
    diagnostic: Option<String>,
}

impl SendWorkflowV3 {
    pub(crate) const fn new(wallet_id: String, source: String, request: SendRequestV3) -> Self {
        Self {
            wallet_id,
            source,
            request,
            stage: SendStageV3::Validating,
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

    pub(crate) const fn stage(&self) -> SendStageV3 {
        self.stage
    }

    pub(crate) fn snapshot(&self) -> SendSnapshotV3 {
        SendSnapshotV3 {
            operation_id: Some(self.request.operation_id.clone()),
            phase: self.stage.public_phase(),
            error_message: self.diagnostic.clone(),
        }
    }

    pub(crate) fn begin(&mut self) -> Result<SendDirectiveV3, SendWorkflowErrorV3> {
        self.expect(SendStageV3::Validating, "begin")?;
        validate_request(&self.wallet_id, &self.source, &self.request)?;
        self.stage = SendStageV3::LoadingJournal;
        Ok(SendDirectiveV3::LoadJournal(self.journal_key()))
    }

    /// Seeds the compare-and-swap version of the wallet-level send slot.
    ///
    /// Only an absent slot or a safely terminal prior operation may be
    /// replaced. In particular, `SubmissionUnknown` blocks a new signature:
    /// the persisted BOC may already have reached the network.
    pub(crate) fn journal_loaded(
        &mut self,
        record: Option<JournalRecordV3>,
    ) -> Result<SendDirectiveV3, SendWorkflowErrorV3> {
        self.expect(SendStageV3::LoadingJournal, "journal_loaded")?;

        self.journal_version = match record {
            None => None,
            Some(record) => {
                let durable = decode_durable_record(&record)?;
                if durable.wallet_id != self.wallet_id || durable.source != self.source {
                    return Err(SendWorkflowErrorV3::InvalidJournal(
                        "record belongs to another wallet".to_owned(),
                    ));
                }
                match durable.stage {
                    SendStageV3::Submitted => {
                        self.prior_submitted_seqno = Some(durable.seqno);
                        Some(record.version)
                    }
                    SendStageV3::Failed | SendStageV3::Cancelled => {
                        self.prior_submitted_seqno = None;
                        Some(record.version)
                    }
                    SendStageV3::SubmissionUnknown => {
                        return Err(SendWorkflowErrorV3::JournalBusy);
                    }
                    _ => return Err(SendWorkflowErrorV3::JournalBusy),
                }
            }
        };

        self.stage = SendStageV3::FetchingFreshAccount;
        Ok(SendDirectiveV3::FetchFreshAccount)
    }

    pub(crate) fn fresh_account_loaded(
        &mut self,
        account: FreshSendAccountV3,
    ) -> Result<SendDirectiveV3, SendWorkflowErrorV3> {
        self.expect(SendStageV3::FetchingFreshAccount, "fresh_account_loaded")?;
        if let Some(prior_seqno) = self.prior_submitted_seqno
            && (account.status != AccountStatusV3::Active || account.seqno <= prior_seqno)
        {
            return Err(SendWorkflowErrorV3::JournalBusy);
        }
        match account.status {
            AccountStatusV3::Active => {}
            AccountStatusV3::Nonexistent | AccountStatusV3::Uninitialized if account.seqno == 0 => {
            }
            AccountStatusV3::Nonexistent
            | AccountStatusV3::Uninitialized
            | AccountStatusV3::Frozen
            | AccountStatusV3::Unknown => {
                return Err(SendWorkflowErrorV3::AccountUnavailable);
            }
        }
        self.fresh_account = Some(account);
        self.stage = SendStageV3::Authorizing;
        Ok(SendDirectiveV3::ReadProtectedSecret(
            ProtectedSecretReadV3 {
                secret_ref: self.request.secret_ref.clone(),
                reason: SecretAccessReasonV3::SignTransfer,
                prompt: "Authenticate to sign this GRAM transfer".to_owned(),
            },
        ))
    }

    /// Marks successful host authorization. The secret itself is intentionally
    /// not accepted or retained by the reducer; the coordinator passes it
    /// directly to the Rust signer and zeroizes its temporary buffer.
    pub(crate) fn authorization_succeeded(
        &mut self,
    ) -> Result<SendDirectiveV3, SendWorkflowErrorV3> {
        self.expect(SendStageV3::Authorizing, "authorization_succeeded")?;
        let account = self
            .fresh_account
            .clone()
            .ok_or(SendWorkflowErrorV3::InvalidTransition {
                from: self.stage,
                event: "authorization_without_fresh_account",
            })?;
        self.stage = SendStageV3::Preparing;
        Ok(SendDirectiveV3::PrepareTransfer {
            request: self.request.clone(),
            account,
        })
    }

    pub(crate) fn transfer_prepared(
        &mut self,
        prepared: PreparedTransferV3,
    ) -> Result<SendDirectiveV3, SendWorkflowErrorV3> {
        self.expect(SendStageV3::Preparing, "transfer_prepared")?;
        self.validate_prepared(&prepared)?;
        self.prepared = Some(prepared);
        self.stage = SendStageV3::PersistingPrepared;
        self.persist_directive()
    }

    pub(crate) fn journal_persisted(
        &mut self,
        result: JournalCompareExchangeResultV3,
    ) -> Result<SendDirectiveV3, SendWorkflowErrorV3> {
        if !result.applied {
            self.fail("Another send operation changed the journal");
            return Err(SendWorkflowErrorV3::JournalConflict);
        }

        self.journal_version = Some(next_journal_version(self.journal_version)?);
        match self.stage {
            SendStageV3::PersistingPrepared => {
                self.stage = SendStageV3::ReadyToSubmit;
                let prepared = self.prepared_ref()?;
                Ok(SendDirectiveV3::Submit {
                    signed_boc: prepared.signed_boc.clone(),
                    message_hash: prepared.message_hash.clone(),
                })
            }
            SendStageV3::SubmissionUnknown
            | SendStageV3::Submitted
            | SendStageV3::Failed
            | SendStageV3::Cancelled => Ok(SendDirectiveV3::Finished),
            stage => Err(SendWorkflowErrorV3::InvalidTransition {
                from: stage,
                event: "journal_persisted",
            }),
        }
    }

    pub(crate) fn submission_started(&mut self) -> Result<(), SendWorkflowErrorV3> {
        self.expect(SendStageV3::ReadyToSubmit, "submission_started")?;
        self.stage = SendStageV3::Submitting;
        Ok(())
    }

    pub(crate) fn submission_succeeded(
        &mut self,
        provider_reference: Option<String>,
    ) -> Result<SendDirectiveV3, SendWorkflowErrorV3> {
        self.expect(SendStageV3::Submitting, "submission_succeeded")?;
        self.provider_reference = provider_reference;
        self.diagnostic = None;
        self.stage = SendStageV3::Submitted;
        self.persist_directive()
    }

    /// A timeout or connection loss after submission is not a definite
    /// failure: the provider may have accepted the exact persisted BOC.
    /// With reconciliation intentionally absent, this state is a terminal
    /// pending result and never signs a replacement.
    pub(crate) fn submission_unknown(
        &mut self,
        diagnostic: String,
    ) -> Result<SendDirectiveV3, SendWorkflowErrorV3> {
        self.expect(SendStageV3::Submitting, "submission_unknown")?;
        self.diagnostic = Some(sanitize_diagnostic(diagnostic));
        self.stage = SendStageV3::SubmissionUnknown;
        self.persist_directive()
    }

    pub(crate) fn submission_rejected(
        &mut self,
        diagnostic: String,
    ) -> Result<SendDirectiveV3, SendWorkflowErrorV3> {
        self.expect(SendStageV3::Submitting, "submission_rejected")?;
        self.fail(diagnostic);
        self.persist_directive()
    }

    pub(crate) fn cancel(&mut self) -> Result<SendDirectiveV3, SendWorkflowErrorV3> {
        if self.stage.is_terminal() {
            return Ok(SendDirectiveV3::Finished);
        }
        self.stage = SendStageV3::Cancelled;
        if self.prepared.is_some() {
            self.persist_directive()
        } else {
            Ok(SendDirectiveV3::Finished)
        }
    }

    fn persist_directive(&self) -> Result<SendDirectiveV3, SendWorkflowErrorV3> {
        let record = self.journal_record()?;
        Ok(SendDirectiveV3::PersistJournal(JournalCompareExchangeV3 {
            key: self.journal_key(),
            expected_version: self.journal_version,
            replacement: JournalRecordV3 {
                version: next_journal_version(self.journal_version)?,
                payload: serde_json::to_vec(&record)
                    .map_err(|error| SendWorkflowErrorV3::InvalidJournal(error.to_string()))?,
            },
        }))
    }

    fn journal_record(&self) -> Result<DurableSendRecordV3, SendWorkflowErrorV3> {
        let prepared = self.prepared_ref()?;
        Ok(DurableSendRecordV3 {
            schema_version: JOURNAL_SCHEMA_VERSION,
            operation_id: prepared.operation_id.clone(),
            wallet_id: prepared.wallet_id.clone(),
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

    fn journal_key(&self) -> JournalKeyV3 {
        JournalKeyV3 {
            wallet_id: self.wallet_id.clone(),
            slot: SEND_SLOT.to_owned(),
        }
    }

    fn prepared_ref(&self) -> Result<&PreparedTransferV3, SendWorkflowErrorV3> {
        self.prepared
            .as_ref()
            .ok_or(SendWorkflowErrorV3::InvalidTransition {
                from: self.stage,
                event: "prepared_transfer_required",
            })
    }

    fn validate_prepared(&self, prepared: &PreparedTransferV3) -> Result<(), SendWorkflowErrorV3> {
        let account = self
            .fresh_account
            .as_ref()
            .ok_or(SendWorkflowErrorV3::PreparedTransferMismatch)?;
        if prepared.operation_id != self.request.operation_id
            || prepared.wallet_id != self.wallet_id
            || prepared.source != self.source
            || prepared.destination != self.request.destination
            || prepared.amount_nanograms != self.request.amount_nanograms
            || prepared.seqno != account.seqno
            || prepared.needs_state_init != account.needs_state_init()
            || prepared.signed_boc.is_empty()
            || prepared.message_hash.is_empty()
        {
            return Err(SendWorkflowErrorV3::PreparedTransferMismatch);
        }
        Ok(())
    }

    fn expect(
        &self,
        expected: SendStageV3,
        event: &'static str,
    ) -> Result<(), SendWorkflowErrorV3> {
        if self.stage == expected {
            Ok(())
        } else {
            Err(SendWorkflowErrorV3::InvalidTransition {
                from: self.stage,
                event,
            })
        }
    }

    fn fail(&mut self, diagnostic: impl Into<String>) {
        self.stage = SendStageV3::Failed;
        self.diagnostic = Some(sanitize_diagnostic(diagnostic.into()));
    }
}

fn validate_request(
    wallet_id: &str,
    source: &str,
    request: &SendRequestV3,
) -> Result<(), SendWorkflowErrorV3> {
    if wallet_id.trim().is_empty() || source.trim().is_empty() {
        return Err(SendWorkflowErrorV3::InvalidRequest(
            "wallet identity is empty".to_owned(),
        ));
    }
    if request.operation_id.trim().is_empty() || request.operation_id.len() > 128 {
        return Err(SendWorkflowErrorV3::InvalidRequest(
            "operation identifier is invalid".to_owned(),
        ));
    }
    if request.destination.trim().is_empty()
        || request.destination.len() > 128
        || request.destination.chars().any(char::is_whitespace)
    {
        return Err(SendWorkflowErrorV3::InvalidRequest(
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
        return Err(SendWorkflowErrorV3::InvalidRequest(
            "amount must be positive canonical nanograms".to_owned(),
        ));
    }
    if request.secret_ref.value.trim().is_empty() {
        return Err(SendWorkflowErrorV3::InvalidRequest(
            "protected secret reference is empty".to_owned(),
        ));
    }
    Ok(())
}

fn decode_durable_record(
    record: &JournalRecordV3,
) -> Result<DurableSendRecordV3, SendWorkflowErrorV3> {
    if record.version == 0 {
        return Err(SendWorkflowErrorV3::InvalidJournal(
            "version must be positive".to_owned(),
        ));
    }
    let durable: DurableSendRecordV3 = serde_json::from_slice(&record.payload)
        .map_err(|error| SendWorkflowErrorV3::InvalidJournal(error.to_string()))?;
    if durable.schema_version != JOURNAL_SCHEMA_VERSION {
        return Err(SendWorkflowErrorV3::InvalidJournal(
            "unsupported schema version".to_owned(),
        ));
    }
    let boc_is_invalid = match BASE64.decode(&durable.signed_boc_base64) {
        Ok(boc) => boc.is_empty(),
        Err(_) => true,
    };
    if durable.operation_id.trim().is_empty()
        || durable.wallet_id.trim().is_empty()
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
        return Err(SendWorkflowErrorV3::InvalidJournal(
            "record fields are invalid".to_owned(),
        ));
    }
    Ok(durable)
}

fn next_journal_version(current: Option<u64>) -> Result<u64, SendWorkflowErrorV3> {
    match current {
        Some(version) => version
            .checked_add(1)
            .ok_or_else(|| SendWorkflowErrorV3::InvalidJournal("version exhausted".to_owned())),
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
