//! Pre-authorization transfer previews built from fresh provider state.

use crate::domain::bounded_diagnostic;
use crate::types::{parse_canonical_decimal, parse_positive_decimal};
use crate::wallet::send::FreshSendAccount;
use crate::wallet::transfer::prepare_transfer_emulation;
use crate::{
    AccountStatus, HttpHostError, HttpRequest, HttpRequestId, HttpResponse, SendAmount,
    SendPreview, SendPreviewRequest, WalletClientError,
};

use super::WalletClient;
use super::emulation::{build_emulation_request, is_message_not_accepted, parse_emulation};
use super::http::{build_toncenter_request, evaluate_response};
use super::provider::parse_account;
use super::send_http::{build_seqno_request, parse_seqno};
use super::state::{OperationFamily, ensure_running};
use super::validation::validate_send_preview;

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
        validate_send_preview(&request)?;

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
            let expected_source = config.parsed_address()?;
            let account_request = build_toncenter_request(
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

        let account_response = self
            .execute_tracked_preview_request(generation, &account_request)
            .await?;
        let account = evaluate_response(&account_request, account_response, parse_account)
            .map_err(|error| {
                self.preview_error(
                    generation,
                    WalletClientError::SendFailed {
                        diagnostic: bounded_diagnostic(error.developer_message),
                    },
                )
            })?;

        let available = parse_positive_decimal(&account.balance_nanograms).ok_or_else(|| {
            self.preview_error(
                generation,
                WalletClientError::SendFailed {
                    diagnostic: "invalid fresh account balance".to_owned(),
                },
            )
        })?;
        if let SendAmount::Exact { nanograms } = &request.amount {
            let requested = parse_positive_decimal(nanograms).ok_or_else(|| {
                self.preview_error(generation, WalletClientError::InvalidSendRequest)
            })?;
            if requested > available {
                return Err(self.preview_error(
                    generation,
                    WalletClientError::InsufficientBalance {
                        available_nanograms: available.to_string(),
                        requested_nanograms: requested.to_string(),
                    },
                ));
            }
        }

        let seqno = match account.status {
            AccountStatus::Active => {
                let response = self
                    .execute_tracked_preview_request(generation, &seqno_request)
                    .await?;
                evaluate_response(&seqno_request, response, parse_seqno).map_err(|error| {
                    self.preview_error(
                        generation,
                        WalletClientError::SendFailed {
                            diagnostic: bounded_diagnostic(error.developer_message),
                        },
                    )
                })?
            }
            AccountStatus::Nonexistent | AccountStatus::Uninitialized => 0,
            status @ (AccountStatus::Frozen | AccountStatus::Unknown) => {
                return Err(self.preview_error(
                    generation,
                    WalletClientError::SendAccountUnavailable { status },
                ));
            }
        };

        let provider_time = account
            .sync_utime
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                self.preview_error(
                    generation,
                    WalletClientError::SendFailed {
                        diagnostic: "fresh account state has no valid synchronization time"
                            .to_owned(),
                    },
                )
            })?;
        let valid_until = provider_time
            .checked_add(config.send_validity_seconds)
            .ok_or_else(|| {
                self.preview_error(
                    generation,
                    WalletClientError::SendFailed {
                        diagnostic: "transfer expiration timestamp overflow".to_owned(),
                    },
                )
            })?;
        let fresh = FreshSendAccount {
            status: account.status,
            seqno,
            observed_at: account.sync_utime.unwrap_or_default(),
        };

        let boc = prepare_transfer_emulation(
            &expected_source,
            &config.public_key,
            config.network,
            &request.destination,
            &request.amount,
            &fresh,
            valid_until,
        )
        .map_err(|error| {
            self.preview_error(
                generation,
                WalletClientError::EmulationFailed {
                    diagnostic: bounded_diagnostic(format!("failed to prepare emulation: {error}")),
                },
            )
        })?;
        let message_boc_base64 = boc.to_base64();
        let emulation_request =
            build_emulation_request(&config, emulation_request_id, boc.as_bytes()).map_err(
                |error| {
                    self.preview_error(
                        generation,
                        WalletClientError::EmulationFailed {
                            diagnostic: bounded_diagnostic(error.to_string()),
                        },
                    )
                },
            )?;
        let emulation_response = self
            .execute_tracked_preview_request(generation, &emulation_request)
            .await?;
        let evaluated = evaluate_response(&emulation_request, emulation_response, |body| {
            parse_emulation(body, &expected_source)
        })
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
            let requested = parse_positive_decimal(nanograms).ok_or_else(|| {
                self.preview_error(generation, WalletClientError::InvalidSendRequest)
            })?;
            let estimated_fee = parse_canonical_decimal(&evaluated.summary.wallet_fees_nanograms)
                .ok_or_else(|| {
                self.preview_error(
                    generation,
                    WalletClientError::EmulationFailed {
                        diagnostic: "emulation returned an invalid wallet fee".to_owned(),
                    },
                )
            })?;
            // Exact sends must leave a positive remainder after the emulated
            // wallet fee. Use `SendAmount::All` when the intent is to drain
            // the wallet with carry-all-balance mode instead.
            if &requested + &estimated_fee >= available {
                return Err(self.preview_error(
                    generation,
                    WalletClientError::InsufficientBalanceForFees {
                        available_nanograms: available.to_string(),
                        requested_nanograms: requested.to_string(),
                        estimated_fee_nanograms: estimated_fee.to_string(),
                    },
                ));
            }
        }

        let preview = SendPreview {
            destination: request.destination,
            amount: request.amount,
            valid_until,
            message_boc_base64,
            emulation: evaluated.summary,
        };
        self.finish_preview(generation)?;
        Ok(preview)
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

    async fn execute_tracked_preview_request(
        &self,
        generation: u64,
        request: &HttpRequest,
    ) -> Result<Result<HttpResponse, HttpHostError>, WalletClientError> {
        self.start_preview_http_request(generation, request.id)?;
        let result = self.http_host.execute_http(request.clone()).await;
        self.finish_preview_http_request(generation, request.id)?;
        Ok(result)
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
