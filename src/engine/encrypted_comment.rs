//! Explicit encrypted-comment preparation and decryption workflows.

use crate::domain::{SecretAccessReason, bounded_diagnostic};
use crate::wallet::encrypted_comment::{
    EncryptedCommentError, MAX_ENCRYPTED_COMMENT_BYTES, decrypt_comment as decrypt_body,
    encrypt_comment as encrypt_body, validate_encrypted_comment_body,
};
use crate::{
    Boc, CreateEncryptedCommentRequest, DecryptCommentRequest, ProtectedSecretRead,
    WalletClientError,
};

use super::WalletClient;
use super::send_http::{build_public_key_request, parse_public_key};
use super::send_state::SensitiveBytes;
use super::state::{OperationFamily, ensure_running};

#[uniffi::export]
impl WalletClient {
    /// Creates a TON encrypted-comment body ready for `SendMessageBody::RawPayload`.
    ///
    /// The engine calls the recipient wallet's `get_public_key` get-method, then
    /// asks the platform host to authorize this wallet's protected mnemonic.
    /// No secret is requested when the comment is already too large.
    pub async fn create_encrypted_comment(
        &self,
        request: CreateEncryptedCommentRequest,
    ) -> Result<Boc, WalletClientError> {
        if request.comment.len() > MAX_ENCRYPTED_COMMENT_BYTES {
            return Err(encrypted_comment_error(
                "the encrypted comment exceeds 960 UTF-8 bytes",
            ));
        }

        let (generation, config, public_key_request, secret_request) = {
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
            let public_key_request = build_public_key_request(
                &config,
                state.allocate_request_id()?,
                &request.recipient,
            )?;
            state.active_resolution = Some((generation, Vec::new()));
            (
                generation,
                config,
                public_key_request,
                ProtectedSecretRead {
                    secret_ref,
                    reason: SecretAccessReason::EncryptComment,
                    prompt: "Authenticate to encrypt this transfer comment".to_owned(),
                },
            )
        };

        let recipient_public_key = match self
            .execute_tracked_standalone_resolution_request(generation, &public_key_request)
            .await
        {
            Ok(result) => result
                .and_then(|body| parse_public_key(&body))
                .map_err(|error| {
                    self.fail_encrypted_comment(generation, error.developer_message)
                })?,
            Err(error) => {
                self.discard_encrypted_comment_operation(generation);
                return Err(error);
            }
        };

        let secret = SensitiveBytes::new(
            self.platform_host
                .read_protected_secret(secret_request)
                .await
                .map_err(|error| self.fail_encrypted_comment(generation, error.to_string()))?,
        );
        self.ensure_encrypted_comment_current(generation)?;

        let body = match encrypt_body(
            secret.as_slice(),
            config.network,
            &config.address,
            &recipient_public_key,
            &request.comment,
        ) {
            Ok(body) => body,
            Err(EncryptedCommentError::InvalidMnemonic) => {
                return Err(self.finish_invalid_protected_secret(generation));
            }
            Err(error) => return Err(self.fail_encrypted_comment(generation, error.to_string())),
        };
        self.complete_encrypted_comment_operation(generation)?;
        Ok(body)
    }

    /// Decrypts one TON encrypted-comment body after explicit host authorization.
    ///
    /// The caller supplies the sender address because TON uses its bounceable,
    /// URL-safe, non-test-only representation as authenticated salt.
    pub async fn decrypt_comment(
        &self,
        request: DecryptCommentRequest,
    ) -> Result<String, WalletClientError> {
        validate_encrypted_comment_body(&request.body)
            .map_err(|error| encrypted_comment_error(error.to_string()))?;

        let (generation, config, secret_request) = {
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
            state.active_resolution = Some((generation, Vec::new()));
            (
                generation,
                config,
                ProtectedSecretRead {
                    secret_ref,
                    reason: SecretAccessReason::DecryptComment,
                    prompt: "Authenticate to decrypt this transfer comment".to_owned(),
                },
            )
        };

        let secret = SensitiveBytes::new(
            self.platform_host
                .read_protected_secret(secret_request)
                .await
                .map_err(|error| self.fail_encrypted_comment(generation, error.to_string()))?,
        );
        self.ensure_encrypted_comment_current(generation)?;
        let comment = match decrypt_body(
            secret.as_slice(),
            config.network,
            &config.address,
            &request.sender,
            &request.body,
        ) {
            Ok(comment) => comment,
            Err(EncryptedCommentError::InvalidMnemonic) => {
                return Err(self.finish_invalid_protected_secret(generation));
            }
            Err(error) => return Err(self.fail_encrypted_comment(generation, error.to_string())),
        };
        self.complete_encrypted_comment_operation(generation)?;
        Ok(comment)
    }
}

impl WalletClient {
    fn ensure_encrypted_comment_current(&self, generation: u64) -> Result<(), WalletClientError> {
        let state = self.lock()?;
        if !state.is_current(OperationFamily::Resolution, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        Ok(())
    }

    fn complete_encrypted_comment_operation(
        &self,
        generation: u64,
    ) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Resolution, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.active_resolution = None;
        Ok(())
    }

    fn discard_encrypted_comment_operation(&self, generation: u64) {
        if let Ok(mut state) = self.lock()
            && state.is_current(OperationFamily::Resolution, generation)
        {
            state.active_resolution = None;
        }
    }

