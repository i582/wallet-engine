//! The private reducer for durable transfer submission.
//!
//! The reducer produces directives for the wallet client. It never performs
//! callbacks itself. Its journal record prevents a second signature after an
//! ambiguous submission result.

use serde::{Deserialize, Serialize};

use crate::domain::{
    AccountStatus, JournalCompareExchange, JournalCompareExchangeResult, JournalKey, JournalRecord,
    PendingReason, ProtectedSecretRead, ProtectedSecretRef, ResolutionInfo, SecretAccessReason,
    SendPhase, SendRequest, SendSnapshot, bounded_diagnostic,
};
use crate::types::Boc;
use crate::{Base64Hash, NonEmptyString, SendAmount, TonAddressString, UnsignedDecimalString};

const JOURNAL_SCHEMA_VERSION: u32 = 2;
const FIRST_JOURNAL_VERSION: u64 = 1;
pub(crate) const SEND_SLOT: &str = "outgoing-transfer";

/// Fresh chain state used to build a wallet transfer.
///
/// It is deliberately supplied to this reducer by the engine. Fetching and
/// parsing provider responses belongs to the HTTP workflow, not to the send
/// state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
pub(crate) struct FreshSendAccount {
    /// The account state from the same fresh provider response used for this send.
    /// Frozen and unknown states stop the operation before secret authorization.
    pub status: AccountStatus,

    /// The current wallet contract sequence number.
    /// A new send after a submitted operation requires this value to increase.
    /// Nonexistent and uninitialized accounts can send only with a zero value.
    pub seqno: u32,
}

impl FreshSendAccount {
    /// Reports whether fresh chain state permits construction of a transfer.
    ///
    /// Active wallets can use any provider seqno. A wallet that does not yet
    /// have executable code can only send its deployment message at seqno zero.
    pub(crate) const fn permits_send(&self) -> bool {
        matches!(self.status, AccountStatus::Active)
            || (self.seqno == 0
                && matches!(
                    self.status,
                    AccountStatus::Nonexistent | AccountStatus::Uninitialized
                ))
    }

    pub(crate) const fn needs_state_init(&self) -> bool {
        !matches!(self.status, AccountStatus::Active)
    }
}

/// Signed material produced inside Rust after the host authorizes access to
/// the protected mnemonic. Secret bytes must not be retained in this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedTransfer {
    /// The caller identity for this send attempt.
    /// It links the prepared message to the immutable [`SendRequest`].
    pub operation_id: NonEmptyString,

    /// The application record that owns the source wallet and journal slot.
    pub record_id: NonEmptyString,

    /// The configured source address after the mnemonic-derived wallet matches it.
    pub source: TonAddressString,

    /// The validated destination TON address from the request.
    pub destination: TonAddressString,

    /// The exact-value or whole-balance policy encoded into the wallet action.
    pub amount: SendAmount,

    /// The optional plaintext comment encoded into the internal message.
    pub comment: Option<String>,

    /// The fresh wallet sequence number signed into the external message.
    pub seqno: u32,

    /// Reports whether the external message contains the wallet `StateInit`.
    /// Only an allowed nonactive account with sequence number zero uses it.
    pub needs_state_init: bool,

    /// The unsigned Unix expiration time signed into the wallet message.
    /// The engine derives it from provider time and the configured validity interval.
    pub valid_until: u64,

    /// The validated signed external-message BOC submitted to Toncenter.
    /// The journal preserves it after an ambiguous transport result.
    pub signed_boc: Boc,

    /// The normalized external-message hash in standard padded Base64.
    /// Applications can use it to locate the submitted message without storing the recovery phrase.
    pub message_hash: Base64Hash,
}

/// Full internal state. Public `SendPhase` intentionally remains a compact
/// UI projection while the engine coordinates host work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
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
    /// The signed external message was executed by an on-chain transaction.
    Confirmed,
    /// Another external message consumed the signed sequence number.
    Replaced,
    /// The signed validity window and indexer margin elapsed without execution.
    Expired,
    /// An explicit same-sequence-number resend replaced this attempt.
    Superseded,
    /// A definite error stopped the operation or Toncenter explicitly rejected the BOC.
    Failed,
    /// Cancellation completed before the durable send boundary.
    Cancelled,
}

/// Events that can change the pure send reducer stage.
///
/// Host work and payload validation happen around this transition table. Keeping
/// stage movement here lets runtime code and bounded verification share one
/// authoritative model of the workflow order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(kani, derive(kani::Arbitrary))]
enum SendEvent {
    Begin,
    JournalLoaded,
    FreshAccountLoaded,
    AuthorizationSucceeded,
    TransferPrepared,
    PreparedJournalPersisted,
    SubmissionStarted,
    SubmissionSucceeded,
    SubmissionUnknown,
    SubmissionRejected,
    TerminalJournalPersisted,
    JournalConflict,
    Cancel,
}

