//! Pre-authorization transfer previews built from fresh provider state.

use crate::domain::bounded_diagnostic;
use crate::transport::build_toncenter_v2_request;
use crate::wallet::send::FreshSendAccount;
use crate::wallet::transfer::{prepare_signed_boc, prepare_transfer_emulation};
use crate::{
    AccountStatus, Boc, DomainError, HttpRequest, HttpRequestId, SendBocRequest, SendEmulation,
    SendPreview, SendPreviewRequest, SendRequest, SignMessagePreview, TonAddressString,
    WalletClientConfig, WalletClientError,
};

use super::WalletClient;
use super::emulation::{build_emulation_request, is_message_not_accepted, parse_emulation};
use super::expiration::resolve_send_expiration;
use super::provider::parse_account;
use super::send_http::{build_seqno_request, parse_seqno};
use super::state::{OperationFamily, ensure_running};

#[uniffi::export]
impl WalletClient {
    /// Emulates a transfer without reading the journal or protected secret.
    ///
    /// The engine fetches fresh account state and seqno, builds a complete
    /// Wallet message with a fake signature, and asks Toncenter to execute it.
    /// Calling [`Self::send`] later repeats the chain-state checks and builds a
    /// new message from fresh state before the real signature is created.
    pub async fn preview_send(
        &self,
        request: SendPreviewRequest,
    ) -> Result<SendPreview, WalletClientError> {
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
        let requested = request
            .intent
            .exact_value_total()
            .map_err(|_| self.preview_error(generation, WalletClientError::InvalidSendRequest))?;

        if let Some(nanograms) = &requested
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
        let valid_until = resolve_send_expiration(
            &request.intent.expiration,
            provider_time,
            config.send_validity_seconds,
        )
        .map_err(|error| {
            self.preview_error(
                generation,
                WalletClientError::SendPreviewFailed {
                    diagnostic: error.to_string(),
                },
            )
        })?;

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

        let emulation = self
            .emulate_preview_boc(
                generation,
                &config,
                &expected_source,
                emulation_request_id,
                &boc,
            )
            .await?;

        if let Some(nanograms) = &requested {
            // Exact sends must leave a positive remainder after the emulated
            // wallet fee. Use `SendAmount::All` when the intent is to drain
            // the wallet with carry-all-balance mode instead.
            #[allow(
                clippy::arithmetic_side_effects,
                reason = "UnsignedDecimalString uses arbitrary-precision BigUint addition"
            )]
            let required = nanograms + &emulation.wallet_fees_nanograms;
            if required >= available {
                return Err(self.preview_error(
                    generation,
                    WalletClientError::InsufficientBalanceForFees {
                        available_nanograms: available.clone(),
                        requested_nanograms: nanograms.clone(),
                        estimated_fee_nanograms: emulation.wallet_fees_nanograms,
                    },
                ));
            }
        }

        let preview = SendPreview {
            messages: request.intent.messages,
            valid_until,
            message_boc_base64: boc,
            emulation,
        };
        self.finish_preview(generation)?;
        Ok(preview)
    }

    /// Emulates an already signed external-message BOC without submitting it.
    ///
    /// The request is validated against the configured source and fresh wallet
    /// seqno and provider time. The exact BOC is sent only to the emulation
    /// endpoint; this operation does not read or write the journal and does not
    /// publish the message to the network. The operation identifier and force
    /// flag are ignored, so the same request can be reused with [`Self::send_boc`].
    pub async fn preview_send_boc(
        &self,
        request: SendBocRequest,
    ) -> Result<SendPreview, WalletClientError> {
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

        let prepared =
            prepare_signed_boc(&config.record_id, &expected_source, &request).map_err(|error| {
                self.preview_error(
                    generation,
                    WalletClientError::SendPreviewFailed {
                        diagnostic: bounded_diagnostic(format!("invalid prepared BOC: {error}")),
                    },
                )
            })?;

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
        if seqno != request.seqno {
            return Err(self.preview_error(
                generation,
                WalletClientError::SendPreviewFailed {
                    diagnostic: format!(
                        "prepared BOC seqno {} does not match current wallet seqno {seqno}",
                        request.seqno
                    ),
                },
            ));
        }

        let valid_until = resolve_send_expiration(
            &crate::SendExpiration::Exact {
                unix_timestamp: request.valid_until,
            },
            account.sync_utime,
            config.send_validity_seconds,
        )
        .map_err(|error| {
            self.preview_error(
                generation,
                WalletClientError::SendPreviewFailed {
                    diagnostic: error.to_string(),
                },
            )
        })?;

        let emulation = self
            .emulate_preview_boc(
                generation,
                &config,
                &expected_source,
                emulation_request_id,
                &prepared.signed_boc,
            )
            .await?;

        let preview = SendPreview {
            messages: prepared.messages,
            valid_until,
            message_boc_base64: prepared.signed_boc,
            emulation,
        };
        self.finish_preview(generation)?;
        Ok(preview)
    }

    /// Emulates the exact transfer fields supplied by a TON Connect request.
    ///
    /// The preview preserves the dApp validity boundary, payload, and
    /// destination `StateInit`. It does not consume the operation identifier.
    pub async fn preview_ton_connect(
        &self,
        request: SendRequest,
    ) -> Result<SendPreview, WalletClientError> {
        self.preview_send(SendPreviewRequest {
            intent: request.intent,
        })
        .await
    }

    /// Validates an internal-message signing request from fresh public state.
    ///
    /// No wallet-paid fee estimate is returned because a relayer supplies the
    /// TON attached to the internal request. Signing repeats these checks and
    /// reads the protected secret only after host approval.
    pub async fn preview_sign_message(
        &self,
        request: SendPreviewRequest,
    ) -> Result<SignMessagePreview, WalletClientError> {
        let (generation, config, account_request) = {
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
            let account_request = build_toncenter_v2_request(
                &config,
                state.allocate_request_id()?,
                "getAddressInformation",
                &[("address", config.address.as_str())],
            )?;
            state.active_preview = Some((generation, Vec::new()));
            (generation, config, account_request)
        };

        let _ = request
            .intent
            .exact_value_total()
            .map_err(|_| self.preview_error(generation, WalletClientError::InvalidSendRequest))?;
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
        if matches!(
            account.status,
            AccountStatus::Frozen | AccountStatus::Unknown
        ) {
            return Err(self.preview_error(
                generation,
                WalletClientError::SendAccountUnavailable {
                    status: account.status,
                },
            ));
        }
        let valid_until = resolve_send_expiration(
            &request.intent.expiration,
            account.sync_utime,
            config.send_validity_seconds,
        )
        .map_err(|error| {
            self.preview_error(
                generation,
                WalletClientError::SendPreviewFailed {
                    diagnostic: error.to_string(),
                },
            )
        })?;
        let preview = SignMessagePreview {
            messages: request.intent.messages,
            valid_until,
            needs_state_init: !matches!(account.status, AccountStatus::Active),
        };
        self.finish_preview(generation)?;
        Ok(preview)
    }

    /// Cancels the current send preview and its active provider request.
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
            self.transport.cancel(request_id).await;
        }

        Ok(())
    }
}

impl WalletClient {
    async fn emulate_preview_boc(
        &self,
        generation: u64,
        config: &WalletClientConfig,
        expected_source: &TonAddressString,
        emulation_request_id: HttpRequestId,
        boc: &Boc,
    ) -> Result<SendEmulation, WalletClientError> {
        let emulation_request = build_emulation_request(config, emulation_request_id, boc)
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
            .and_then(|body| parse_emulation(&body, expected_source))
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

        Ok(evaluated.summary)
    }

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

    fn start_preview_request(
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

    /// Executes a preview-owned provider request and returns its checked body.
    ///
    /// Tracking is finished after the transport completes and before its result
    /// is propagated, including for rejected responses.
    async fn execute_tracked_preview_request(
        &self,
        generation: u64,
        request: &HttpRequest,
    ) -> Result<Result<Vec<u8>, DomainError>, WalletClientError> {
        self.start_preview_request(generation, request.id)?;
        let result = self.transport.execute(request).await;
        self.finish_preview_request(generation, request.id)?;
        Ok(result)
    }

    fn finish_preview_request(
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
