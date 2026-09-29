//! Explicit encrypted-comment preparation and decryption workflows.

use ed25519_dalek::SigningKey;

use crate::domain::{SecretAccessReason, bounded_diagnostic};
use crate::transport::build_toncenter_v2_request;
use crate::wallet::crypto::RotationKeys;
use crate::wallet::encrypted_comment::{
    EncryptedCommentError, MAX_ENCRYPTED_COMMENT_BYTES, comment_keys, decrypt_comment_with_keys,
    encrypt_comment as encrypt_body, validate_encrypted_comment_body,
};
use crate::wallet::key_history::{KeyChange, recover_signing_keys};
use crate::wallet::recipient_public_key::verify_recipient_public_key;
use crate::{
    AccountStatus, Boc, CreateEncryptedCommentRequest, DecryptCommentRequest,
    EncryptedCommentRecipientRequest, HttpRequest, Network, ProtectedSecretRead, TonAddressString,
    WalletClientConfig, WalletClientError,
};

use super::WalletClient;
use super::key_history::{
    MAX_KEY_CHANGE_PAGES, build_key_change_request, history_reaches, parse_key_change_page,
};
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
    /// authorize this wallet's protected mnemonic. The sender key is this
    /// wallet's current signing key.
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
    ///
    /// The body may use any signing key this wallet ever had. The engine first
    /// tries the current signing key and the anchor key, which the recovery
    /// phrase holds, without any HTTP request. When neither matches and the
    /// wallet has rotated its key, the engine reads the wallet's
    /// `change_wallet_key` actions from Toncenter v3, recovers each earlier
    /// signing key from the encrypted old key its rotation published, and tries
    /// those. The secret is read once per call and recovered keys never leave
    /// it. The history holds no secret; later calls reuse it while it still
    /// contains the rotation that installed the current signing key.
    ///
    /// `EncryptedCommentLookupFailed` means the provider did not answer or did
    /// not return a history that reaches the current signing key yet; a retry
    /// can succeed.
    /// `EncryptedCommentUnavailable` means no key of this wallet decrypts the
    /// body or the body is malformed.
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
        let keys = match comment_keys(secret.as_slice(), config.network, &config.address) {
            Ok(keys) => keys,
            Err(EncryptedCommentError::InvalidMnemonic) => {
                return Err(self.finish_invalid_protected_secret(generation));
            }
            Err(error) => return Err(self.fail_encrypted_comment(generation, error.to_string())),
        };
        drop(secret);
        let RotationKeys { anchor, signing } = keys;
        let anchor_public_key = anchor.verifying_key().to_bytes();
        let signing_public_key = signing.verifying_key().to_bytes();
        let rotated = anchor_public_key != signing_public_key;

        let mut own_keys: Vec<&SigningKey> = vec![&signing];
        if rotated {
            own_keys.push(&anchor);
        }
        match decrypt_comment_with_keys(own_keys, &request.sender, &request.body) {
            Ok(comment) => {
                self.complete_encrypted_comment_operation(generation)?;
                return Ok(comment);
            }
            Err(error) if rotated && error.is_key_mismatch() => {}
            Err(error) => return Err(self.fail_encrypted_comment(generation, error.to_string())),
        }
        drop(anchor);

        // Only earlier signing keys remain. They are recovered from public
        // history with the current key, which stays in memory until then.
        let changes = self
            .key_change_history(generation, &signing_public_key)
            .await?;
        if !history_reaches(&changes, &signing_public_key) {
            return Err(self.fail_encrypted_comment_lookup(
                generation,
                "the provider's key-change history does not include the current signing key yet",
            ));
        }
        let earlier_keys = recover_signing_keys(&anchor_public_key, &signing, &changes);
        drop(signing);
        let comment = match decrypt_comment_with_keys(&earlier_keys, &request.sender, &request.body)
        {
            Ok(comment) => comment,
            Err(error) => {
                return Err(self.fail_encrypted_comment(generation, error.to_string()));
            }
        };
        self.complete_encrypted_comment_operation(generation)?;
        Ok(comment)
    }
}