impl SendStage {
    /// Applies one reducer event without performing host work.
    ///
    /// `None` means that the event is out of order. Terminal submission results
    /// are absorbing until the separate resolver writes on-chain evidence.
    const fn transition(self, event: SendEvent) -> Option<Self> {
        match (self, event) {
            (Self::Validating, SendEvent::Begin) => Some(Self::LoadingJournal),
            (Self::LoadingJournal, SendEvent::JournalLoaded) => Some(Self::FetchingFreshAccount),
            (Self::FetchingFreshAccount, SendEvent::FreshAccountLoaded) => Some(Self::Authorizing),
            (Self::Authorizing, SendEvent::AuthorizationSucceeded) => Some(Self::Preparing),
            (Self::Preparing, SendEvent::TransferPrepared) => Some(Self::PersistingPrepared),
            (Self::PersistingPrepared, SendEvent::PreparedJournalPersisted) => {
                Some(Self::ReadyToSubmit)
            }
            (Self::ReadyToSubmit, SendEvent::SubmissionStarted) => Some(Self::Submitting),
            (Self::Submitting, SendEvent::SubmissionSucceeded) => Some(Self::Submitted),
            (Self::Submitting, SendEvent::SubmissionUnknown) => Some(Self::SubmissionUnknown),
            (Self::Submitting, SendEvent::SubmissionRejected) => Some(Self::Failed),
            (
                stage
                @ (Self::SubmissionUnknown | Self::Submitted | Self::Failed | Self::Cancelled),
                SendEvent::TerminalJournalPersisted,
            ) => Some(stage),
            (stage, SendEvent::JournalConflict) if !stage.is_terminal() => Some(Self::Failed),
            (stage, SendEvent::Cancel) => Some(stage.after_cancellation()),
            _ => None,
        }
    }

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
            Self::Confirmed => SendPhase::Confirmed,
            Self::Replaced => SendPhase::Replaced,
            Self::Expired => SendPhase::Expired,
            Self::Superseded => SendPhase::Superseded,
            Self::Failed => SendPhase::Failed,
            Self::Cancelled => SendPhase::Cancelled,
        }
    }

    const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::SubmissionUnknown
                | Self::Submitted
                | Self::Confirmed
                | Self::Replaced
                | Self::Expired
                | Self::Superseded
                | Self::Failed
                | Self::Cancelled
        )
    }

    const fn permits_replacement(self) -> bool {
        matches!(
            self,
            Self::Confirmed
                | Self::Replaced
                | Self::Expired
                | Self::Superseded
                | Self::Failed
                | Self::Cancelled
        )
    }

    /// Returns the stage produced by a cancellation request.
    ///
    /// A terminal result is immutable. Every nonterminal stage can still move
    /// to `Cancelled`; the coordinator separately enforces the earlier durable
    /// commit boundary before it invokes the reducer.
    const fn after_cancellation(self) -> Self {
        if self.is_terminal() {
            self
        } else {
            Self::Cancelled
        }
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
    #[error("invalid send transition from {from:?}: {event}")]
    InvalidTransition {
        from: SendStage,
        event: &'static str,
    },
    #[error("prepared transfer does not match the active send request")]
    PreparedTransferMismatch,
    #[error("send journal was changed by another operation")]
    JournalConflict,
    #[error("the previous submission outcome is unresolved")]
    PreviousSubmissionUnresolved,
    #[error("wallet account state {status:?} does not permit sending")]
    AccountUnavailable { status: AccountStatus },
    #[error("send journal record is invalid: {0}")]
    InvalidJournal(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DurableSendRecord {
    /// The durable JSON schema version.
    /// A reader rejects unknown versions before it uses any other field.
    schema_version: u32,
    /// The caller identity of the send attempt.
    operation_id: NonEmptyString,
    /// The application record that owns the wallet-wide send slot.
    record_id: NonEmptyString,
    /// The validated source TON address.
    source: TonAddressString,
    /// The validated destination TON address.
    destination: TonAddressString,
    /// The exact-value or whole-balance policy stored with the signed message.
    amount: SendAmount,
    /// The optional plaintext comment stored with the signed message.
    #[serde(default)]
    comment: Option<String>,
    /// The wallet sequence number signed into the external message.
    seqno: u32,
    /// Reports whether the signed message contains the wallet `StateInit`.
    needs_state_init: bool,
    /// The unsigned Unix expiration time signed into the wallet message.
    valid_until: u64,
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
    /// Confirmed transaction hash retained as terminal evidence.
    #[serde(default)]
    confirmed_transaction_hash: Option<Base64Hash>,
    /// Confirmed transaction logical time retained as terminal evidence.
    #[serde(default)]
    confirmed_transaction_lt: Option<UnsignedDecimalString>,
}

/// Durable signed send that still needs chain evidence before replacement is safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingSendRecord {
    pub(crate) journal_version: u64,
    pub(crate) operation_id: NonEmptyString,
    pub(crate) record_id: NonEmptyString,
    pub(crate) source: TonAddressString,
    pub(crate) seqno: u32,
    pub(crate) valid_until: u64,
    pub(crate) message_hash: Base64Hash,
    stage: SendStage,
    durable: DurableSendRecord,
    journal: JournalRecord,
}

/// One read-only resolution result for a durable signed send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SendResolution {
    Confirmed {
        transaction_hash: Base64Hash,
        transaction_lt: UnsignedDecimalString,
    },
    Replaced,
    Expired,
    StillPending(PendingReason),
}

impl PendingSendRecord {
    /// Returns the exact journal version inspected by the resolver for a
    /// non-terminal result that must not mutate durable state.
    pub(crate) fn current_journal(&self) -> JournalRecord {
        self.journal.clone()
    }

    /// Converts internal evidence into the public send snapshot while retaining
    /// the original operation identity and bounded submission diagnostic.
    pub(crate) fn snapshot(&self, resolution: &SendResolution) -> SendSnapshot {
        let (phase, transaction_hash, transaction_lt, pending_reason) = match resolution {
            SendResolution::Confirmed {
                transaction_hash,
                transaction_lt,
            } => (
                SendPhase::Confirmed,
                Some(transaction_hash.clone()),
                Some(transaction_lt.clone()),
                None,
            ),
            SendResolution::Replaced => (SendPhase::Replaced, None, None, None),
            SendResolution::Expired => (SendPhase::Expired, None, None, None),
            SendResolution::StillPending(reason) => {
                (self.stage.public_phase(), None, None, Some(*reason))
            }
        };

        SendSnapshot {
            operation_id: Some(self.operation_id.clone()),
            phase,
            error_message: self.durable.diagnostic.clone(),
            resolution: Some(ResolutionInfo {
                transaction_hash,
                transaction_lt,
                pending_reason,
                can_force_retry: false,
                retry_after_hint_ms: pending_reason.map(|_| 4_000),
            }),
        }
    }

    /// Builds a forward-only CAS mutation for terminal evidence.
    ///
    /// `StillPending` intentionally produces no mutation: lack of evidence must
    /// never rewrite the durable record or unlock replacement signing.
    pub(crate) fn terminal_mutation(
        &self,
        resolution: &SendResolution,
    ) -> Result<Option<JournalCompareExchange>, SendWorkflowError> {
        // Resolution augments the existing forensic record instead of replacing
        // it with a smaller status object. The original signed BOC, seqno, and
        // intent remain available after a crash or a future schema migration.
        let (stage, transaction_hash, transaction_lt) = match resolution {
            SendResolution::Confirmed {
                transaction_hash,
                transaction_lt,
            } => (
                SendStage::Confirmed,
                Some(transaction_hash.clone()),
                Some(transaction_lt.clone()),
            ),
            SendResolution::Replaced => (SendStage::Replaced, None, None),
            SendResolution::Expired => (SendStage::Expired, None, None),
            SendResolution::StillPending(_) => return Ok(None),
        };

        let mut durable = self.durable.clone();
        durable.stage = stage;
        durable.diagnostic = None;
        durable.confirmed_transaction_hash = transaction_hash;
        durable.confirmed_transaction_lt = transaction_lt;
        let replacement_version = next_journal_version(Some(self.journal_version))?;

        Ok(Some(JournalCompareExchange {
            key: JournalKey {
                record_id: self.record_id.to_string(),
                slot: SEND_SLOT.to_owned(),
            },
            expected_version: Some(self.journal_version),
            replacement: JournalRecord {
                version: replacement_version,
                payload: serde_json::to_vec(&durable)
                    .map_err(|error| SendWorkflowError::InvalidJournal(error.to_string()))?,
            },
        }))
    }
}

