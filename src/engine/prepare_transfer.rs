//! Preparation of alternative signed transfer delivery forms.

use crate::domain::{ProtectedSecretRead, SecretAccessReason, bounded_diagnostic};
use crate::transport::build_toncenter_v2_request;
use crate::wallet::send::FreshSendAccount;
use crate::wallet::transfer::{derive_source, prepare_transfer_pair};
use crate::{
    AccountStatus, PrepareTransferRequest, PreparedTransfer, SendRequest, WalletClientError,
};

use super::WalletClient;
use super::expiration::resolve_send_expiration;
use super::provider::parse_account;
use super::send_http::{build_seqno_request, parse_seqno};
use super::send_state::SensitiveBytes;
use super::state::{OperationFamily, ensure_running};

#[uniffi::export]
impl WalletClient {
    /// Signs external and internal delivery forms without submitting either one.
    ///
    /// The client fetches account state and `seqno` once, resolves one
    /// `valid_until`, and unlocks the protected phrase once. Both returned BOCs
    /// share a sequence number and are mutually exclusive delivery alternatives. This method does not read or write the send
    /// journal and does not submit either message.
    pub async fn prepare_transfer(
        &self,
        request: PrepareTransferRequest,
    ) -> Result<PreparedTransfer, WalletClientError> {
        let requested = request
            .intent
            .exact_value_total()
            .map_err(|_| WalletClientError::InvalidSendRequest)?;
        let gasless_intent = request.gasless_messages.map(|messages| crate::SendIntent {
            expiration: request.intent.expiration.clone(),
            messages,
        });
        let gasless_requested = gasless_intent
            .as_ref()
            .map(crate::SendIntent::exact_value_total)
            .transpose()
            .map_err(|_| WalletClientError::InvalidSendRequest)?;
        let operation_id = request.operation_id.clone();
        let send_request = SendRequest {
            operation_id: request.operation_id,
            force: false,
            intent: request.intent,
        };

        let (generation, config, expected_source, account_request, seqno_request, secret_request) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            let secret_ref = state
                .config
                .local_secret_ref
                .clone()
                .ok_or(WalletClientError::LocalSigningUnavailable)?;
            if state.active_send.is_some() || state.active_resolution.is_some() {
                return Err(WalletClientError::SendAlreadyInProgress);
            }

            state.resolution_generation = state
                .resolution_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.resolution_generation;
            let config = state.config.clone();
            let expected_source = config.address.clone();
            let account_request = build_toncenter_v2_request(
                &config,
                state.allocate_request_id()?,
                "getAddressInformation",
                &[("address", config.address.as_str())],
            )?;
            let seqno_request = build_seqno_request(&config, state.allocate_request_id()?)?;
            state.active_resolution = Some((generation, Vec::new()));

            (
                generation,
                config,
                expected_source,
                account_request,
                seqno_request,
                ProtectedSecretRead {
                    secret_ref,
                    reason: SecretAccessReason::SignTransfer,
                    prompt: "Authenticate to prepare this wallet transaction".to_owned(),
                },
            )
        };

        let account = match self
            .execute_tracked_standalone_resolution_request(generation, &account_request)
            .await
        {
            Ok(result) => result
                .and_then(|body| parse_account(&body))
                .map_err(|error| {
                    self.fail_transfer_preparation(generation, error.developer_message)
                })?,
            Err(error) => {
                self.discard_transfer_preparation(generation);
                return Err(error);
            }
        };

        let minimum_requested = match (requested, gasless_requested) {
            (Some(normal), Some(Some(gasless))) => Some(normal.min(gasless)),
            (normal, None) => normal,
            _ => None,
        };
        if let Some(requested_nanograms) = minimum_requested
            && requested_nanograms > account.balance_nanograms
        {
            let error = WalletClientError::InsufficientBalance {
                available_nanograms: account.balance_nanograms,
                requested_nanograms,
            };
            self.discard_transfer_preparation(generation);
            return Err(error);
        }

        let seqno = match account.status {
            AccountStatus::Active => match self
                .execute_tracked_standalone_resolution_request(generation, &seqno_request)
                .await
            {
                Ok(result) => result
                    .and_then(|body| parse_seqno(&body))
                    .map_err(|error| {
                        self.fail_transfer_preparation(generation, error.developer_message)
                    })?,
                Err(error) => {
                    self.discard_transfer_preparation(generation);
                    return Err(error);
                }
            },
            AccountStatus::Nonexistent | AccountStatus::Uninitialized => 0,
            status @ (AccountStatus::Frozen | AccountStatus::Unknown) => {
                self.discard_transfer_preparation(generation);
                return Err(WalletClientError::SendAccountUnavailable { status });
            }
        };
        let fresh = FreshSendAccount {
            status: account.status,
            seqno,
        };
        let valid_until = resolve_send_expiration(
            &send_request.intent.expiration,
            account.sync_utime,
            config.send_validity_seconds,
        )
        .map_err(|error| self.fail_transfer_preparation(generation, error.to_string()))?;

        let secret = SensitiveBytes::new(
            self.platform_host
                .read_protected_secret(secret_request)
                .await
                .map_err(|error| self.fail_transfer_preparation(generation, error.to_string()))?,
        );
        self.ensure_transfer_preparation_current(generation)?;

        let source = derive_source(secret.as_slice(), config.network).map_err(|_| {
            self.discard_transfer_preparation(generation);
            WalletClientError::InvalidProtectedSecret
        })?;
        if &source != expected_source.as_address() {
            return Err(self.fail_transfer_preparation(
                generation,
                "protected mnemonic does not belong to this wallet",
            ));
        }

        let (external, internal) = prepare_transfer_pair(
            secret.as_slice(),
            &config.record_id,
            &expected_source,
            config.network,
            &send_request,
            gasless_intent,
            &fresh,
            valid_until,
        )
        .map_err(|error| {
            self.fail_transfer_preparation(
                generation,
                format!("failed to prepare transfer messages: {error}"),
            )
        })?;

        self.complete_transfer_preparation(generation)?;
        Ok(PreparedTransfer {
            operation_id,
            external_boc: external.signed_boc,
            internal_boc: internal.signed_boc,
            seqno,
            valid_until,
        })
    }
}

impl WalletClient {
    fn ensure_transfer_preparation_current(
        &self,
        generation: u64,
    ) -> Result<(), WalletClientError> {
        let state = self.lock()?;
        if state.is_current(OperationFamily::Resolution, generation) {
            Ok(())
        } else {
            Err(WalletClientError::StateUnavailable)
        }
    }

    fn complete_transfer_preparation(&self, generation: u64) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Resolution, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.active_resolution = None;
        Ok(())
    }

    fn discard_transfer_preparation(&self, generation: u64) {
        if let Ok(mut state) = self.lock()
            && state.is_current(OperationFamily::Resolution, generation)
        {
            state.active_resolution = None;
        }
    }

    fn fail_transfer_preparation(
        &self,
        generation: u64,
        message: impl Into<String>,
    ) -> WalletClientError {
        self.discard_transfer_preparation(generation);
        WalletClientError::SendFailed {
            diagnostic: bounded_diagnostic(message.into()),
        }
    }
}
