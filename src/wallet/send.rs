//! The private reducer for durable transfer submission.
//!
//! The reducer produces directives for the wallet client. It never performs
//! callbacks itself. Its journal record prevents a second signature after an
//! ambiguous submission result.

use num_bigint::BigUint;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use ton::ton_core::types::TonAddress;

use crate::Base64Hash;
use crate::domain::{
    AccountStatus, JournalCompareExchange, JournalCompareExchangeResult, JournalKey, JournalRecord,
    PreparedSend, ProtectedSecretRead, SecretAccessReason, SendPhase, SendRequest, SendSnapshot,
    bounded_diagnostic,
};
use crate::types::{Boc, TonAddressExt as _, parse_positive_decimal};

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
    /// The account state from the same fresh provider response used for this send.
    /// Frozen and unknown states stop the operation before secret authorization.
    pub status: AccountStatus,

    /// The current wallet contract sequence number.
    /// A new send after a submitted operation requires this value to increase.
    /// Nonexistent and uninitialized accounts can send only with a zero value.
    pub seqno: u32,

    /// The provider synchronization time as a Unix timestamp.
    /// Transfer expiration uses this value instead of the device clock.
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
    /// The caller identity for this send attempt.
    /// It links the prepared message to the immutable [`SendRequest`].
    pub operation_id: String,

    /// The application record that owns the source wallet and journal slot.
    pub record_id: String,

    /// The configured source address after the mnemonic-derived wallet matches it.
    pub source: TonAddress,

    /// The validated destination TON address from the request.
    pub destination: TonAddress,

    /// The exact transfer value in nanograms.
    /// Arbitrary precision prevents truncation before public or journal serialization.
    pub amount_nanograms: BigUint,

    /// The fresh wallet sequence number signed into the external message.
    pub seqno: u32,

    /// Reports whether the external message contains the wallet `StateInit`.
    /// Only an allowed nonactive account with sequence number zero uses it.
    pub needs_state_init: bool,

    /// The unsigned Unix expiration time signed into the wallet message.
    /// The engine derives it from provider time and the configured validity interval.
    pub valid_until: u32,

    /// The validated signed external-message BOC submitted to Toncenter.
    /// The journal preserves it after an ambiguous transport result.
    pub signed_boc: Boc,

    /// The normalized external-message hash in standard padded Base64.
    /// Applications can use it to locate the submitted message without storing the recovery phrase.
    pub message_hash: Base64Hash,
}

impl PreparedTransfer {
    pub(crate) fn public_summary(&self, network: crate::Network) -> PreparedSend {
        PreparedSend {
            operation_id: self.operation_id.clone(),
            valid_until: self.valid_until,
            destination: self.destination.to_user_friendly(network),
            amount_nanograms: self.amount_nanograms.to_string(),
        }
    }
}

/// Full internal state. Public `SendPhase` intentionally remains a compact
/// UI projection while the engine coordinates host work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SendStage {
    /// The reducer validates the immutable request before it reads persistent state.
    Validating,
    /// The reducer waits for the shared wallet send slot from the durable journal.
    LoadingJournal,
    /// The engine fetches fresh status, synchronization time, and sequence number.
    FetchingFreshAccount,
    /// The platform requests user authorization and reads the protected mnemonic.
    Authorizing,
    /// Rust derives the source wallet and creates the signed external message.
    Preparing,
    /// The platform commits the exact signed BOC before any network submission.
    /// Cancellation becomes too late immediately before this stage starts.
    PersistingPrepared,
    /// The durable commit succeeded, but the submit HTTP request has not started.
    /// Cancellation remains too late because the BOC now survives a process failure.
    ReadyToSubmit,
    /// Toncenter can have received the submitted BOC.
    /// Cancellation is too late in this stage.
    Submitting,
    /// The submit result is ambiguous.
    /// This terminal stage blocks another signature until an external process resolves it.
    SubmissionUnknown,
    /// Toncenter returned an explicit success and the journal stores the terminal result.
    Submitted,
    /// A definite error stopped the operation or Toncenter explicitly rejected the BOC.
    Failed,
    /// Cancellation completed before the durable send boundary.
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
        signed_boc: Boc,
        /// The normalized external-message hash in standard padded Base64.
        message_hash: Base64Hash,
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
    /// The durable JSON schema version.
    /// A reader rejects unknown versions before it uses any other field.
    schema_version: u32,
    /// The caller identity of the send attempt.
    operation_id: String,
    /// The application record that owns the wallet-wide send slot.
    record_id: String,
    /// The source TON address.
    /// Serde stores it as raw `workchain:hex` and accepts friendly values.
    #[serde(with = "crate::types::raw_address_serde")]
    source: TonAddress,
    /// The destination TON address.
    /// Serde stores it as raw `workchain:hex` and accepts friendly values.
    #[serde(with = "crate::types::raw_address_serde")]
    destination: TonAddress,
    /// The exact transfer value as a canonical base-10 nanogram string.
    amount_nanograms: String,
    /// The wallet sequence number signed into the external message.
    seqno: u32,
    /// Reports whether the signed message contains the wallet `StateInit`.
    needs_state_init: bool,
    /// The unsigned Unix expiration time signed into the wallet message.
    valid_until: u32,
    /// The validated signed BOC.
    /// JSON clients receive it as standard padded Base64.
    #[serde(rename = "signed_boc_base64")]
    signed_boc: Boc,
    /// The normalized external-message hash in standard padded Base64.
    message_hash: Base64Hash,
    /// The reducer stage stored by the latest successful journal CAS.
    stage: SendStage,
    /// The optional receipt returned by the provider after submission.
    provider_reference: Option<String>,
    /// A bounded diagnostic for a failed or ambiguous terminal result.
    /// This value contains no recovery phrase, signed BOC, or host credential.
    diagnostic: Option<String>,
}