/// Validates a journal record's wallet ownership and returns it only when its
/// stage still blocks replacement signing.
pub(crate) fn pending_send_record(
    record: &JournalRecord,
    record_id: &NonEmptyString,
    source: &TonAddressString,
) -> Result<Option<PendingSendRecord>, SendWorkflowError> {
    let durable = decode_durable_record(record)?;
    if &durable.record_id != record_id || &durable.source != source {
        return Err(SendWorkflowError::InvalidJournal(
            "record belongs to another wallet".to_owned(),
        ));
    }
    if durable.stage.permits_replacement() {
        return Ok(None);
    }

    Ok(Some(PendingSendRecord {
        journal_version: record.version,
        operation_id: durable.operation_id.clone(),
        record_id: durable.record_id.clone(),
        source: durable.source.clone(),
        seqno: durable.seqno,
        valid_until: durable.valid_until,
        message_hash: durable.message_hash.clone(),
        stage: durable.stage,
        durable,
        journal: record.clone(),
    }))
}

/// Reads a resolver terminal from a competing CAS winner, requiring complete
/// transaction evidence for `Confirmed` records.
pub(crate) fn terminal_send_resolution(
    record: &JournalRecord,
    record_id: &NonEmptyString,
    source: &TonAddressString,
) -> Result<Option<SendResolution>, SendWorkflowError> {
    let durable = decode_durable_record(record)?;
    if &durable.record_id != record_id || &durable.source != source {
        return Err(SendWorkflowError::InvalidJournal(
            "record belongs to another wallet".to_owned(),
        ));
    }

    match durable.stage {
        SendStage::Confirmed => Ok(Some(SendResolution::Confirmed {
            transaction_hash: durable.confirmed_transaction_hash.ok_or_else(|| {
                SendWorkflowError::InvalidJournal(
                    "confirmed record has no transaction hash".to_owned(),
                )
            })?,
            transaction_lt: durable.confirmed_transaction_lt.ok_or_else(|| {
                SendWorkflowError::InvalidJournal(
                    "confirmed record has no transaction logical time".to_owned(),
                )
            })?,
        })),
        SendStage::Replaced => Ok(Some(SendResolution::Replaced)),
        SendStage::Expired => Ok(Some(SendResolution::Expired)),
        SendStage::Validating
        | SendStage::LoadingJournal
        | SendStage::FetchingFreshAccount
        | SendStage::Authorizing
        | SendStage::Preparing
        | SendStage::PersistingPrepared
        | SendStage::ReadyToSubmit
        | SendStage::Submitting
        | SendStage::SubmissionUnknown
        | SendStage::Submitted
        | SendStage::Superseded
        | SendStage::Failed
        | SendStage::Cancelled => Ok(None),
    }
}

/// Reconstructs observable send state during restart even when the journal is
/// already terminal and no provider request is necessary.
pub(crate) fn send_snapshot_from_journal(
    record: &JournalRecord,
    record_id: &NonEmptyString,
    source: &TonAddressString,
) -> Result<SendSnapshot, SendWorkflowError> {
    let durable = decode_durable_record(record)?;
    if &durable.record_id != record_id || &durable.source != source {
        return Err(SendWorkflowError::InvalidJournal(
            "record belongs to another wallet".to_owned(),
        ));
    }

    let resolution = match durable.stage {
        SendStage::Confirmed => Some(ResolutionInfo {
            transaction_hash: durable.confirmed_transaction_hash,
            transaction_lt: durable.confirmed_transaction_lt,
            pending_reason: None,
            can_force_retry: false,
            retry_after_hint_ms: None,
        }),
        SendStage::Replaced | SendStage::Expired | SendStage::Superseded => Some(ResolutionInfo {
            transaction_hash: None,
            transaction_lt: None,
            pending_reason: None,
            can_force_retry: false,
            retry_after_hint_ms: None,
        }),
        SendStage::Validating
        | SendStage::LoadingJournal
        | SendStage::FetchingFreshAccount
        | SendStage::Authorizing
        | SendStage::Preparing
        | SendStage::PersistingPrepared
        | SendStage::ReadyToSubmit
        | SendStage::Submitting
        | SendStage::SubmissionUnknown
        | SendStage::Submitted => Some(ResolutionInfo {
            transaction_hash: None,
            transaction_lt: None,
            pending_reason: Some(PendingReason::AwaitingWindow),
            can_force_retry: false,
            retry_after_hint_ms: Some(4_000),
        }),
        SendStage::Failed | SendStage::Cancelled => None,
    };

    Ok(SendSnapshot {
        operation_id: Some(durable.operation_id),
        phase: durable.stage.public_phase(),
        error_message: durable.diagnostic,
        resolution,
    })
}

/// Pure send reducer. The owning coordinator invokes callbacks between reducer
/// calls. It invokes no callback while the reducer is borrowed.
#[derive(Debug, Clone)]
pub(crate) struct SendWorkflow {
    /// The application record that owns the source wallet and shared send journal slot.
    record_id: NonEmptyString,

    /// The expected wallet address from the client configuration.
    /// Secret authorization succeeds only when mnemonic derivation produces this address.
    source: TonAddressString,

    /// The immutable caller intent for this operation.
    request: SendRequest,

    /// The wallet-level protected secret used only by the local signing path.
    local_secret_ref: ProtectedSecretRef,

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

    /// The optional provider receipt stored with a successful terminal record.
    provider_reference: Option<String>,

    /// A bounded developer diagnostic for failed or ambiguous terminal states.
    /// This value must not contain the mnemonic, signed BOC, or host credential.
    diagnostic: Option<String>,
}

