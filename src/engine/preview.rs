//! Pre-authorization transfer previews built from fresh provider state.

use crate::domain::bounded_diagnostic;
use crate::wallet::send::FreshSendAccount;
use crate::wallet::transfer::prepare_transfer_emulation;
use crate::{
    AccountStatus, DomainError, HttpRequest, HttpRequestId, SendAmount, SendPreview,
    SendPreviewRequest, SendRequest, WalletClientError,
};

use super::WalletClient;
use super::emulation::{build_emulation_request, is_message_not_accepted, parse_emulation};
use super::http::{build_toncenter_v2_request, process_response};
use super::provider::parse_account;
use super::send_http::{build_seqno_request, parse_seqno};
use super::state::{OperationFamily, ensure_running};

#[uniffi::export]
impl WalletClient {
    /// Emulates a transfer without reading the journal or protected secret.
    ///
    /// The engine fetches fresh account state and seqno, builds a complete
    /// V5R1 message with a fake signature, and asks Toncenter to execute it.
    /// Calling [`Self::send`] later repeats the chain-state checks and builds a
    /// new message from fresh state before the real signature is created.
    pub async fn preview_send(
        &self,
        request: SendPreviewRequest,
    ) -> Result<SendPreview, WalletClientError> {
        if request.payload.is_some() && request.comment.is_some() {
            return Err(WalletClientError::InvalidSendRequest);
        }
        let (
            generation,
            config,
            expected_source,
            account_request,
            seqno_request,
            emulation_request_id,
        ) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;

            if state.active_send.is_some() {
                return Err(WalletClientError::SendAlreadyInProgress);
            }
            if state.active_preview.is_some() {
                return Err(WalletClientError::SendPreviewAlreadyInProgress);
            }

            state.preview_generation = state
                .preview_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.preview_generation;
            let config = state.config.clone();
            let expected_source = config.address.clone();
            let account_request = build_toncenter_v2_request(
                &config,
                state.allocate_request_id()?,
                "getAddressInformation",
                &[("address", config.address.as_str())],
            )?;
            let seqno_request = build_seqno_request(&config, state.allocate_request_id()?)?;
            let emulation_request_id = state.allocate_request_id()?;

            state.active_preview = Some((generation, Vec::new()));

            (
                generation,
                config,
                expected_source,
                account_request,
                seqno_request,
                emulation_request_id,
            )
        };

        let account = self
            .execute_tracked_preview_request(generation, &account_request)
            .await?
            .and_then(|body| parse_account(&body))
            .map_err(|error| {
                self.preview_error(
                    generation,
                    WalletClientError::SendPreviewFailed {
                        diagnostic: bounded_diagnostic(error.developer_message),
                    },
                )
            })?;

        let available = account.balance_nanograms.clone();

        if let SendAmount::Exact { nanograms } = &request.amount
            && nanograms > &available
        {
            return Err(self.preview_error(
                generation,
                WalletClientError::InsufficientBalance {
                    available_nanograms: available.clone(),
                    requested_nanograms: nanograms.clone(),
                },
            ));
        }

        let seqno = match account.status {
            AccountStatus::Active => self
                .execute_tracked_preview_request(generation, &seqno_request)
                .await?
                .and_then(|body| parse_seqno(&body))
                .map_err(|error| {
                    self.preview_error(
                        generation,
                        WalletClientError::SendPreviewFailed {
                            diagnostic: bounded_diagnostic(error.developer_message),
                        },
                    )
                })?,
            AccountStatus::Nonexistent | AccountStatus::Uninitialized => 0,
            status @ (AccountStatus::Frozen | AccountStatus::Unknown) => {
                return Err(self.preview_error(
                    generation,
                    WalletClientError::SendAccountUnavailable { status },
                ));
            }
        };

        let provider_time = account.sync_utime;
        let valid_until = if let Some(valid_until) = request.valid_until {
            if valid_until <= provider_time {
                return Err(self.preview_error(
                    generation,
                    WalletClientError::SendPreviewFailed {
                        diagnostic:
                            "transfer expiration timestamp is not after fresh provider time"
                                .to_owned(),
                    },
                ));
            }
            valid_until
        } else {
            provider_time
                .checked_add(config.send_validity_seconds)
                .ok_or_else(|| {
                    self.preview_error(
                        generation,
                        WalletClientError::SendPreviewFailed {
                            diagnostic: "transfer expiration timestamp overflow".to_owned(),
                        },
                    )
                })?
        };

        let fresh = FreshSendAccount {
            status: account.status,
            seqno,
        };

        let boc = prepare_transfer_emulation(
            &expected_source,
            &config.public_key,
            config.network,
            &request,
            &fresh,
            valid_until,
        )
        .map_err(|error| {
            self.preview_error(
                generation,
                WalletClientError::SendPreviewFailed {
                    diagnostic: bounded_diagnostic(format!("failed to prepare preview: {error}")),
                },
            )
        })?;

        let emulation_request = build_emulation_request(&config, emulation_request_id, &boc)
            .map_err(|error| {
                self.preview_error(
                    generation,
                    WalletClientError::SendPreviewFailed {
                        diagnostic: bounded_diagnostic(error.to_string()),
                    },
                )
            })?;