/// Pure send reducer. The owning coordinator invokes callbacks between reducer
/// calls. It invokes no callback while the reducer is borrowed.
#[derive(Debug, Clone)]
pub(crate) struct SendWorkflow {
    /// The application record that owns the source wallet and shared send journal slot.
    record_id: String,

    /// The expected wallet address from the client configuration.
    /// Secret authorization succeeds only when mnemonic derivation produces this address.
    source: TonAddress,

    /// The immutable caller intent for this operation.
    request: SendRequest,

    /// The current reducer stage.
    /// Each reducer method accepts only its documented predecessor stage.
    stage: SendStage,

    /// Fresh provider state captured before secret authorization.
    /// Preparation uses the same status and sequence number that passed reducer validation.
    fresh_account: Option<FreshSendAccount>,

    /// The signed message produced after authorization.
    /// If this value exists, cancellation must persist a terminal journal record.
    prepared: Option<PreparedTransfer>,

    /// The journal version used by the next compare-and-swap operation.
    /// A missing value means that the shared wallet slot did not exist.
    journal_version: Option<u64>,

    /// The sequence number from the last safely submitted journal record.
    /// The next send waits until fresh chain state contains a larger sequence number.
    prior_submitted_seqno: Option<u32>,

    /// The optional provider receipt stored with a successful terminal record.
    provider_reference: Option<String>,

    /// A bounded developer diagnostic for failed or ambiguous terminal states.
    /// This value must not contain the mnemonic, signed BOC, or host credential.
    diagnostic: Option<String>,
}

impl SendWorkflow {
    pub(crate) const fn new(record_id: String, source: TonAddress, request: SendRequest) -> Self {
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

    pub(crate) fn snapshot(&self) -> SendSnapshot {
        SendSnapshot {
            operation_id: Some(self.request.operation_id.clone()),
            phase: self.stage.public_phase(),
            error_message: self.diagnostic.clone(),
        }
    }

    pub(crate) fn begin(&mut self) -> Result<SendDirective, SendWorkflowError> {
        self.expect(SendStage::Validating, "begin")?;
        validate_request(&self.record_id, &self.request)?;

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
    /// it directly to the wallet transfer builder and zeroizes its temporary buffer.
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

        self.diagnostic = Some(bounded_diagnostic(diagnostic));
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
            amount_nanograms: prepared.amount_nanograms.to_string(),
            seqno: prepared.seqno,
            needs_state_init: prepared.needs_state_init,
            valid_until: prepared.valid_until,
            signed_boc: prepared.signed_boc.clone(),
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
        let request_destination = TonAddress::from_str(&self.request.destination)
            .map_err(|_| SendWorkflowError::PreparedTransferMismatch)?;
        let request_amount = parse_amount_nanograms(&self.request.amount_nanograms)
            .map_err(|_| SendWorkflowError::PreparedTransferMismatch)?;

        if prepared.operation_id != self.request.operation_id
            || prepared.record_id != self.record_id
            || prepared.source != self.source
            || prepared.destination != request_destination
            || prepared.amount_nanograms != request_amount
            || prepared.seqno != account.seqno
            || prepared.needs_state_init != account.needs_state_init()
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
        self.diagnostic = Some(bounded_diagnostic(diagnostic.into()));
    }
}

fn validate_request(record_id: &str, request: &SendRequest) -> Result<(), SendWorkflowError> {
    if record_id.trim().is_empty() {
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
        || TonAddress::from_str(&request.destination).is_err()
    {
        return Err(SendWorkflowError::InvalidRequest(
            "destination is invalid".to_owned(),
        ));
    }

    parse_amount_nanograms(&request.amount_nanograms)?;

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

    if durable.operation_id.trim().is_empty()
        || durable.record_id.trim().is_empty()
        || parse_positive_decimal(&durable.amount_nanograms).is_none()
    {
        return Err(SendWorkflowError::InvalidJournal(
            "record fields are invalid".to_owned(),
        ));
    }

    Ok(durable)
}

fn parse_amount_nanograms(value: &str) -> Result<BigUint, SendWorkflowError> {
    parse_positive_decimal(value).ok_or_else(|| {
        SendWorkflowError::InvalidRequest("amount must be positive canonical nanograms".to_owned())
    })
}

fn next_journal_version(current: Option<u64>) -> Result<u64, SendWorkflowError> {
    match current {
        Some(version) => version
            .checked_add(1)
            .ok_or_else(|| SendWorkflowError::InvalidJournal("version exhausted".to_owned())),
        None => Ok(FIRST_JOURNAL_VERSION),
    }
}