impl SendWorkflow {
    pub(crate) const fn new(
        record_id: NonEmptyString,
        source: TonAddressString,
        request: SendRequest,
        local_secret_ref: ProtectedSecretRef,
    ) -> Self {
        Self {
            record_id,
            source,
            request,
            local_secret_ref,
            stage: SendStage::Validating,
            fresh_account: None,
            prepared: None,
            journal_version: None,
            provider_reference: None,
            diagnostic: None,
        }
    }

    pub(crate) fn snapshot(&self) -> SendSnapshot {
        SendSnapshot {
            operation_id: Some(self.request.operation_id.clone()),
            phase: self.stage.public_phase(),
            error_message: self.diagnostic.clone(),
            resolution: None,
        }
    }

    pub(crate) fn begin(&mut self) -> Result<SendDirective, SendWorkflowError> {
        self.apply_event(SendEvent::Begin, "begin")?;

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
        let next_stage = self.next_stage(SendEvent::JournalLoaded, "journal_loaded")?;

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
                    SendStage::Confirmed
                    | SendStage::Replaced
                    | SendStage::Expired
                    | SendStage::Superseded
                    | SendStage::Failed
                    | SendStage::Cancelled => Some(record.version),
                    SendStage::Validating
                    | SendStage::LoadingJournal
                    | SendStage::FetchingFreshAccount
                    | SendStage::Authorizing
                    | SendStage::Preparing
                    | SendStage::PersistingPrepared
                    | SendStage::ReadyToSubmit
                    | SendStage::Submitting
                    | SendStage::SubmissionUnknown
                    | SendStage::Submitted => {
                        return Err(SendWorkflowError::PreviousSubmissionUnresolved);
                    }
                }
            }
        };

        self.stage = next_stage;

        Ok(SendDirective::FetchFreshAccount)
    }

    pub(crate) fn fresh_account_loaded(
        &mut self,
        account: FreshSendAccount,
    ) -> Result<SendDirective, SendWorkflowError> {
        let next_stage = self.next_stage(SendEvent::FreshAccountLoaded, "fresh_account_loaded")?;

        if !account.permits_send() {
            return Err(SendWorkflowError::AccountUnavailable {
                status: account.status,
            });
        }

        self.fresh_account = Some(account);
        self.stage = next_stage;
        Ok(self.read_secret_directive())
    }

    /// Marks successful host authorization.
    ///
    /// The reducer does not accept or retain the secret. The coordinator passes
    /// it directly to the wallet transfer builder and zeroizes its temporary buffer.
    pub(crate) fn authorization_succeeded(&mut self) -> Result<SendDirective, SendWorkflowError> {
        let next_stage =
            self.next_stage(SendEvent::AuthorizationSucceeded, "authorization_succeeded")?;
        let account = self
            .fresh_account
            .clone()
            .ok_or(SendWorkflowError::InvalidTransition {
                from: self.stage,
                event: "authorization_without_fresh_account",
            })?;

        self.stage = next_stage;

        Ok(SendDirective::PrepareTransfer {
            request: self.request.clone(),
            account,
        })
    }

    pub(crate) fn transfer_prepared(
        &mut self,
        prepared: PreparedTransfer,
    ) -> Result<SendDirective, SendWorkflowError> {
        let next_stage = self.next_stage(SendEvent::TransferPrepared, "transfer_prepared")?;
        self.validate_prepared(&prepared)?;

        self.prepared = Some(prepared);
        self.stage = next_stage;

        self.persist_directive()
    }

    pub(crate) fn journal_persisted(
        &mut self,
        result: &JournalCompareExchangeResult,
    ) -> Result<SendDirective, SendWorkflowError> {
        if !result.applied {
            self.apply_event(SendEvent::JournalConflict, "journal_conflict")?;
            self.diagnostic = Some(bounded_diagnostic(
                "Another send operation changed the journal",
            ));
            return Err(SendWorkflowError::JournalConflict);
        }

        let event = if self.stage == SendStage::PersistingPrepared {
            SendEvent::PreparedJournalPersisted
        } else {
            SendEvent::TerminalJournalPersisted
        };
        let next_stage = self.next_stage(event, "journal_persisted")?;
        let next_version = next_journal_version(self.journal_version)?;
        self.journal_version = Some(next_version);
        self.stage = next_stage;

        match next_stage {
            SendStage::ReadyToSubmit => {
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
            stage @ (SendStage::Validating
            | SendStage::LoadingJournal
            | SendStage::FetchingFreshAccount
            | SendStage::Authorizing
            | SendStage::Preparing
            | SendStage::PersistingPrepared
            | SendStage::Submitting
            | SendStage::Confirmed
            | SendStage::Replaced
            | SendStage::Expired
            | SendStage::Superseded) => Err(SendWorkflowError::InvalidTransition {
                from: stage,
                event: "journal_persisted",
            }),
        }
    }

    pub(crate) fn submission_started(&mut self) -> Result<(), SendWorkflowError> {
        self.apply_event(SendEvent::SubmissionStarted, "submission_started")?;

        Ok(())
    }

    pub(crate) fn submission_succeeded(
        &mut self,
        provider_reference: Option<String>,
    ) -> Result<SendDirective, SendWorkflowError> {
        self.apply_event(SendEvent::SubmissionSucceeded, "submission_succeeded")?;

        self.provider_reference = provider_reference;
        self.diagnostic = None;

        self.persist_directive()
    }

    /// A timeout or connection loss after submission is not a definite
    /// failure. The provider can have accepted the exact persisted BOC.
    /// This remains pending until the resolver records chain evidence. A new
    /// transfer cannot be signed while this journal record is unresolved.
    pub(crate) fn submission_unknown(
        &mut self,
        diagnostic: String,
    ) -> Result<SendDirective, SendWorkflowError> {
        self.apply_event(SendEvent::SubmissionUnknown, "submission_unknown")?;

        self.diagnostic = Some(bounded_diagnostic(diagnostic));

        self.persist_directive()
    }

    pub(crate) fn submission_rejected(
        &mut self,
        diagnostic: String,
    ) -> Result<SendDirective, SendWorkflowError> {
        self.apply_event(SendEvent::SubmissionRejected, "submission_rejected")?;
        self.diagnostic = Some(bounded_diagnostic(diagnostic));

        self.persist_directive()
    }

    pub(crate) fn cancel(&mut self) -> Result<SendDirective, SendWorkflowError> {
        let cancelled_stage = self.stage.transition(SendEvent::Cancel).ok_or(
            SendWorkflowError::InvalidTransition {
                from: self.stage,
                event: "cancel",
            },
        )?;
        if cancelled_stage == self.stage {
            return Ok(SendDirective::Finished);
        }

        self.stage = cancelled_stage;

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
            amount: prepared.amount.clone(),
            comment: prepared.comment.clone(),
            seqno: prepared.seqno,
            needs_state_init: prepared.needs_state_init,
            valid_until: prepared.valid_until,
            signed_boc: prepared.signed_boc.clone(),
            message_hash: prepared.message_hash.clone(),
            stage: self.stage,
            provider_reference: self.provider_reference.clone(),
            diagnostic: self.diagnostic.clone(),
            confirmed_transaction_hash: None,
            confirmed_transaction_lt: None,
        })
    }

    fn journal_key(&self) -> JournalKey {
        JournalKey {
            record_id: self.record_id.to_string(),
            slot: SEND_SLOT.to_owned(),
        }
    }

    fn read_secret_directive(&self) -> SendDirective {
        SendDirective::ReadProtectedSecret(ProtectedSecretRead {
            secret_ref: self.local_secret_ref.clone(),
            reason: SecretAccessReason::SignTransfer,
            prompt: "Authenticate to sign this GRAM transfer".to_owned(),
        })
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
            || prepared.amount != self.request.amount
            || prepared.comment != self.request.comment
            || prepared.seqno != account.seqno
            || prepared.needs_state_init != account.needs_state_init()
        {
            return Err(SendWorkflowError::PreparedTransferMismatch);
        }

        Ok(())
    }

    fn next_stage(
        &self,
        event: SendEvent,
        event_name: &'static str,
    ) -> Result<SendStage, SendWorkflowError> {
        self.stage
            .transition(event)
            .ok_or(SendWorkflowError::InvalidTransition {
                from: self.stage,
                event: event_name,
            })
    }

    fn apply_event(
        &mut self,
        event: SendEvent,
        event_name: &'static str,
    ) -> Result<(), SendWorkflowError> {
        self.stage = self.next_stage(event, event_name)?;
        Ok(())
    }
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

    if durable.stage == SendStage::Confirmed
        && (durable.confirmed_transaction_hash.is_none()
            || durable
                .confirmed_transaction_lt
                .as_ref()
                .is_none_or(|lt| lt.try_to::<u64>().is_err()))
    {
        return Err(SendWorkflowError::InvalidJournal(
            "confirmed record evidence is invalid".to_owned(),
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

/// Exhaustive checks for the small, security-sensitive part of the send model.
///
/// These harnesses intentionally avoid HTTP, serialization, and cryptography.
/// Kani can therefore explore every journal version and every internal send
/// stage instead of sampling a few examples as a unit test would.
#[cfg(kani)]
mod verification {
    use super::*;

    /// Proves that journal compare-and-swap versions increase exactly once and
    /// that exhaustion is reported instead of wrapping to zero.
    #[kani::proof]
    fn journal_version_increments_without_wrapping() {
        let current = kani::any::<Option<u64>>();
        let result = next_journal_version(current);

        match current {
            None => assert_eq!(result, Ok(FIRST_JOURNAL_VERSION)),
            Some(u64::MAX) => assert!(result.is_err()),
            Some(version) => {
                assert!(result.is_ok());
                if let Ok(next) = result {
                    assert_eq!(next, version + 1);
                    assert!(next > version);
                }
            }
        }
    }

    /// Proves that replacement signing is enabled only by durable terminal
    /// evidence. Provider acceptance and ambiguous submission remain blocking
    /// until the resolver records an on-chain outcome.
    #[kani::proof]
    fn only_resolved_terminal_stages_permit_replacement() {
        let stage = kani::any::<SendStage>();

        if stage.permits_replacement() {
            assert!(stage.is_terminal());
        }
        if matches!(stage, SendStage::SubmissionUnknown | SendStage::Submitted) {
            assert!(stage.is_terminal());
            assert!(!stage.permits_replacement());
        }
        if stage.is_terminal()
            && !matches!(stage, SendStage::SubmissionUnknown | SendStage::Submitted)
        {
            assert!(stage.permits_replacement());
        }
    }

    /// Proves that cancellation cannot rewrite an existing terminal result and
    /// always turns a nonterminal reducer stage into the terminal cancelled state.
    #[kani::proof]
    fn cancellation_preserves_terminal_results() {
        let stage = kani::any::<SendStage>();
        let cancelled = stage.after_cancellation();

        if stage.is_terminal() {
            assert_eq!(cancelled, stage);
        } else {
            assert_eq!(cancelled, SendStage::Cancelled);
            assert!(cancelled.permits_replacement());
        }
        assert!(cancelled.is_terminal());
    }

    /// Proves the complete account-state policy for every status and every
    /// possible wallet sequence number.
    #[kani::proof]
    fn fresh_account_policy_accepts_only_signable_states() {
        let account = kani::any::<FreshSendAccount>();

        match account.status {
            AccountStatus::Active => {
                assert!(account.permits_send());
                assert!(!account.needs_state_init());
            }
            AccountStatus::Nonexistent | AccountStatus::Uninitialized => {
                assert_eq!(account.permits_send(), account.seqno == 0);
                assert!(account.needs_state_init());
            }
            AccountStatus::Frozen | AccountStatus::Unknown => {
                assert!(!account.permits_send());
            }
        }
    }

    /// Proves that every accepted deployment uses seqno zero and that no active
    /// wallet accidentally includes `StateInit`.
    #[kani::proof]
    fn state_init_is_used_only_for_a_first_deployment() {
        let account = kani::any::<FreshSendAccount>();

        if account.permits_send() && account.needs_state_init() {
            assert_eq!(account.seqno, 0);
            assert!(matches!(
                account.status,
                AccountStatus::Nonexistent | AccountStatus::Uninitialized
            ));
        }
        if account.permits_send() && !account.needs_state_init() {
            assert_eq!(account.status, AccountStatus::Active);
        }
    }

    /// Explores arbitrary ordered and out-of-order reducer events as one bounded
    /// workflow instead of checking a single transition in isolation.
    ///
    /// The proof tracks the historical durable-commit fact separately from the
    /// current stage. Submission-capable stages must never become reachable
    /// before the prepared BOC has crossed that boundary.
    #[kani::proof]
    #[kani::unwind(11)]
    fn arbitrary_event_sequences_cannot_submit_before_durable_persistence() {
        let events = kani::any::<[SendEvent; 10]>();
        let mut stage = SendStage::Validating;
        let mut prepared_is_durable = false;
        let mut reached_submitted = false;
        let mut reached_submission_unknown = false;

        for event in events {
            let previous = stage;
            if let Some(next) = previous.transition(event) {
                if previous == SendStage::PersistingPrepared
                    && event == SendEvent::PreparedJournalPersisted
                {
                    prepared_is_durable = true;
                }
                stage = next;
            } else {
                assert_eq!(stage, previous);
            }

            if matches!(
                stage,
                SendStage::ReadyToSubmit
                    | SendStage::Submitting
                    | SendStage::SubmissionUnknown
                    | SendStage::Submitted
            ) {
                assert!(prepared_is_durable);
            }
            if matches!(stage, SendStage::SubmissionUnknown | SendStage::Submitted) {
                assert!(!stage.permits_replacement());
            }

            reached_submitted |= stage == SendStage::Submitted;
            reached_submission_unknown |= stage == SendStage::SubmissionUnknown;
        }

        // Ensure that both terminal submission branches are genuinely reachable
        // within the bound; otherwise their safety assertions could pass vacuously.
        kani::cover!(reached_submitted);
        kani::cover!(reached_submission_unknown);
    }

    /// Proves that send-reducer events cannot rewrite any terminal result. Only
    /// the separate resolver is allowed to replace pending submission evidence.
    #[kani::proof]
    fn terminal_send_stages_are_absorbing() {
        let stage = kani::any::<SendStage>();
        let event = kani::any::<SendEvent>();

        kani::assume(stage.is_terminal());
        kani::cover!();
        if let Some(next) = stage.transition(event) {
            assert_eq!(next, stage);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NonEmptyString, ProtectedSecretRef, TonAddressString};
    use ton::ton_core::cell::TonCell;

    const RAW_SOURCE: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";
    const RAW_DESTINATION: &str =
        "0:2222222222222222222222222222222222222222222222222222222222222222";
    const RAW_OTHER: &str = "0:3333333333333333333333333333333333333333333333333333333333333333";

    #[test]
    fn workflow_accepts_a_long_nonempty_operation_identifier() {
        let request = SendRequest {
            operation_id: NonEmptyString::try_from("x".repeat(1_024))
                .expect("long operation remains non-empty"),
            ..request()
        };
        let mut workflow =
            SendWorkflow::new(non_empty("record"), source(), request, local_secret_ref());

        assert!(matches!(
            workflow.begin(),
            Ok(SendDirective::LoadJournal(_))
        ));
    }

    #[test]
    fn journal_loading_rejects_corrupt_or_foreign_records() {
        let records = [
            JournalRecord {
                version: 0,
                payload: durable_payload(SendStage::Submitted),
            },
            JournalRecord {
                version: 1,
                payload: b"not-json".to_vec(),
            },
            JournalRecord {
                version: 1,
                payload: durable_payload_with(|record| {
                    record.schema_version = JOURNAL_SCHEMA_VERSION + 1;
                }),
            },
            JournalRecord {
                version: 1,
                payload: durable_payload_with_json_field("operation_id", ""),
            },
            JournalRecord {
                version: 1,
                payload: durable_payload_with(|record| record.record_id = non_empty("other")),
            },
        ];

        for record in records {
            let mut workflow = workflow();
            assert!(matches!(
                workflow.begin(),
                Ok(SendDirective::LoadJournal(_))
            ));
            assert!(matches!(
                workflow.journal_loaded(Some(record)),
                Err(SendWorkflowError::InvalidJournal(_))
            ));
        }
    }

    #[test]
    fn every_journal_reader_rejects_each_foreign_identity_field_independently() {
        let foreign_record_id = JournalRecord {
            version: 1,
            payload: durable_payload_with(|record| record.record_id = non_empty("other")),
        };
        let foreign_source = JournalRecord {
            version: 1,
            payload: durable_payload_with(|record| record.source = other_address()),
        };

        for record in [&foreign_record_id, &foreign_source] {
            assert!(matches!(
                pending_send_record(record, &non_empty("record"), &source()),
                Err(SendWorkflowError::InvalidJournal(_))
            ));
            assert!(matches!(
                terminal_send_resolution(record, &non_empty("record"), &source()),
                Err(SendWorkflowError::InvalidJournal(_))
            ));
            assert!(matches!(
                send_snapshot_from_journal(record, &non_empty("record"), &source()),
                Err(SendWorkflowError::InvalidJournal(_))
            ));
        }
    }

    #[test]
    fn durable_records_from_before_comment_support_remain_compatible() {
        let mut payload: serde_json::Value =
            serde_json::from_slice(&durable_payload(SendStage::Cancelled))
                .expect("durable fixture is JSON");
        payload
            .as_object_mut()
            .expect("durable fixture is an object")
            .remove("comment");
        payload
            .as_object_mut()
            .expect("durable fixture is an object")
            .remove("confirmed_transaction_hash");
        payload
            .as_object_mut()
            .expect("durable fixture is an object")
            .remove("confirmed_transaction_lt");
        let record = JournalRecord {
            version: 1,
            payload: serde_json::to_vec(&payload).expect("legacy durable fixture serializes"),
        };

        let decoded = decode_durable_record(&record).expect("legacy durable record decodes");
        assert_eq!(decoded.comment, None);
    }

    #[test]
    fn terminal_resolution_persists_evidence_with_a_forward_cas() {
        let journal = JournalRecord {
            version: 4,
            payload: durable_payload(SendStage::SubmissionUnknown),
        };
        let pending = pending_send_record(&journal, &non_empty("record"), &source())
            .expect("durable record is valid")
            .expect("unknown submission needs resolution");
        let transaction_hash =
            Base64Hash::from_bytes(&[9; 32]).expect("the hash fixture has 32 bytes");
        let resolution = SendResolution::Confirmed {
            transaction_hash: transaction_hash.clone(),
            transaction_lt: UnsignedDecimalString::try_from("42").expect("valid transaction lt"),
        };
        let mutation = pending
            .terminal_mutation(&resolution)
            .expect("terminal evidence serializes")
            .expect("confirmed is terminal");

        assert_eq!(mutation.expected_version, Some(4));
        assert_eq!(mutation.replacement.version, 5);
        assert_eq!(
            terminal_send_resolution(&mutation.replacement, &non_empty("record"), &source()),
            Ok(Some(SendResolution::Confirmed {
                transaction_hash,
                transaction_lt: UnsignedDecimalString::try_from("42")
                    .expect("valid transaction lt"),
            }))
        );
    }

    #[test]
    fn confirmed_record_without_complete_evidence_is_rejected() {
        let transaction_hash =
            Base64Hash::from_bytes(&[7; 32]).expect("the hash fixture has 32 bytes");
        let transaction_lt = UnsignedDecimalString::try_from("42").expect("valid transaction lt");
        let records = [
            JournalRecord {
                version: 1,
                payload: durable_payload_with(|record| {
                    record.stage = SendStage::Confirmed;
                    record.confirmed_transaction_hash = Some(transaction_hash.clone());
                }),
            },
            JournalRecord {
                version: 1,
                payload: durable_payload_with(|record| {
                    record.stage = SendStage::Confirmed;
                    record.confirmed_transaction_lt = Some(transaction_lt.clone());
                }),
            },
        ];

        for record in records {
            assert!(matches!(
                decode_durable_record(&record),
                Err(SendWorkflowError::InvalidJournal(message))
                    if message == "confirmed record evidence is invalid"
            ));
        }
    }

    #[test]
    fn cancellation_cannot_replace_a_completed_submission() {
        let mut workflow = ready_to_submit_workflow();
        assert_eq!(workflow.submission_started(), Ok(()));
        assert!(matches!(
            workflow.submission_succeeded(Some("receipt-7".to_owned())),
            Ok(SendDirective::PersistJournal(_))
        ));
        assert_eq!(
            workflow.journal_persisted(&applied()),
            Ok(SendDirective::Finished)
        );

        assert_eq!(workflow.cancel(), Ok(SendDirective::Finished));
        assert_eq!(workflow.snapshot().phase, SendPhase::Submitted);
    }

    #[test]
    fn every_nonreplaceable_durable_stage_blocks_a_new_signature() {
        for stage in [
            SendStage::LoadingJournal,
            SendStage::FetchingFreshAccount,
            SendStage::Authorizing,
            SendStage::Preparing,
            SendStage::PersistingPrepared,
            SendStage::ReadyToSubmit,
            SendStage::Submitting,
            SendStage::SubmissionUnknown,
        ] {
            let mut workflow = workflow();
            assert!(workflow.begin().is_ok());
            let record = JournalRecord {
                version: 1,
                payload: durable_payload(stage),
            };
            assert_eq!(
                workflow.journal_loaded(Some(record)),
                Err(SendWorkflowError::PreviousSubmissionUnresolved),
                "stage {stage:?} must keep the shared send slot blocked"
            );
        }
    }

    #[test]
    fn cancelled_and_failed_records_are_safe_to_replace() {
        for stage in [SendStage::Cancelled, SendStage::Failed] {
            let mut workflow = workflow();
            assert!(workflow.begin().is_ok());
            let record = JournalRecord {
                version: 8,
                payload: durable_payload(stage),
            };
            assert_eq!(
                workflow.journal_loaded(Some(record)),
                Ok(SendDirective::FetchFreshAccount)
            );
            assert_eq!(workflow.journal_version, Some(8));
        }
    }

    #[test]
    fn undeployed_account_with_nonzero_seqno_is_rejected() {
        for status in [AccountStatus::Nonexistent, AccountStatus::Uninitialized] {
            let mut workflow = workflow();
            assert!(workflow.begin().is_ok());
            assert_eq!(
                workflow.journal_loaded(None),
                Ok(SendDirective::FetchFreshAccount)
            );
            assert_eq!(
                workflow.fresh_account_loaded(FreshSendAccount { status, seqno: 1 }),
                Err(SendWorkflowError::AccountUnavailable { status })
            );
        }
    }

    #[test]
    fn cancellation_is_terminal_and_persists_only_when_a_boc_exists() {
        let mut early = workflow();
        assert!(early.begin().is_ok());
        assert_eq!(early.cancel(), Ok(SendDirective::Finished));
        assert_eq!(early.snapshot().phase, SendPhase::Cancelled);
        assert_eq!(early.cancel(), Ok(SendDirective::Finished));

        let (mut prepared, transfer) = preparing_workflow();
        assert!(matches!(
            prepared.transfer_prepared(transfer),
            Ok(SendDirective::PersistJournal(_))
        ));
        assert!(matches!(
            prepared.cancel(),
            Ok(SendDirective::PersistJournal(_))
        ));
        assert_eq!(prepared.snapshot().phase, SendPhase::Cancelled);
        assert_eq!(
            prepared.journal_persisted(&applied()),
            Ok(SendDirective::Finished)
        );
    }

    #[test]
    fn successful_terminal_persistence_keeps_the_provider_reference() {
        let mut workflow = ready_to_submit_workflow();
        assert_eq!(workflow.snapshot().phase, SendPhase::ReadyToSubmit);
        assert_eq!(workflow.submission_started(), Ok(()));
        assert!(matches!(
            workflow.submission_succeeded(Some("receipt-7".to_owned())),
            Ok(SendDirective::PersistJournal(_))
        ));
        assert_eq!(
            workflow.journal_persisted(&applied()),
            Ok(SendDirective::Finished)
        );
        assert_eq!(workflow.snapshot().phase, SendPhase::Submitted);
        assert_eq!(workflow.provider_reference.as_deref(), Some("receipt-7"));
    }

    #[test]
    fn reducer_rejects_out_of_order_events_without_changing_stage() {
        let mut workflow = workflow();
        assert!(matches!(
            workflow.authorization_succeeded(),
            Err(SendWorkflowError::InvalidTransition {
                from: SendStage::Validating,
                event: "authorization_succeeded"
            })
        ));
        assert!(matches!(
            workflow.journal_persisted(&applied()),
            Err(SendWorkflowError::InvalidTransition {
                from: SendStage::Validating,
                event: "journal_persisted"
            })
        ));
        assert_eq!(workflow.snapshot().phase, SendPhase::Validating);
    }

    #[test]
    fn prepared_transfer_must_match_every_bound_send_field() {
        type TransferMutation = Box<dyn FnOnce(&mut PreparedTransfer)>;

        let mismatches: Vec<TransferMutation> = vec![
            Box::new(|prepared| prepared.operation_id = non_empty("other-operation")),
            Box::new(|prepared| prepared.record_id = non_empty("other-record")),
            Box::new(|prepared| prepared.source = other_address()),
            Box::new(|prepared| prepared.destination = other_address()),
            Box::new(|prepared| {
                prepared.amount = SendAmount::exact("2").expect("valid exact amount");
            }),
            Box::new(|prepared| prepared.comment = Some("different".to_owned())),
            Box::new(|prepared| prepared.seqno = 8),
            Box::new(|prepared| prepared.needs_state_init = true),
        ];

        for mutate in mismatches {
            let (mut workflow, mut prepared) = preparing_workflow();
            mutate(&mut prepared);
            assert_eq!(
                workflow.transfer_prepared(prepared),
                Err(SendWorkflowError::PreparedTransferMismatch)
            );
            assert_eq!(workflow.snapshot().phase, SendPhase::Preparing);
        }
    }

    #[test]
    fn journal_version_exhaustion_is_reported_before_serialization() {
        let (mut workflow, prepared) = preparing_workflow();
        workflow.journal_version = Some(u64::MAX);

        assert_eq!(
            workflow.transfer_prepared(prepared),
            Err(SendWorkflowError::InvalidJournal(
                "version exhausted".to_owned()
            ))
        );
    }

    fn workflow() -> SendWorkflow {
        SendWorkflow::new(non_empty("record"), source(), request(), local_secret_ref())
    }

    fn request() -> SendRequest {
        SendRequest {
            operation_id: NonEmptyString::try_from("operation").expect("valid operation"),
            destination: TonAddressString::try_from(RAW_DESTINATION)
                .expect("valid destination address"),
            amount: SendAmount::exact("1").expect("valid exact amount"),
            comment: None,
        }
    }

    fn local_secret_ref() -> ProtectedSecretRef {
        ProtectedSecretRef {
            value: "secret".to_owned(),
        }
    }

    fn source() -> TonAddressString {
        TonAddressString::try_from(RAW_SOURCE).expect("test source address is valid")
    }

    fn destination() -> TonAddressString {
        TonAddressString::try_from(RAW_DESTINATION).expect("test destination address is valid")
    }

    fn other_address() -> TonAddressString {
        TonAddressString::try_from(RAW_OTHER).expect("test alternate address is valid")
    }

    fn fresh_account() -> FreshSendAccount {
        FreshSendAccount {
            status: AccountStatus::Active,
            seqno: 7,
        }
    }

    fn prepared_transfer() -> PreparedTransfer {
        PreparedTransfer {
            operation_id: non_empty("operation"),
            record_id: non_empty("record"),
            source: source(),
            destination: destination(),
            amount: SendAmount::exact("1").expect("valid exact amount"),
            comment: None,
            seqno: 7,
            needs_state_init: false,
            valid_until: 1_800_000_300,
            signed_boc: Boc::try_from(TonCell::EMPTY_BOC.to_vec())
                .expect("the empty-cell BOC fixture is valid"),
            message_hash: Base64Hash::from_bytes(&[7; 32]).expect("the hash fixture has 32 bytes"),
        }
    }

    fn preparing_workflow() -> (SendWorkflow, PreparedTransfer) {
        let mut workflow = workflow();
        assert!(matches!(
            workflow.begin(),
            Ok(SendDirective::LoadJournal(_))
        ));
        assert_eq!(
            workflow.journal_loaded(None),
            Ok(SendDirective::FetchFreshAccount)
        );
        assert!(matches!(
            workflow.fresh_account_loaded(fresh_account()),
            Ok(SendDirective::ReadProtectedSecret(_))
        ));
        assert!(matches!(
            workflow.authorization_succeeded(),
            Ok(SendDirective::PrepareTransfer { .. })
        ));
        (workflow, prepared_transfer())
    }

    fn ready_to_submit_workflow() -> SendWorkflow {
        let (mut workflow, prepared) = preparing_workflow();
        assert!(matches!(
            workflow.transfer_prepared(prepared),
            Ok(SendDirective::PersistJournal(_))
        ));
        assert!(matches!(
            workflow.journal_persisted(&applied()),
            Ok(SendDirective::Submit { .. })
        ));
        workflow
    }

    fn durable_payload(stage: SendStage) -> Vec<u8> {
        durable_payload_with(|record| record.stage = stage)
    }

    fn durable_payload_with(mutate: impl FnOnce(&mut DurableSendRecord)) -> Vec<u8> {
        let prepared = prepared_transfer();
        let mut record = DurableSendRecord {
            schema_version: JOURNAL_SCHEMA_VERSION,
            operation_id: prepared.operation_id,
            record_id: prepared.record_id,
            source: prepared.source,
            destination: prepared.destination,
            amount: prepared.amount,
            comment: prepared.comment,
            seqno: prepared.seqno,
            needs_state_init: prepared.needs_state_init,
            valid_until: prepared.valid_until,
            signed_boc: prepared.signed_boc,
            message_hash: prepared.message_hash,
            stage: SendStage::Submitted,
            provider_reference: None,
            diagnostic: None,
            confirmed_transaction_hash: None,
            confirmed_transaction_lt: None,
        };
        mutate(&mut record);
        serde_json::to_vec(&record).expect("durable record fixture serializes")
    }

    fn durable_payload_with_json_field(field: &str, value: &str) -> Vec<u8> {
        let mut payload: serde_json::Value =
            serde_json::from_slice(&durable_payload(SendStage::Submitted))
                .expect("durable fixture is JSON");
        payload[field] = serde_json::Value::String(value.to_owned());
        serde_json::to_vec(&payload).expect("durable JSON fixture serializes")
    }

    fn non_empty(value: &str) -> NonEmptyString {
        NonEmptyString::try_from(value).expect("test string is non-empty")
    }

    fn applied() -> JournalCompareExchangeResult {
        JournalCompareExchangeResult {
            applied: true,
            current: None,
        }
    }
}