impl WalletClient {
    /// Returns this wallet's key-change history while `generation` holds the
    /// resolution slot.
    ///
    /// A cached history that already contains the rotation installing
    /// `current_public_key` is complete for every older key and is reused.
    /// Otherwise the engine reads all pages again and caches the result. Every
    /// failure releases the slot.
    async fn key_change_history(
        &self,
        generation: u64,
        current_public_key: &[u8; 32],
    ) -> Result<Vec<KeyChange>, WalletClientError> {
        {
            let state = self.lock()?;
            if let Some(changes) = &state.key_changes
                && history_reaches(changes, current_public_key)
            {
                return Ok(changes.clone());
            }
        }

        let mut changes = Vec::new();
        let mut offset = 0_usize;
        for _ in 0..MAX_KEY_CHANGE_PAGES {
            let (request, address) = {
                let built = self.lock().and_then(|mut state| {
                    if !state.is_current(OperationFamily::Resolution, generation) {
                        return Err(WalletClientError::StateUnavailable);
                    }
                    let id = state.allocate_request_id()?;
                    let request = build_key_change_request(&state.config, id, offset)?;
                    Ok((request, state.config.address.clone()))
                });
                built.inspect_err(|_| self.discard_encrypted_comment_operation(generation))?
            };
            let body = self
                .execute_encrypted_comment_request(generation, &request)
                .await?;
            let page = parse_key_change_page(&body, &address).map_err(|error| {
                self.fail_encrypted_comment_lookup(generation, error.developer_message)
            })?;
            offset = offset.saturating_add(page.raw_count);
            let has_more = page.has_more();
            changes.extend(page.changes);
            if !has_more {
                break;
            }
        }

        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Resolution, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.key_changes = Some(changes.clone());
        Ok(changes)
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
            .execute_encrypted_comment_request(generation, &account_request)
            .await?;
        let account = parse_account(&body).map_err(|error| {
            self.fail_encrypted_comment_lookup(generation, error.developer_message)
        })?;
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
                return Err(self.fail_encrypted_comment_lookup(
                    generation,
                    "the provider returned an unrecognized recipient account state",
                ));
            }
        }

        let body = self
            .execute_encrypted_comment_request(generation, &public_key_request)
            .await?;
        match parse_public_key(&body) {
            Ok(PublicKeyAnswer::Key(key)) => Ok(key),
            Ok(PublicKeyAnswer::NoKey(reason)) => {
                Err(self.fail_encrypted_comment(generation, reason))
            }
            Err(error) => {
                Err(self.fail_encrypted_comment_lookup(generation, error.developer_message))
            }
        }
    }

    async fn execute_encrypted_comment_request(
        &self,
        generation: u64,
        request: &HttpRequest,
    ) -> Result<Vec<u8>, WalletClientError> {
        match self
            .execute_tracked_standalone_resolution_request(generation, request)
            .await
        {
            Ok(Ok(body)) => Ok(body),
            Ok(Err(error)) => {
                Err(self.fail_encrypted_comment_lookup(generation, error.developer_message))
            }
            Err(error) => {
                self.discard_encrypted_comment_operation(generation);
                Err(error)
            }
        }
    }

    fn fail_encrypted_comment_lookup(
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

        let recipient_keys =
            comment_keys(RECIPIENT_MNEMONIC.as_bytes(), Network::Testnet, &recipient)
                .expect("recipient keys derive");
        let plaintext = decrypt_comment_with_keys([&recipient_keys.signing], &source, &body)
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

    /// Key-change history fixtures: the anchor half of [`MNEMONIC`], the lost
    /// signing half K1 (words 13-24 of [`MNEMONIC`]), and the current half K2.
    mod history {
        use std::collections::VecDeque;

        use super::*;
        use crate::wallet::crypto::{RotationKeys, derive_half_key, derive_rotation_keys};
        use crate::wallet::encrypted_comment::encrypt_comment;
        use crate::wallet::key_history::encrypt_old_private_key;
        use crate::wallet::mnemonic::{Bip39Half, RotationMnemonic};

        const SENDER_MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

        pub(super) struct Keys {
            pub(super) anchor: SigningKey,
            pub(super) lost: SigningKey,
            pub(super) current: SigningKey,
            pub(super) current_phrase: String,
        }

        pub(super) fn keys() -> Keys {
            let rotated = RotationMnemonic::parse(MNEMONIC).expect("fixture parses");
            let RotationKeys { anchor, signing } = derive_rotation_keys(&rotated);
            let current_half = Bip39Half::from_entropy(&[0x42; 16]).expect("fixed entropy encodes");
            let current_phrase = format!(
                "{} {}",
                rotated.anchor().to_phrase().as_str(),
                current_half.to_phrase().as_str()
            );
            Keys {
                anchor,
                lost: signing,
                current: derive_half_key(&current_half),
                current_phrase,
            }
        }

        fn hex(bytes: &[u8]) -> String {
            bytes.iter().map(|byte| format!("{byte:02x}")).collect()
        }

        pub(super) fn rotation(
            wallet: &TonAddressString,
            old: &SigningKey,
            new: &SigningKey,
        ) -> serde_json::Value {
            serde_json::json!({
                "action_id": hex(&new.verifying_key().to_bytes()),
                "success": true,
                "type": "change_wallet_key",
                "details": {
                    "source": null,
                    "destination": wallet.as_str(),
                    "new_public_key": hex(&new.verifying_key().to_bytes()),
                    "rotation_signature": null,
                    "encrypted_old_private_key": hex(&encrypt_old_private_key(old, new)),
                }
            })
        }

        pub(super) fn full_history(
            wallet: &TonAddressString,
            keys: &Keys,
        ) -> Vec<serde_json::Value> {
            vec![
                rotation(wallet, &keys.lost, &keys.current),
                rotation(wallet, &keys.anchor, &keys.lost),
            ]
        }

        pub(super) fn page(actions: Vec<serde_json::Value>) -> Vec<u8> {
            serde_json::to_vec(&serde_json::json!({
                "actions": actions,
                "address_book": {},
                "metadata": {}
            }))
            .expect("page JSON")
        }

        pub(super) fn sender() -> TonAddressString {
            let wallet = derive_wallet(SENDER_MNEMONIC, Network::Testnet).expect("sender derives");
            TonAddressString::from_address(&wallet.address, Network::Testnet)
        }

        /// A comment from [`SENDER_MNEMONIC`] encrypted to `recipient`.
        pub(super) fn comment_to(recipient: &SigningKey, text: &str) -> Boc {
            encrypt_comment(
                SENDER_MNEMONIC.as_bytes(),
                Network::Testnet,
                &sender(),
                &recipient.verifying_key().to_bytes(),
                text,
            )
            .expect("comment encrypts")
        }

        pub(super) enum Reply {
            Page(Vec<u8>),
            Status(u16),
            HostFailure,
        }

        /// Answers only `/api/v3/actions`, with scripted replies in order.
        pub(super) struct HistoryHost {
            pub(super) replies: Mutex<VecDeque<Reply>>,
            pub(super) requests: Mutex<Vec<HttpRequest>>,
        }

        impl HistoryHost {
            pub(super) fn new(replies: Vec<Reply>) -> Arc<Self> {
                Arc::new(Self {
                    replies: Mutex::new(replies.into()),
                    requests: Mutex::new(Vec::new()),
                })
            }

            pub(super) fn urls(&self) -> Vec<String> {
                self.requests
                    .lock()
                    .expect("request lock")
                    .iter()
                    .map(|request| request.url.clone())
                    .collect()
            }

            pub(super) fn push(&self, reply: Reply) {
                self.replies.lock().expect("reply lock").push_back(reply);
            }
        }

        #[async_trait::async_trait]
        impl WalletHttpHost for HistoryHost {
            async fn execute_http(
                &self,
                request: HttpRequest,
            ) -> Result<HttpResponse, HttpHostError> {
                self.requests
                    .lock()
                    .expect("request lock")
                    .push(request.clone());
                assert!(
                    request.url.contains("/api/v3/actions?"),
                    "unexpected request {}",
                    request.url
                );
                let reply = self
                    .replies
                    .lock()
                    .expect("reply lock")
                    .pop_front()
                    .expect("a scripted reply");
                match reply {
                    Reply::Page(body) => Ok(response(request, 200, body)),
                    Reply::Status(status) => Ok(response(request, status, b"{}".to_vec())),
                    Reply::HostFailure => Err(HttpHostError::Failed {
                        kind: HttpHostErrorKind::Offline,
                        diagnostic: "scripted offline".to_owned(),
                    }),
                }
            }

            async fn cancel_http(&self, _request_id: HttpRequestId) {}
        }

        /// Returns `phrase` for every secret read and records each reason.
        pub(super) struct PhraseHost {
            pub(super) phrase: String,
            pub(super) reasons: Mutex<Vec<SecretAccessReason>>,
        }

        impl PhraseHost {
            pub(super) fn new(phrase: &str) -> Arc<Self> {
                Arc::new(Self {
                    phrase: phrase.to_owned(),
                    reasons: Mutex::new(Vec::new()),
                })
            }

            pub(super) fn reads(&self) -> usize {
                self.reasons.lock().expect("reason lock").len()
            }
        }

        #[async_trait::async_trait]
        impl WalletPlatformHost for PhraseHost {
            async fn read_protected_secret(
                &self,
                request: ProtectedSecretRead,
            ) -> Result<Vec<u8>, ProtectedSecretHostError> {
                self.reasons
                    .lock()
                    .expect("reason lock")
                    .push(request.reason);
                Ok(self.phrase.as_bytes().to_vec())
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
    }

    fn decrypt(client: &WalletClient, body: Boc) -> Result<String, WalletClientError> {
        block_on(client.decrypt_comment(DecryptCommentRequest {
            sender: history::sender(),
            body,
        }))
    }

    #[test]
    fn a_rotated_wallet_decrypts_its_current_and_anchor_keys_without_history() {
        let keys = history::keys();
        let http = history::HistoryHost::new(Vec::new());
        let platform = history::PhraseHost::new(&keys.current_phrase);
        let client =
            WalletClient::new(client_config(), http.clone(), platform.clone()).expect("client");

        let current = decrypt(
            &client,
            history::comment_to(&keys.current, "to the current key"),
        )
        .expect("the current signing key decrypts");
        let anchor = decrypt(
            &client,
            history::comment_to(&keys.anchor, "to the anchor key"),
        )
        .expect("the anchor key decrypts");

        assert_eq!(current, "to the current key");
        assert_eq!(anchor, "to the anchor key");
        assert!(http.urls().is_empty(), "{:?}", http.urls());
        assert_eq!(platform.reads(), 2);
        assert!(slot_released(&client));
    }

    #[test]
    fn a_lost_signing_key_is_recovered_from_the_key_change_history() {
        let keys = history::keys();
        let config = client_config();
        let wallet = config.address.clone();
        let relayed_for_another_wallet = history::rotation(
            &TonAddressString::try_from(RECIPIENT).expect("recipient"),
            &keys.current,
            &SigningKey::from_bytes(&[9; 32]),
        );
        let mut actions = vec![relayed_for_another_wallet];
        actions.extend(history::full_history(&wallet, &keys));
        let http = history::HistoryHost::new(vec![history::Reply::Page(history::page(actions))]);
        let platform = history::PhraseHost::new(&keys.current_phrase);
        let client = WalletClient::new(config, http.clone(), platform.clone()).expect("client");

        let first = decrypt(
            &client,
            history::comment_to(&keys.lost, "sent before rotation"),
        )
        .expect("the recovered key decrypts");
        let second = decrypt(&client, history::comment_to(&keys.lost, "another old one"))
            .expect("the cached history is reused");

        assert_eq!(first, "sent before rotation");
        assert_eq!(second, "another old one");
        assert_eq!(
            http.urls(),
            [format!(
                "https://provider.example/api/v3/actions?account={}&action_type=change_wallet_key&limit=100&offset=0&sort=desc",
                wallet.as_str()
            )],
            "one history read serves both comments"
        );
        assert_eq!(
            *platform.reasons.lock().expect("reason lock"),
            [
                SecretAccessReason::DecryptComment,
                SecretAccessReason::DecryptComment
            ]
        );
        assert!(slot_released(&client));
    }

    #[test]
    fn the_history_is_read_page_by_page() {
        let keys = history::keys();
        let config = client_config();
        let wallet = config.address.clone();
        let failed = serde_json::json!({
            "success": false,
            "type": "change_wallet_key",
            "details": { "destination": wallet.as_str() }
        });
        let http = history::HistoryHost::new(vec![
            history::Reply::Page(history::page(vec![failed; 100])),
            history::Reply::Page(history::page(history::full_history(&wallet, &keys))),
        ]);
        let platform = history::PhraseHost::new(&keys.current_phrase);
        let client = WalletClient::new(config, http.clone(), platform).expect("client");

        let comment = decrypt(&client, history::comment_to(&keys.lost, "on page two"))
            .expect("the second page completes the history");

        assert_eq!(comment, "on page two");
        let urls = http.urls();
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("&offset=0&"), "{}", urls[0]);
        assert!(urls[1].contains("&offset=100&"), "{}", urls[1]);
    }

    #[test]
    fn a_history_without_the_current_key_is_a_retryable_lookup_failure() {
        let keys = history::keys();
        let config = client_config();
        let wallet = config.address.clone();
        let lagging = vec![history::rotation(&wallet, &keys.anchor, &keys.lost)];
        let http = history::HistoryHost::new(vec![history::Reply::Page(history::page(lagging))]);
        let platform = history::PhraseHost::new(&keys.current_phrase);
        let client = WalletClient::new(config, http.clone(), platform).expect("client");
        let body = history::comment_to(&keys.lost, "indexed later");

        let error = decrypt(&client, body.clone()).expect_err("the history lags");
        assert!(
            matches!(
                error,
                WalletClientError::EncryptedCommentLookupFailed { .. }
            ),
            "{error:?}"
        );
        assert!(slot_released(&client));

        http.push(history::Reply::Page(history::page(history::full_history(
            &wallet, &keys,
        ))));
        let comment = decrypt(&client, body).expect("a retry reads the history again");

        assert_eq!(comment, "indexed later");
        assert_eq!(http.urls().len(), 2);
    }

    #[test]
    fn a_provider_failure_while_reading_history_releases_the_slot() {
        let keys = history::keys();
        for reply in [
            history::Reply::HostFailure,
            history::Reply::Status(500),
            history::Reply::Page(b"{\"error\":\"unsupported\"}".to_vec()),
        ] {
            let http = history::HistoryHost::new(vec![reply]);
            let platform = history::PhraseHost::new(&keys.current_phrase);
            let client =
                WalletClient::new(client_config(), http.clone(), platform).expect("client");

            let error = decrypt(
                &client,
                history::comment_to(&keys.lost, "unreadable for now"),
            )
            .expect_err("the history is unavailable");

            assert!(
                matches!(
                    error,
                    WalletClientError::EncryptedCommentLookupFailed { .. }
                ),
                "{error:?}"
            );
            assert_eq!(http.urls().len(), 1);
            assert!(slot_released(&client));
            assert!(client.lock().expect("state lock").key_changes.is_none());
        }
    }

    #[test]
    fn a_comment_for_no_key_of_this_wallet_fails_after_the_history() {
        let keys = history::keys();
        let config = client_config();
        let wallet = config.address.clone();
        let http = history::HistoryHost::new(vec![history::Reply::Page(history::page(
            history::full_history(&wallet, &keys),
        ))]);
        let platform = history::PhraseHost::new(&keys.current_phrase);
        let client = WalletClient::new(config, http.clone(), platform).expect("client");
        let stranger = SigningKey::from_bytes(&[5; 32]);

        let error = decrypt(&client, history::comment_to(&stranger, "not for us"))
            .expect_err("no wallet key matches");

        assert!(
            matches!(
                &error,
                WalletClientError::EncryptedCommentUnavailable { diagnostic }
                    if diagnostic.contains("authentication failed")
            ),
            "{error:?}"
        );
        assert_eq!(http.urls().len(), 1);
        assert!(slot_released(&client));
    }

    #[test]
    fn an_unrotated_wallet_never_reads_history() {
        let keys = history::keys();
        let anchor_only = MNEMONIC
            .split_whitespace()
            .take(12)
            .collect::<Vec<_>>()
            .join(" ");
        let http = history::HistoryHost::new(Vec::new());
        let platform = history::PhraseHost::new(&anchor_only);
        let client = WalletClient::new(client_config(), http.clone(), platform).expect("client");

        let error = decrypt(&client, history::comment_to(&keys.lost, "someone else's"))
            .expect_err("only the anchor key exists");

        assert!(
            matches!(error, WalletClientError::EncryptedCommentUnavailable { .. }),
            "{error:?}"
        );
        assert!(http.urls().is_empty());
        assert!(slot_released(&client));
    }

    #[test]
    fn a_malformed_body_fails_before_any_history_read() {
        let keys = history::keys();
        let http = history::HistoryHost::new(Vec::new());
        let platform = history::PhraseHost::new(&keys.current_phrase);
        let client =
            WalletClient::new(client_config(), http.clone(), platform.clone()).expect("client");
        let truncated = {
            use ton::ton_core::cell::TonCell;
            use ton::ton_core::traits::tlb::TLB as _;
            let mut cell = TonCell::builder();
            cell.write_num(&0x2167_da4b_u32, 32).expect("opcode writes");
            cell.write_bits([0_u8; 20], 160)
                .expect("short payload writes");
            let cell = cell.build().expect("cell builds");
            Boc::try_from(cell.to_boc().expect("BOC encodes")).expect("valid BOC")
        };

        let error = decrypt(&client, truncated).expect_err("the body is malformed");

        assert!(
            matches!(error, WalletClientError::EncryptedCommentUnavailable { .. }),
            "{error:?}"
        );
        assert!(http.urls().is_empty());
        assert_eq!(platform.reads(), 0, "shape errors never read the secret");
    }
}
