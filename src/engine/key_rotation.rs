//! Wallet key-rotation preparation backed by fresh provider state.

use crate::domain::{ProtectedSecretRead, SecretAccessReason, bounded_diagnostic};
use crate::wallet::crypto::SensitiveMnemonic;
use crate::wallet::key_rotation::{KeyRotationError, prepare_key_rotation as prepare_rotation};
use crate::{PrepareKeyRotationRequest, PreparedKeyRotation, RecoveryPhrase, WalletClientError};

use super::WalletClient;
use super::send_http::{build_seqno_request, parse_seqno};
use super::state::{OperationFamily, ensure_running};

#[uniffi::export]
impl WalletClient {
    /// Generates a new signing half and a signed Wallet rev00 key-change message.
    ///
    /// The client fetches the wallet's current `seqno` through its configured
    /// provider before asking the host to unlock the protected phrase. It does
    /// not update protected storage and does not submit the returned BOC.
    pub async fn prepare_key_rotation(
        &self,
        request: PrepareKeyRotationRequest,
    ) -> Result<PreparedKeyRotation, WalletClientError> {
        if u32::try_from(request.valid_until).is_err() {
            return Err(key_rotation_error(
                "the expiration timestamp exceeds uint32",
            ));
        }

        let (generation, config, seqno_request, secret_request) = {
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
            let seqno_request = build_seqno_request(&config, state.allocate_request_id()?)?;
            state.active_resolution = Some((generation, Vec::new()));
            (
                generation,
                config,
                seqno_request,
                ProtectedSecretRead {
                    secret_ref,
                    reason: SecretAccessReason::PrepareKeyRotation,
                    prompt: "Authenticate to create a new wallet signing key".to_owned(),
                },
            )
        };

        let seqno = match self
            .execute_tracked_standalone_resolution_request(generation, &seqno_request)
            .await
        {
            Ok(result) => result
                .and_then(|body| parse_seqno(&body))
                .map_err(|error| self.fail_key_rotation(generation, error.developer_message))?,
            Err(error) => {
                self.discard_key_rotation_operation(generation);
                return Err(error);
            }
        };

        let bytes = self
            .platform_host
            .read_protected_secret(secret_request)
            .await
            .map_err(|error| self.fail_key_rotation(generation, error.to_string()))?;
        self.ensure_key_rotation_current(generation)?;
        let secret = SensitiveMnemonic::from_bytes(bytes).map_err(|_| {
            self.discard_key_rotation_operation(generation);
            WalletClientError::InvalidProtectedSecret
        })?;

        let prepared = prepare_rotation(
            &secret,
            config.network,
            &config.address,
            seqno,
            request.valid_until,
            request.message_kind,
        )
        .map_err(|error| match error {
            KeyRotationError::InvalidMnemonic => {
                self.discard_key_rotation_operation(generation);
                WalletClientError::InvalidProtectedSecret
            }
            KeyRotationError::WalletIdentityMismatch => self.fail_key_rotation(
                generation,
                "the protected mnemonic does not belong to this wallet",
            ),
            KeyRotationError::ExpirationOutOfRange => {
                self.fail_key_rotation(generation, "the expiration timestamp exceeds uint32")
            }
            KeyRotationError::Preparation => self.fail_key_rotation(
                generation,
                "failed to generate, sign, or serialize key-rotation data",
            ),
        })?;
        let replacement_phrase = prepared
            .replacement_mnemonic
            .as_str()
            .map_err(|_| {
                self.fail_key_rotation(generation, "generated recovery phrase is invalid")
            })?
            .to_owned();

        self.complete_key_rotation_operation(generation)?;
        Ok(PreparedKeyRotation {
            replacement_recovery_phrase: RecoveryPhrase {
                phrase: replacement_phrase,
            },
            new_public_key: prepared.new_public_key.to_vec(),
            signed_boc: prepared.signed_boc,
            seqno,
            valid_until: request.valid_until,
            message_kind: request.message_kind,
        })
    }
}

impl WalletClient {
    fn ensure_key_rotation_current(&self, generation: u64) -> Result<(), WalletClientError> {
        let state = self.lock()?;
        if state.is_current(OperationFamily::Resolution, generation) {
            Ok(())
        } else {
            Err(WalletClientError::StateUnavailable)
        }
    }

    fn complete_key_rotation_operation(&self, generation: u64) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Resolution, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.active_resolution = None;
        Ok(())
    }

    fn discard_key_rotation_operation(&self, generation: u64) {
        if let Ok(mut state) = self.lock()
            && state.is_current(OperationFamily::Resolution, generation)
        {
            state.active_resolution = None;
        }
    }

    fn fail_key_rotation(&self, generation: u64, message: impl AsRef<str>) -> WalletClientError {
        self.discard_key_rotation_operation(generation);
        key_rotation_error(message)
    }
}

fn key_rotation_error(message: impl AsRef<str>) -> WalletClientError {
    WalletClientError::KeyRotationUnavailable {
        diagnostic: bounded_diagnostic(message),
    }
}