        let evaluated = self
            .execute_tracked_preview_request(generation, &emulation_request)
            .await?
            .and_then(|body| parse_emulation(&body, &expected_source))
            .map_err(|error| {
                let not_accepted = is_message_not_accepted(&error);
                let diagnostic = bounded_diagnostic(error.developer_message);
                let public = if not_accepted {
                    WalletClientError::EmulationMessageNotAccepted { diagnostic }
                } else {
                    WalletClientError::EmulationFailed { diagnostic }
                };
                self.preview_error(generation, public)
            })?;

        if !evaluated.wallet_succeeded {
            return Err(self.preview_error(
                generation,
                WalletClientError::EmulationRejected {
                    diagnostic: "emulated transfer did not complete successfully".to_owned(),
                    compute_exit_code: evaluated.compute_exit_code,
                    action_result_code: evaluated.action_result_code,
                },
            ));
        }

        if let SendAmount::Exact { nanograms } = &request.amount {
            // Exact sends must leave a positive remainder after the emulated
            // wallet fee. Use `SendAmount::All` when the intent is to drain
            // the wallet with carry-all-balance mode instead.
            #[allow(
                clippy::arithmetic_side_effects,
                reason = "UnsignedDecimalString uses arbitrary-precision BigUint addition"
            )]
            let required = nanograms + &evaluated.summary.wallet_fees_nanograms;
            if required >= available {
                return Err(self.preview_error(
                    generation,
                    WalletClientError::InsufficientBalanceForFees {
                        available_nanograms: available.clone(),
                        requested_nanograms: nanograms.clone(),
                        estimated_fee_nanograms: evaluated.summary.wallet_fees_nanograms,
                    },
                ));
            }
        }

        let preview = SendPreview {
            destination: request.destination,
            amount: request.amount,
            comment: request.comment,
            valid_until,
            message_boc_base64: boc,
            emulation: evaluated.summary,
        };
        self.finish_preview(generation)?;
        Ok(preview)
    }

    /// Emulates the exact transfer fields supplied by a TON Connect request.
    ///
    /// This reuses the regular preview pipeline while preserving the dApp's
    /// validity boundary, payload, and destination `StateInit` byte-for-byte.
    pub async fn preview_ton_connect(
        &self,
        request: SendRequest,
    ) -> Result<SendPreview, WalletClientError> {
        self.preview_send(SendPreviewRequest {
            destination: request.destination,
            amount: request.amount,
            valid_until: request.valid_until,
            payload: request.payload,
            state_init: request.state_init,
            comment: request.comment,
        })
        .await
    }

    /// Cancels the current send preview and its active HTTP request.
    pub async fn cancel_send_preview(&self) -> Result<(), WalletClientError> {
        let request_ids = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            state
                .active_preview
                .take()
                .map_or_else(Vec::new, |active| active.1)
        };

        for request_id in request_ids {
            self.http_host.cancel_http(request_id).await;
        }

        Ok(())
    }
}

impl WalletClient {
    fn preview_error(&self, generation: u64, error: WalletClientError) -> WalletClientError {
        match self.finish_preview(generation) {
            Ok(()) => error,
            Err(state_error) => state_error,
        }
    }

    fn finish_preview(&self, generation: u64) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if state.is_current(OperationFamily::Preview, generation) {
            state.active_preview = None;
            Ok(())
        } else if state.shutdown {
            Err(WalletClientError::Shutdown)
        } else {
            Err(WalletClientError::StateUnavailable)
        }
    }

    fn start_preview_http_request(
        &self,
        generation: u64,
        request_id: HttpRequestId,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        ensure_running(&state)?;
        let Some((active_generation, request_ids)) = state.active_preview.as_mut() else {
            return Err(WalletClientError::StateUnavailable);
        };
        if *active_generation != generation {
            return Err(WalletClientError::StateUnavailable);
        }
        request_ids.push(request_id);
        Ok(())
    }

    /// Executes a preview-owned HTTP request and returns its engine-checked body.
    ///
    /// Tracking is finished before response processing so every completed host
    /// callback releases its request ID, including rejected responses.
    async fn execute_tracked_preview_request(
        &self,
        generation: u64,
        request: &HttpRequest,
    ) -> Result<Result<Vec<u8>, DomainError>, WalletClientError> {
        self.start_preview_http_request(generation, request.id)?;
        let result = self.http_host.execute_http(request.clone()).await;
        self.finish_preview_http_request(generation, request.id)?;
        Ok(process_response(request, result))
    }

    fn finish_preview_http_request(
        &self,
        generation: u64,
        request_id: HttpRequestId,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        ensure_running(&state)?;
        let Some((active_generation, request_ids)) = state.active_preview.as_mut() else {
            return Err(WalletClientError::StateUnavailable);
        };
        if *active_generation != generation {
            return Err(WalletClientError::StateUnavailable);
        }
        request_ids.retain(|active| *active != request_id);
        Ok(())
    }
}