    fn fail_encrypted_comment(
        &self,
        generation: u64,
        message: impl AsRef<str>,
    ) -> WalletClientError {
        self.discard_encrypted_comment_operation(generation);
        encrypted_comment_error(message)
    }

    fn finish_invalid_protected_secret(&self, generation: u64) -> WalletClientError {
        match self.complete_encrypted_comment_operation(generation) {
            Ok(()) => WalletClientError::InvalidProtectedSecret,
            Err(error) => error,
        }
    }
}

fn encrypted_comment_error(message: impl AsRef<str>) -> WalletClientError {
    WalletClientError::EncryptedCommentUnavailable {
        diagnostic: bounded_diagnostic(message),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ed25519_dalek::SigningKey;
    use futures::executor::block_on;

    use super::*;
    use crate::wallet::crypto::derive_wallet;
    use crate::{
        HttpHostError, HttpRequest, HttpRequestId, HttpResponse, JournalCompareExchange,
        JournalCompareExchangeResult, JournalHostError, JournalKey, JournalRecord, Network,
        NonEmptyString, ProtectedSecretHostError, ProtectedSecretRef, ProtectedSecretStore,
        ProviderConfig, TonAddressString, WalletClientConfig, WalletHttpHost, WalletPlatformHost,
    };

    const MNEMONIC: &str = "section garden tomato dinner season dice renew length useful spin trade intact use universe what post spike keen mandate behind concert egg doll rug";
    const RECIPIENT: &str = "0:2222222222222222222222222222222222222222222222222222222222222222";

    struct PublicKeyHost {
        public_key: [u8; 32],
    }

    #[async_trait::async_trait]
    impl WalletHttpHost for PublicKeyHost {
        async fn execute_http(&self, request: HttpRequest) -> Result<HttpResponse, HttpHostError> {
            let encoded = self
                .public_key
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            Ok(HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: serde_json::to_vec(&serde_json::json!({
                    "result": { "stack": [["num", format!("0x{encoded}")]] }
                }))
                .expect("response JSON"),
                final_url: request.url,
            })
        }

        async fn cancel_http(&self, _request_id: HttpRequestId) {}
    }

    struct SecretHost {
        reasons: Mutex<Vec<SecretAccessReason>>,
    }

    #[async_trait::async_trait]
    impl WalletPlatformHost for SecretHost {
        async fn read_protected_secret(
            &self,
            request: ProtectedSecretRead,
        ) -> Result<Vec<u8>, ProtectedSecretHostError> {
            self.reasons
                .lock()
                .expect("reason lock")
                .push(request.reason);
            Ok(MNEMONIC.as_bytes().to_vec())
        }

        async fn store_protected_secret(
            &self,
            _request: ProtectedSecretStore,
        ) -> Result<(), ProtectedSecretHostError> {
            panic!("not used by encrypted comments")
        }

        async fn delete_protected_secret(
            &self,
            _secret_ref: ProtectedSecretRef,
        ) -> Result<(), ProtectedSecretHostError> {
            panic!("not used by encrypted comments")
        }

        async fn load_journal(
            &self,
            _key: JournalKey,
        ) -> Result<Option<JournalRecord>, JournalHostError> {
            panic!("not used by encrypted comments")
        }

        async fn compare_exchange_journal(
            &self,
            _mutation: JournalCompareExchange,
        ) -> Result<JournalCompareExchangeResult, JournalHostError> {
            panic!("not used by encrypted comments")
        }
    }

    #[test]
    fn public_workflow_fetches_the_peer_key_and_authorizes_each_secret_use() {
        let wallet = derive_wallet(MNEMONIC, Network::Testnet).expect("wallet derives");
        let source = TonAddressString::from_address(&wallet.address, Network::Testnet);
        let config = WalletClientConfig {
            record_id: NonEmptyString::try_from("encrypted-comment-test").expect("record ID"),
            address: source.clone(),
            public_key: wallet.key_pair.public_key.to_vec(),
            local_secret_ref: Some(ProtectedSecretRef {
                value: "wallet-secret".to_owned(),
            }),
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig {
                toncenter_base_url: "https://provider.example".to_owned(),
                request_timeout_ms: 15_000,
            },
        };
        let recipient_public_key = SigningKey::from_bytes(&[7_u8; 32])
            .verifying_key()
            .to_bytes();
        let platform = Arc::new(SecretHost {
            reasons: Mutex::new(Vec::new()),
        });
        let client = WalletClient::new(
            config,
            Arc::new(PublicKeyHost {
                public_key: recipient_public_key,
            }),
            platform.clone(),
        )
        .expect("client builds");

        let body = block_on(
            client.create_encrypted_comment(CreateEncryptedCommentRequest {
                recipient: TonAddressString::try_from(RECIPIENT).expect("recipient"),
                comment: "secret hello".to_owned(),
            }),
        )
        .expect("comment encrypts");
        let plaintext = block_on(client.decrypt_comment(DecryptCommentRequest {
            sender: source,
            body,
        }))
        .expect("outgoing comment decrypts");

        assert_eq!(plaintext, "secret hello");
        assert_eq!(
            *platform.reasons.lock().expect("reason lock"),
            [
                SecretAccessReason::EncryptComment,
                SecretAccessReason::DecryptComment,
            ]
        );
    }
}
