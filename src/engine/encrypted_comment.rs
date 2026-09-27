//! Explicit encrypted-comment preparation and decryption workflows.

use crate::domain::{SecretAccessReason, bounded_diagnostic};
use crate::transport::build_toncenter_v2_request;
use crate::wallet::encrypted_comment::{
    EncryptedCommentError, MAX_ENCRYPTED_COMMENT_BYTES, decrypt_comment as decrypt_body,
    encrypt_comment as encrypt_body, validate_encrypted_comment_body,
};
use crate::wallet::recipient_public_key::verify_recipient_public_key;
use crate::{
    AccountStatus, Boc, CreateEncryptedCommentRequest, DecryptCommentRequest,
    EncryptedCommentRecipientRequest, HttpRequest, Network, ProtectedSecretRead, TonAddressString,
    WalletClientConfig, WalletClientError,
};

use super::WalletClient;
use super::provider::parse_account;
use super::send_http::{PublicKeyAnswer, build_public_key_request, parse_public_key};
use super::send_state::SensitiveBytes;
use super::state::{OperationFamily, State, ensure_running};

enum RecipientPublicKeySource {
    Provided([u8; 32]),
    OnChain {
        account: HttpRequest,
        public_key: HttpRequest,
    },
}

#[uniffi::export]
impl WalletClient {
    /// Creates a TON encrypted-comment body ready for `SendMessageBody::RawPayload`.
    ///
    /// The engine uses the supplied recipient public key or calls the recipient
    /// wallet's `get_public_key` get-method, then asks the platform host to
    /// authorize this wallet's protected mnemonic.
    /// A supplied key must locally derive the recipient's address using supported
    /// default wallet parameters. Verification happens before secret authorization.
    /// No secret is requested when the comment is already too large.
    /// The recipient key is resolved exactly as
    /// [`Self::resolve_encrypted_comment_recipient`] resolves it, including its
    /// errors, and before any secret is requested.
    pub async fn create_encrypted_comment(
        &self,
        request: CreateEncryptedCommentRequest,
    ) -> Result<Boc, WalletClientError> {
        if request.comment.len() > MAX_ENCRYPTED_COMMENT_BYTES {
            return Err(encrypted_comment_error(
                "the encrypted comment exceeds 960 UTF-8 bytes",
            ));
        }
        let provided_public_key = provided_recipient_key(request.recipient_public_key.as_deref())?;

        let (generation, config, public_key_source, secret_request) = {
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
            if let Some(key) = &provided_public_key {
                verify_provided_recipient_key(&request.recipient, key, state.config.network)?;
            }
            let generation = next_resolution_generation(&mut state)?;
            let config = state.config.clone();
            let public_key_source = recipient_public_key_source(
                &mut state,
                &config,
                &request.recipient,
                provided_public_key,
            )?;
            state.active_resolution = Some((generation, Vec::new()));
            (
                generation,
                config,
                public_key_source,
                ProtectedSecretRead {
                    secret_ref,
                    reason: SecretAccessReason::EncryptComment,
                    prompt: "Authenticate to encrypt this transfer comment".to_owned(),
                },
            )
        };

        let recipient_public_key = self
            .resolve_recipient_public_key(generation, public_key_source)
            .await?;

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

    /// Resolves the Ed25519 public key an encrypted comment for a recipient uses.
    ///
    /// This answers whether [`Self::create_encrypted_comment`] with the same
    /// recipient and supplied key can encrypt, without a comment and without
    /// requesting any protected secret, so a client configured without a local
    /// signing secret can ask it too. A supplied key is verified locally, with
    /// no HTTP request and without the single-flight slot. Otherwise the engine
    /// reads the recipient account state and, for an active contract, calls its
    /// `get_public_key` get-method.
    ///
    /// `EncryptedCommentUnavailable` means the recipient cannot receive an
    /// encrypted comment: the supplied key does not derive its address, its
    /// wallet is not deployed or is frozen, or its contract did not return a
    /// public key. `EncryptedCommentLookupFailed` means the provider did not
    /// answer and nothing is known about the recipient.
    pub async fn resolve_encrypted_comment_recipient(
        &self,
        request: EncryptedCommentRecipientRequest,
    ) -> Result<Vec<u8>, WalletClientError> {
        let provided_public_key = provided_recipient_key(request.recipient_public_key.as_deref())?;

        let (generation, public_key_source) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            if let Some(key) = provided_public_key {
                verify_provided_recipient_key(&request.recipient, &key, state.config.network)?;
                return Ok(key.to_vec());
            }
            if state.active_send.is_some() || state.active_resolution.is_some() {
                return Err(WalletClientError::SendAlreadyInProgress);
            }
            let generation = next_resolution_generation(&mut state)?;
            let config = state.config.clone();
            let public_key_source =
                recipient_public_key_source(&mut state, &config, &request.recipient, None)?;
            state.active_resolution = Some((generation, Vec::new()));
            (generation, public_key_source)
        };

        let recipient_public_key = self
            .resolve_recipient_public_key(generation, public_key_source)
            .await?;
        self.complete_encrypted_comment_operation(generation)?;
        Ok(recipient_public_key.to_vec())
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

impl WalletClient {
    /// Resolves the recipient key while `generation` holds the resolution slot.
    ///
    /// Every failure releases the slot. Only evidence about the recipient
    /// becomes `EncryptedCommentUnavailable`; a provider that did not answer
    /// becomes `EncryptedCommentLookupFailed`, so no caller gives up on
    /// encryption because of a transient failure.
    async fn resolve_recipient_public_key(
        &self,
        generation: u64,
        source: RecipientPublicKeySource,
    ) -> Result<[u8; 32], WalletClientError> {
        let (account_request, public_key_request) = match source {
            RecipientPublicKeySource::Provided(key) => return Ok(key),
            RecipientPublicKeySource::OnChain {
                account,
                public_key,
            } => (account, public_key),
        };

        let body = self
            .execute_recipient_request(generation, &account_request)
            .await?;
        let account = parse_account(&body)
            .map_err(|error| self.fail_recipient_lookup(generation, error.developer_message))?;
        match account.status {
            AccountStatus::Active => {}
            AccountStatus::Nonexistent | AccountStatus::Uninitialized => {
                return Err(self.fail_encrypted_comment(
                    generation,
                    "the recipient wallet is not deployed and cannot report its public key",
                ));
            }
            AccountStatus::Frozen => {
                return Err(self.fail_encrypted_comment(
                    generation,
                    "the recipient account is frozen and cannot report its public key",
                ));
            }
            AccountStatus::Unknown => {
                return Err(self.fail_recipient_lookup(
                    generation,
                    "the provider returned an unrecognized recipient account state",
                ));
            }
        }

        let body = self
            .execute_recipient_request(generation, &public_key_request)
            .await?;
        match parse_public_key(&body) {
            Ok(PublicKeyAnswer::Key(key)) => Ok(key),
            Ok(PublicKeyAnswer::NoKey(reason)) => {
                Err(self.fail_encrypted_comment(generation, reason))
            }
            Err(error) => Err(self.fail_recipient_lookup(generation, error.developer_message)),
        }
    }

    async fn execute_recipient_request(
        &self,
        generation: u64,
        request: &HttpRequest,
    ) -> Result<Vec<u8>, WalletClientError> {
        match self
            .execute_tracked_standalone_resolution_request(generation, request)
            .await
        {
            Ok(Ok(body)) => Ok(body),
            Ok(Err(error)) => Err(self.fail_recipient_lookup(generation, error.developer_message)),
            Err(error) => {
                self.discard_encrypted_comment_operation(generation);
                Err(error)
            }
        }
    }

    fn fail_recipient_lookup(
        &self,
        generation: u64,
        message: impl AsRef<str>,
    ) -> WalletClientError {
        self.discard_encrypted_comment_operation(generation);
        WalletClientError::EncryptedCommentLookupFailed {
            diagnostic: bounded_diagnostic(message),
        }
    }
}

fn provided_recipient_key(key: Option<&[u8]>) -> Result<Option<[u8; 32]>, WalletClientError> {
    key.map(<[u8; 32]>::try_from).transpose().map_err(|_| {
        encrypted_comment_error(EncryptedCommentError::InvalidPeerPublicKey.to_string())
    })
}

fn verify_provided_recipient_key(
    recipient: &TonAddressString,
    key: &[u8; 32],
    network: Network,
) -> Result<(), WalletClientError> {
    verify_recipient_public_key(recipient, key, network)
        .map_err(|error| encrypted_comment_error(error.to_string()))
}

fn next_resolution_generation(state: &mut State) -> Result<u64, WalletClientError> {
    state.resolution_generation = state
        .resolution_generation
        .checked_add(1)
        .ok_or(WalletClientError::IdentifierExhausted)?;
    Ok(state.resolution_generation)
}

fn recipient_public_key_source(
    state: &mut State,
    config: &WalletClientConfig,
    recipient: &TonAddressString,
    provided: Option<[u8; 32]>,
) -> Result<RecipientPublicKeySource, WalletClientError> {
    if let Some(key) = provided {
        return Ok(RecipientPublicKeySource::Provided(key));
    }
    let account = build_toncenter_v2_request(
        config,
        state.allocate_request_id()?,
        "getAddressInformation",
        &[("address", recipient.as_str())],
    )?;
    let public_key = build_public_key_request(config, state.allocate_request_id()?, recipient)?;
    Ok(RecipientPublicKeySource::OnChain {
        account,
        public_key,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ed25519_dalek::SigningKey;
    use futures::executor::block_on;

    use super::*;
    use crate::wallet::crypto::derive_wallet;
    use crate::{
        HttpHostError, HttpHostErrorKind, HttpRequest, HttpRequestId, HttpResponse,
        JournalCompareExchange, JournalCompareExchangeResult, JournalHostError, JournalKey,
        JournalRecord, Network, NonEmptyString, ProtectedSecretHostError, ProtectedSecretRef,
        ProtectedSecretStore, ProviderConfig, TonAddressString, WalletClientConfig, WalletHttpHost,
        WalletPlatformHost,
    };

    const MNEMONIC: &str = "notice tortoise soup strong gun divide offer process salon siren general carry clump left year void clutch tool case burden fix income champion lounge";
    const RECIPIENT: &str = "0:2222222222222222222222222222222222222222222222222222222222222222";

    struct PublicKeyHost {
        public_key: [u8; 32],
        requests: Mutex<Vec<HttpRequest>>,
    }

    #[async_trait::async_trait]
    impl WalletHttpHost for PublicKeyHost {
        async fn execute_http(&self, request: HttpRequest) -> Result<HttpResponse, HttpHostError> {
            self.requests
                .lock()
                .expect("request lock")
                .push(request.clone());
            let body = if is_account_request(&request) {
                account_body("active")
            } else {
                public_key_body(&self.public_key)
            };
            Ok(response(request, 200, body))
        }

        async fn cancel_http(&self, _request_id: HttpRequestId) {}
    }

    /// How a scripted provider answers the recipient `get_public_key` call.
    #[derive(Clone)]
    enum GetterReply {
        Key([u8; 32]),
        Body(u16, String),
        HostFailure,
    }

    /// Answers the account-state read with `account_state` and the getter
    /// with `getter`, recording every request.
    struct RecipientHost {
        account_state: Option<&'static str>,
        getter: GetterReply,
        requests: Mutex<Vec<HttpRequest>>,
    }

    impl RecipientHost {
        fn new(account_state: Option<&'static str>, getter: GetterReply) -> Self {
            Self {
                account_state,
                getter,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn request_count(&self) -> usize {
            self.requests.lock().expect("request lock").len()
        }
    }

    #[async_trait::async_trait]
    impl WalletHttpHost for RecipientHost {
        async fn execute_http(&self, request: HttpRequest) -> Result<HttpResponse, HttpHostError> {
            self.requests
                .lock()
                .expect("request lock")
                .push(request.clone());
            if is_account_request(&request) {
                return match self.account_state {
                    Some(state) => Ok(response(request, 200, account_body(state))),
                    None => Err(HttpHostError::Failed {
                        kind: HttpHostErrorKind::Offline,
                        diagnostic: "scripted offline".to_owned(),
                    }),
                };
            }
            match self.getter.clone() {
                GetterReply::Key(key) => Ok(response(request, 200, public_key_body(&key))),
                GetterReply::Body(status, body) => Ok(response(request, status, body.into_bytes())),
                GetterReply::HostFailure => Err(HttpHostError::Failed {
                    kind: HttpHostErrorKind::Timeout,
                    diagnostic: "scripted timeout".to_owned(),
                }),
            }
        }

        async fn cancel_http(&self, _request_id: HttpRequestId) {}
    }

    fn is_account_request(request: &HttpRequest) -> bool {
        request.url.contains("getAddressInformation")
    }

    fn account_body(state: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "ok": true,
            "result": { "balance": "1000000000", "state": state, "sync_utime": 1 }
        }))
        .expect("response JSON")
    }

    fn public_key_body(public_key: &[u8; 32]) -> Vec<u8> {
        let encoded = public_key
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        serde_json::to_vec(&serde_json::json!({
            "ok": true,
            "result": { "exit_code": 0, "stack": [["num", format!("0x{encoded}")]] }
        }))
        .expect("response JSON")
    }

    fn response(request: HttpRequest, status: u16, body: Vec<u8>) -> HttpResponse {
        HttpResponse {
            status,
            headers: Vec::new(),
            body,
            final_url: request.url,
        }
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

    fn client_config() -> WalletClientConfig {
        let wallet = derive_wallet(MNEMONIC, Network::Testnet).expect("wallet derives");
        let source = TonAddressString::from_address(&wallet.address, Network::Testnet);
        WalletClientConfig {
            record_id: NonEmptyString::try_from("encrypted-comment-test").expect("record ID"),
            address: source,
            public_key: wallet.key_pair.public_key.to_vec(),
            local_secret_ref: Some(ProtectedSecretRef {
                value: "wallet-secret".to_owned(),
            }),
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig {
                toncenter_base_url: "https://provider.example".to_owned(),
                dns_root_address: None,
                request_timeout_ms: 15_000,
            },
        }
    }

    #[test]
    fn public_workflow_fetches_the_peer_key_and_authorizes_each_secret_use() {
        let config = client_config();
        let source = config.address.clone();
        let recipient_public_key = SigningKey::from_bytes(&[7_u8; 32])
            .verifying_key()
            .to_bytes();
        let platform = Arc::new(SecretHost {
            reasons: Mutex::new(Vec::new()),
        });
        let http = Arc::new(PublicKeyHost {
            public_key: recipient_public_key,
            requests: Mutex::new(Vec::new()),
        });
        let client =
            WalletClient::new(config, http.clone(), platform.clone()).expect("client builds");

        let body = block_on(
            client.create_encrypted_comment(CreateEncryptedCommentRequest {
                recipient: TonAddressString::try_from(RECIPIENT).expect("recipient"),
                comment: "secret hello".to_owned(),
                recipient_public_key: None,
            }),
        )
        .expect("comment encrypts");
        let plaintext = block_on(client.decrypt_comment(DecryptCommentRequest {
            sender: source,
            body,
        }))
        .expect("outgoing comment decrypts");

        assert_eq!(plaintext, "secret hello");
        assert_eq!(http.requests.lock().expect("request lock").len(), 2);
        assert_eq!(
            *platform.reasons.lock().expect("reason lock"),
            [
                SecretAccessReason::EncryptComment,
                SecretAccessReason::DecryptComment,
            ]
        );
    }

    #[test]
    fn supplied_key_skips_http_and_encrypts_for_the_recipient() {
        const RECIPIENT_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let recipient_wallet =
            derive_wallet(RECIPIENT_MNEMONIC, Network::Testnet).expect("recipient wallet derives");
        let recipient = TonAddressString::from_address(&recipient_wallet.address, Network::Testnet);
        let config = client_config();
        let source = config.address.clone();
        let http = Arc::new(PublicKeyHost {
            public_key: [0; 32],
            requests: Mutex::new(Vec::new()),
        });
        let platform = Arc::new(SecretHost {
            reasons: Mutex::new(Vec::new()),
        });
        let client =
            WalletClient::new(config, http.clone(), platform.clone()).expect("client builds");
        let body = block_on(
            client.create_encrypted_comment(CreateEncryptedCommentRequest {
                recipient: recipient.clone(),
                comment: "secret for an undeployed wallet".to_owned(),
                recipient_public_key: Some(recipient_wallet.key_pair.public_key.to_vec()),
            }),
        )
        .expect("comment encrypts with the supplied key");

        let plaintext = decrypt_body(
            RECIPIENT_MNEMONIC.as_bytes(),
            Network::Testnet,
            &recipient,
            &source,
            &body,
        )
        .expect("recipient decrypts the comment");
        assert_eq!(plaintext, "secret for an undeployed wallet");
        assert!(http.requests.lock().expect("request lock").is_empty());
        assert_eq!(
            *platform.reasons.lock().expect("reason lock"),
            [SecretAccessReason::EncryptComment],
        );
        assert!(
            client
                .lock()
                .expect("state lock")
                .active_resolution
                .is_none()
        );
    }

    #[test]
    fn supplied_key_with_invalid_length_is_rejected_before_io() {
        let http = Arc::new(PublicKeyHost {
            public_key: [0; 32],
            requests: Mutex::new(Vec::new()),
        });
        let platform = Arc::new(SecretHost {
            reasons: Mutex::new(Vec::new()),
        });
        let client = WalletClient::new(client_config(), http.clone(), platform.clone())
            .expect("client builds");

        for length in [0, 31, 33] {
            let error = block_on(
                client.create_encrypted_comment(CreateEncryptedCommentRequest {
                    recipient: TonAddressString::try_from(RECIPIENT).expect("recipient"),
                    comment: "secret hello".to_owned(),
                    recipient_public_key: Some(vec![7; length]),
                }),
            )
            .expect_err("invalid key length must fail");
            assert!(matches!(
                error,
                WalletClientError::EncryptedCommentUnavailable { .. }
            ));
        }
        assert!(http.requests.lock().expect("request lock").is_empty());
        assert!(platform.reasons.lock().expect("reason lock").is_empty());
        assert!(
            client
                .lock()
                .expect("state lock")
                .active_resolution
                .is_none()
        );
    }

    #[test]
    fn supplied_key_for_another_wallet_is_rejected_before_io() {
        let config = client_config();
        let recipient = config.address.clone();
        let correct_key = config.public_key.clone();
        let wrong_key = SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes();
        let http = Arc::new(PublicKeyHost {
            public_key: wrong_key,
            requests: Mutex::new(Vec::new()),
        });
        let platform = Arc::new(SecretHost {
            reasons: Mutex::new(Vec::new()),
        });
        let client =
            WalletClient::new(config, http.clone(), platform.clone()).expect("client builds");

        let error = block_on(
            client.create_encrypted_comment(CreateEncryptedCommentRequest {
                recipient: recipient.clone(),
                comment: "must remain private".to_owned(),
                recipient_public_key: Some(wrong_key.to_vec()),
            }),
        )
        .expect_err("a key for another wallet must fail");
        assert_eq!(
            error,
            encrypted_comment_error(EncryptedCommentError::RecipientPublicKeyMismatch.to_string()),
        );
        assert!(http.requests.lock().expect("request lock").is_empty());
        assert!(platform.reasons.lock().expect("reason lock").is_empty());
        assert!(
            client
                .lock()
                .expect("state lock")
                .active_resolution
                .is_none()
        );

        block_on(
            client.create_encrypted_comment(CreateEncryptedCommentRequest {
                recipient,
                comment: "the correct key still works".to_owned(),
                recipient_public_key: Some(correct_key),
            }),
        )
        .expect("rejected key does not leave a pending operation");
        assert!(http.requests.lock().expect("request lock").is_empty());
        assert_eq!(
            *platform.reasons.lock().expect("reason lock"),
            [SecretAccessReason::EncryptComment],
        );
    }

    fn resolve(
        client: &WalletClient,
        recipient: TonAddressString,
        recipient_public_key: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, WalletClientError> {
        block_on(
            client.resolve_encrypted_comment_recipient(EncryptedCommentRecipientRequest {
                recipient,
                recipient_public_key,
            }),
        )
    }

    fn raw_recipient() -> TonAddressString {
        TonAddressString::try_from(RECIPIENT).expect("recipient")
    }

    fn secretless_config() -> WalletClientConfig {
        WalletClientConfig {
            local_secret_ref: None,
            ..client_config()
        }
    }

    fn secret_host() -> Arc<SecretHost> {
        Arc::new(SecretHost {
            reasons: Mutex::new(Vec::new()),
        })
    }

    fn slot_released(client: &WalletClient) -> bool {
        client
            .lock()
            .expect("state lock")
            .active_resolution
            .is_none()
    }

    #[test]
    fn resolution_reads_the_recipient_key_without_a_secret() {
        let key = SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes();
        let http = Arc::new(RecipientHost::new(Some("active"), GetterReply::Key(key)));
        let platform = secret_host();
        let client = WalletClient::new(secretless_config(), http.clone(), platform.clone())
            .expect("a client without a local secret builds");

        let resolved = resolve(&client, raw_recipient(), None).expect("an active wallet answers");

        assert_eq!(resolved, key.to_vec());
        assert_eq!(http.request_count(), 2);
        assert!(platform.reasons.lock().expect("reason lock").is_empty());
        assert!(slot_released(&client));

        let json_rpc_success = format!(
            r#"{{"ok":true,"error":null,"result":{{"exit_code":0,"stack":[["num","0x{}"]]}}}}"#,
            "11".repeat(32)
        );
        let http = Arc::new(RecipientHost::new(
            Some("active"),
            GetterReply::Body(200, json_rpc_success),
        ));
        let client =
            WalletClient::new(secretless_config(), http, secret_host()).expect("client builds");
        assert_eq!(
            resolve(&client, raw_recipient(), None).expect("a null error is no error"),
            vec![0x11; 32]
        );
    }

    #[test]
    fn resolution_verifies_a_supplied_key_locally_even_while_the_slot_is_busy() {
        let config = secretless_config();
        let recipient = config.address.clone();
        let correct_key = config.public_key.clone();
        let wrong_key = SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes();
        let http = Arc::new(RecipientHost::new(None, GetterReply::HostFailure));
        let platform = secret_host();
        let client =
            WalletClient::new(config, http.clone(), platform.clone()).expect("client builds");
        client.lock().expect("state lock").active_resolution = Some((u64::MAX, Vec::new()));

        assert_eq!(
            resolve(&client, recipient.clone(), Some(correct_key.clone()))
                .expect("the supplied key derives the recipient"),
            correct_key
        );
        assert!(matches!(
            resolve(&client, recipient.clone(), Some(wrong_key.to_vec())),
            Err(WalletClientError::EncryptedCommentUnavailable { .. })
        ));
        assert!(matches!(
            resolve(&client, recipient.clone(), Some(vec![7; 31])),
            Err(WalletClientError::EncryptedCommentUnavailable { .. })
        ));
        assert_eq!(
            resolve(&client, recipient, None),
            Err(WalletClientError::SendAlreadyInProgress)
        );
        assert_eq!(http.request_count(), 0);
        assert!(platform.reasons.lock().expect("reason lock").is_empty());
    }

    #[test]
    fn undeployed_and_frozen_recipients_cannot_receive_encrypted_comments() {
        for state in ["uninit", "uninitialized", "nonexist", "frozen"] {
            let http = Arc::new(RecipientHost::new(Some(state), GetterReply::HostFailure));
            let client = WalletClient::new(secretless_config(), http.clone(), secret_host())
                .expect("client builds");

            let error = resolve(&client, raw_recipient(), None).expect_err("no key to resolve");

            assert!(
                matches!(error, WalletClientError::EncryptedCommentUnavailable { .. }),
                "{state}: {error:?}"
            );
            assert_eq!(http.request_count(), 1, "{state} never runs the getter");
            assert!(slot_released(&client), "{state}");
        }
    }

    #[test]
    fn only_a_contract_answer_is_evidence_that_encryption_is_unavailable() {
        let key = "11".repeat(32);
        let unavailable = [
            format!(r#"{{"ok":true,"result":{{"exit_code":11,"stack":[["num","0x{key}"]]}}}}"#),
            r#"{"ok":true,"result":{"exit_code":0,"stack":[]}}"#.to_owned(),
            r#"{"ok":true,"result":{"exit_code":0,"stack":[["num","0"]]}}"#.to_owned(),
        ];
        for body in unavailable {
            let http = Arc::new(RecipientHost::new(
                Some("active"),
                GetterReply::Body(200, body.clone()),
            ));
            let client =
                WalletClient::new(secretless_config(), http, secret_host()).expect("client builds");
            let error = resolve(&client, raw_recipient(), None).expect_err("no usable key");
            assert!(
                matches!(error, WalletClientError::EncryptedCommentUnavailable { .. }),
                "{body}: {error:?}"
            );
            assert!(slot_released(&client), "{body}");
        }

        let silent = [
            (Some("active"), GetterReply::HostFailure),
            (None, GetterReply::Key([0x11; 32])),
            (Some("strange"), GetterReply::Key([0x11; 32])),
            (
                Some("active"),
                GetterReply::Body(
                    429,
                    r#"{"ok":false,"error":"Ratelimit exceed","code":429}"#.to_owned(),
                ),
            ),
            (
                Some("active"),
                GetterReply::Body(502, "<html>Bad gateway</html>".to_owned()),
            ),
            (
                Some("active"),
                GetterReply::Body(
                    200,
                    r#"{"ok":false,"error":"LITE_SERVER_NETWORK","code":500}"#.to_owned(),
                ),
            ),
            (
                Some("active"),
                GetterReply::Body(200, "<html>proxy</html>".to_owned()),
            ),
            (
                Some("active"),
                GetterReply::Body(200, r#"{"ok":true}"#.to_owned()),
            ),
        ];
        for (index, (account, getter)) in silent.into_iter().enumerate() {
            let http = Arc::new(RecipientHost::new(account, getter));
            let client =
                WalletClient::new(secretless_config(), http, secret_host()).expect("client builds");
            let error = resolve(&client, raw_recipient(), None).expect_err("lookup fails");
            assert!(
                matches!(
                    error,
                    WalletClientError::EncryptedCommentLookupFailed { .. }
                ),
                "case {index}: {error:?}"
            );
            assert!(slot_released(&client), "case {index}");
        }
    }

    #[test]
    fn encryption_resolves_the_recipient_before_asking_for_the_secret() {
        let cases = [
            (Some("uninit"), GetterReply::HostFailure, false),
            (Some("active"), GetterReply::HostFailure, true),
        ];
        for (account, getter, lookup_failed) in cases {
            let http = Arc::new(RecipientHost::new(account, getter));
            let platform = secret_host();
            let client =
                WalletClient::new(client_config(), http, platform.clone()).expect("client builds");

            let error = block_on(
                client.create_encrypted_comment(CreateEncryptedCommentRequest {
                    recipient: raw_recipient(),
                    comment: "must stay private".to_owned(),
                    recipient_public_key: None,
                }),
            )
            .expect_err("nothing to encrypt for");

            assert_eq!(
                matches!(
                    error,
                    WalletClientError::EncryptedCommentLookupFailed { .. }
                ),
                lookup_failed,
                "{error:?}"
            );
            assert_eq!(
                matches!(error, WalletClientError::EncryptedCommentUnavailable { .. }),
                !lookup_failed,
                "{error:?}"
            );
            assert!(platform.reasons.lock().expect("reason lock").is_empty());
            assert!(slot_released(&client));
        }
    }
}
