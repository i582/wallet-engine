use std::collections::HashSet;
use std::env;
use std::fs::File;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::Method;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use tempfile::TempDir;
use ton::block_tlb::{CommonMsgInfoInt, Msg};
use ton::tlb_adapters::{DictKeyAdapterUint, DictValAdapterTLB, TLBHashMap};
use ton::ton_core::cell::TonCell;
use ton::ton_core::traits::tlb::TLB;
use ton::ton_core::types::TonAddress;
use ton::ton_core::types::tlb_core::{TLBCoins, TLBRef};
use ton::ton_wallet::{
    TonWallet, WALLET_SUBWALLET_ID_DEFAULT_TESTNET, WalletData, WalletExtMsgBody, WalletVersion,
};
use wallet_engine::{
    Boc, HttpHeader, HttpHostError, HttpHostErrorKind, HttpMethod, HttpRequest, HttpRequestId,
    HttpResponse, WalletHttpHost,
};

use super::host::{RequestKind, SubmittedMessage, decode_submitted_comment};
use super::test_wallet;

const READY_TIMEOUT: Duration = Duration::from_secs(45);
const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) struct LocalnetHttpHost {
    localnet: Mutex<Localnet>,
    address: String,
    cancelled: Mutex<HashSet<HttpRequestId>>,
    submitted_message: Mutex<Option<SubmittedMessage>>,
    submitted_boc_base64: Mutex<Option<String>>,
    activity_requests: Mutex<Vec<String>>,
    request_gate: Mutex<Option<RequestGate>>,
    request_changed: Condvar,
}

struct RequestGate {
    name: String,
    kind: RequestKind,
    request_id: Option<HttpRequestId>,
    reached: bool,
    released: bool,
}

impl std::fmt::Debug for LocalnetHttpHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalnetHttpHost")
            .finish_non_exhaustive()
    }
}

impl LocalnetHttpHost {
    pub(super) fn start(address: &str, balance_nanograms: &str) -> Result<Self, String> {
        let balance = balance_nanograms
            .parse::<u64>()
            .map_err(|error| format!("localnet balance does not fit u64: {error}"))?;
        let localnet = Localnet::start()?;
        localnet.fund(address, balance)?;
        localnet.mine()?;
        localnet.wait_for_state(address, "uninitialized", None)?;

        Ok(Self {
            localnet: Mutex::new(localnet),
            address: address.to_owned(),
            cancelled: Mutex::new(HashSet::new()),
            submitted_message: Mutex::new(None),
            submitted_boc_base64: Mutex::new(None),
            activity_requests: Mutex::new(Vec::new()),
            request_gate: Mutex::new(None),
            request_changed: Condvar::new(),
        })
    }

    pub(super) fn provider_base_url(&self) -> String {
        lock(&self.localnet).base_url.clone()
    }

    pub(super) fn submitted_message(&self) -> Option<SubmittedMessage> {
        lock(&self.submitted_message).clone()
    }

    pub(super) fn last_activity_request(&self) -> Option<String> {
        lock(&self.activity_requests).last().cloned()
    }

    pub(super) fn pause_next_request(&self, name: String, kind: RequestKind) {
        *lock(&self.request_gate) = Some(RequestGate {
            name,
            kind,
            request_id: None,
            reached: false,
            released: false,
        });
    }

    pub(super) fn wait_for_request(&self, name: &str) -> Result<(), String> {
        let mut gate = lock(&self.request_gate);
        let deadline = Instant::now() + CONFIRMATION_TIMEOUT;
        loop {
            let checkpoint = gate
                .as_ref()
                .ok_or_else(|| format!("request checkpoint `{name}` does not exist"))?;
            if checkpoint.name != name {
                return Err(format!(
                    "expected request checkpoint `{}`, got `{name}`",
                    checkpoint.name
                ));
            }
            if checkpoint.reached {
                return Ok(());
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "request checkpoint `{name}` was not reached within {CONFIRMATION_TIMEOUT:?}"
                ));
            }
            gate = match self.request_changed.wait_timeout(gate, remaining) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }

    pub(super) fn release_request(&self, name: &str) -> Result<(), String> {
        self.wait_for_request(name)?;
        let mut gate = lock(&self.request_gate);
        let checkpoint = gate
            .as_mut()
            .ok_or_else(|| format!("request checkpoint `{name}` disappeared"))?;
        checkpoint.released = true;
        self.request_changed.notify_all();
        Ok(())
    }

    pub(super) fn request_was_cancelled(&self, name: &str) -> Result<bool, String> {
        let gate = lock(&self.request_gate);
        let checkpoint = gate
            .as_ref()
            .ok_or_else(|| format!("request checkpoint `{name}` does not exist"))?;
        let request_id = checkpoint
            .request_id
            .ok_or_else(|| format!("request checkpoint `{name}` was not reached"))?;
        Ok(lock(&self.cancelled).contains(&request_id))
    }

    fn wait_at_request_gate(
        &self,
        kind: RequestKind,
        request_id: HttpRequestId,
    ) -> Result<(), HttpHostError> {
        let mut gate = lock(&self.request_gate);
        if matches!(gate.as_ref(), Some(checkpoint) if checkpoint.kind == kind && checkpoint.request_id.is_none())
        {
            let checkpoint = gate.as_mut().expect("request checkpoint exists");
            checkpoint.request_id = Some(request_id);
            checkpoint.reached = true;
            self.request_changed.notify_all();
        }

        loop {
            let Some(checkpoint) = gate.as_ref() else {
                return Ok(());
            };
            if checkpoint.request_id != Some(request_id) {
                return Ok(());
            }
            if checkpoint.released {
                *gate = None;
                return Ok(());
            }
            gate = match self.request_changed.wait(gate) {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
    }

    pub(super) fn assert_wallet(
        &self,
        expect_active: bool,
        expected_seqno: Option<u32>,
    ) -> Result<(), String> {
        let expected_state = if expect_active {
            "active"
        } else {
            "uninitialized"
        };
        self.assert_account(&self.address, expected_state, expected_seqno)
    }

    /// Waits for any localnet account to reach an exact state and optional wallet sequence number.
    pub(super) fn assert_account(
        &self,
        address: &str,
        expected_state: &str,
        expected_seqno: Option<u32>,
    ) -> Result<(), String> {
        lock(&self.localnet).wait_for_state(address, expected_state, expected_seqno)
    }

    /// Verifies that an unfunded deployment target has not become active before submission.
    #[allow(
        dead_code,
        reason = "used by the separate TON Connect integration-test target"
    )]
    pub(super) fn assert_account_absent(&self, address: &str) -> Result<(), String> {
        let localnet = lock(&self.localnet);
        let state = localnet.account_state(address)?;
        let balance = localnet.balance(address)?;
        if state == "active" || balance != 0 {
            return Err(format!(
                "deployment target must be absent before submission, got state={state}, balance={balance}"
            ));
        }
        Ok(())
    }

    /// Returns one account's current localnet balance in nanograms.
    #[allow(
        dead_code,
        reason = "used by the separate TON Connect integration-test target"
    )]
    pub(super) fn account_balance(&self, address: &str) -> Result<u128, String> {
        lock(&self.localnet).balance(address)
    }

    /// Advances Acton's virtual clock without waiting for wall-clock time.
    pub(super) fn increase_time(&self, seconds: u64) -> Result<(), String> {
        lock(&self.localnet).increase_time(seconds)
    }

    /// Delivers a wallet-signed internal message through an independently funded wallet.
    #[allow(
        dead_code,
        reason = "used by the separate TON Connect integration-test target"
    )]
    pub(super) fn relay_signed_message(
        &self,
        internal_boc: &Boc,
        attached_nanograms: u64,
    ) -> Result<(), String> {
        const RELAYER_BALANCE_NANOGRAMS: u64 = 2_000_000_000;

        let localnet = lock(&self.localnet);
        let mnemonic = std::str::from_utf8(test_wallet().other_recovery_phrase_bytes())
            .map_err(|error| error.to_string())?;
        let key_pair = test_wallet::rotation_anchor_key_pair(mnemonic)?;
        let relayer_wallet_id = WALLET_SUBWALLET_ID_DEFAULT_TESTNET
            .checked_add(1)
            .ok_or_else(|| "localnet relayer wallet ID overflowed".to_owned())?;
        let relayer =
            TonWallet::new_with_params(WalletVersion::Wallet, key_pair, 0, relayer_wallet_id)
                .map_err(|error| error.to_string())?;
        let relayer_address = relayer.address.to_string();
        localnet.fund(&relayer_address, RELAYER_BALANCE_NANOGRAMS)?;
        localnet.mine()?;
        localnet.wait_for_state(&relayer_address, "uninitialized", None)?;

        let signed_bytes = STANDARD
            .decode(internal_boc.to_base64())
            .map_err(|error| error.to_string())?;
        let signed_cell = TonCell::from_boc(signed_bytes).map_err(|error| error.to_string())?;
        let signed_message =
            Msg::<TonCell>::from_cell(&signed_cell).map_err(|error| error.to_string())?;
        let destination = TonAddress::from_str(&self.address).map_err(|error| error.to_string())?;
        let mut info = CommonMsgInfoInt::new(
            destination.to_msg_address(),
            TLBCoins::new(u128::from(attached_nanograms)),
        );
        info.bounce = false;
        let mut relayed_message = Msg::new(info, signed_message.body.value.clone());
        relayed_message.init = signed_message.init;
        let relayed_cell = relayed_message
            .to_cell()
            .map_err(|error| error.to_string())?;
        let external = relayer
            .create_ext_in_msg(vec![relayed_cell], 0, u32::MAX, true)
            .map_err(|error| error.to_string())?;
        let boc = STANDARD.encode(external.to_boc().map_err(|error| error.to_string())?);
        let (status, body) = request(
            &localnet.client,
            Method::POST,
            &format!("{}/api/v2/jsonRPC", localnet.base_url),
            Some(&json!({
                "jsonrpc": "2.0",
                "id": "wallet-engine-localnet-relay",
                "method": "sendBoc",
                "params": { "boc": boc }
            })),
        )?;
        if !(200..300).contains(&status)
            || body.pointer("/result/@type").and_then(Value::as_str) != Some("ok")
        {
            return Err(format!(
                "localnet relayer submission failed with HTTP {status}: {body}"
            ));
        }

        for _ in 0..3 {
            localnet.mine()?;
        }
        localnet.wait_for_state(&relayer_address, "active", Some(1))
    }

    pub(super) fn spam_transfers(&self, count: u32) -> Result<(), String> {
        let localnet = lock(&self.localnet);
        let mnemonic = std::str::from_utf8(test_wallet().recovery_phrase_bytes())
            .map_err(|error| error.to_string())?;
        let key_pair = test_wallet::rotation_anchor_key_pair(mnemonic)?;
        let wallet = TonWallet::new_with_params(
            WalletVersion::Wallet,
            key_pair,
            0,
            WALLET_SUBWALLET_ID_DEFAULT_TESTNET,
        )
        .map_err(|error| error.to_string())?;
        let destination = TonAddress::from_str(&self.address).map_err(|error| error.to_string())?;
        // `runGetMethod` has no meaningful result for an uninitialized account.
        // Treat its first wallet message as seqno zero and include StateInit.
        let initial_seqno = if localnet.account_state(&self.address)? == "active" {
            localnet.seqno(&self.address)?
        } else {
            0
        };

        for offset in 0..count {
            let mut info = CommonMsgInfoInt::new(destination.to_msg_address(), TLBCoins::new(1));
            info.bounce = false;
            let internal = Msg::new(info, TonCell::empty().to_owned())
                .to_cell()
                .map_err(|error| error.to_string())?;
            let seqno = initial_seqno
                .checked_add(offset)
                .ok_or_else(|| "localnet spam seqno overflowed".to_owned())?;
            let external = wallet
                .create_ext_in_msg(vec![internal], seqno, u32::MAX, seqno == 0)
                .map_err(|error| error.to_string())?;
            let boc = STANDARD.encode(external.to_boc().map_err(|error| error.to_string())?);
            let (status, body) = request(
                &localnet.client,
                Method::POST,
                &format!("{}/api/v2/jsonRPC", localnet.base_url),
                Some(&json!({
                    "jsonrpc": "2.0",
                    "id": format!("wallet-engine-localnet-spam-{offset}"),
                    "method": "sendBoc",
                    "params": { "boc": boc }
                })),
            )?;
            if !(200..300).contains(&status)
                || body.pointer("/result/@type").and_then(Value::as_str) != Some("ok")
            {
                return Err(format!(
                    "localnet spam submission failed with HTTP {status}: {body}"
                ));
            }

            localnet.mine()?;
        }

        let expected_seqno = initial_seqno
            .checked_add(count)
            .ok_or_else(|| "localnet spam seqno overflowed".to_owned())?;
        localnet.wait_for_state(&self.address, "active", Some(expected_seqno))
    }

    pub(super) fn replay_last_submission(&self) -> Result<(), String> {
        let boc = lock(&self.submitted_boc_base64)
            .clone()
            .ok_or_else(|| "no submitted BOC is available for replay".to_owned())?;
        let localnet = lock(&self.localnet);
        let (status, body) = request(
            &localnet.client,
            Method::POST,
            &format!("{}/api/v2/jsonRPC", localnet.base_url),
            Some(&json!({
                "jsonrpc": "2.0",
                "id": "wallet-engine-localnet-replay",
                "method": "sendBoc",
                "params": { "boc": boc }
            })),
        )?;
        if !(200..300).contains(&status)
            || body.pointer("/result/@type").and_then(Value::as_str) != Some("ok")
        {
            return Err(format!(
                "localnet replay submission failed with HTTP {status}: {body}"
            ));
        }

        localnet.mine()
    }

    /// Submits a complete external message without assuming an ordinary transfer body.
    pub(super) fn submit_external_boc(&self, boc: &Boc, expected_seqno: u32) -> Result<(), String> {
        let localnet = lock(&self.localnet);
        let (status, body) = request(
            &localnet.client,
            Method::POST,
            &format!("{}/api/v2/jsonRPC", localnet.base_url),
            Some(&json!({
                "jsonrpc": "2.0",
                "id": "wallet-engine-localnet-raw-external",
                "method": "sendBoc",
                "params": { "boc": boc.to_base64() }
            })),
        )?;
        if !(200..300).contains(&status)
            || body.pointer("/result/@type").and_then(Value::as_str) != Some("ok")
        {
            return Err(format!(
                "localnet raw external submission failed with HTTP {status}: {body}"
            ));
        }

        for _ in 0..3 {
            localnet.mine()?;
        }
        localnet.wait_for_state(&self.address, "active", Some(expected_seqno))
    }

    /// Returns the wallet's current public key as normalized lowercase hexadecimal.
    pub(super) fn public_key_hex(&self) -> Result<String, String> {
        let value = lock(&self.localnet).get_method_number(&self.address, "get_public_key")?;
        normalize_u256_hex(&value)
    }

    fn execute(&self, request: &HttpRequest) -> Result<HttpResponse, HttpHostError> {
        if lock(&self.cancelled).remove(&request.id) {
            return Err(host_error(
                HttpHostErrorKind::Cancelled,
                "localnet request was cancelled",
            ));
        }
        if request.url.contains("getTransactions") {
            lock(&self.activity_requests).push(request.url.clone());
        }

        if request.method == HttpMethod::Post
            && let Ok(payload) = serde_json::from_slice::<Value>(&request.body)
            && payload.get("method").and_then(Value::as_str) == Some("runGetMethod")
            && let Some(address) = payload.pointer("/params/address").and_then(Value::as_str)
            && let Some(get_method) = payload.pointer("/params/method").and_then(Value::as_str)
            && let Some(body) = lock(&self.localnet)
                .wallet_get_method(address, get_method)
                .map_err(|error| host_error(HttpHostErrorKind::Other, &error))?
        {
            return Ok(HttpResponse {
                status: 200,
                headers: vec![HttpHeader {
                    name: "content-type".to_owned(),
                    value: "application/json".to_owned(),
                }],
                body: body.to_string().into_bytes(),
                final_url: request.url.clone(),
            });
        }

        let method = match request.method {
            HttpMethod::Get => Method::GET,
            HttpMethod::Post => Method::POST,
        };
        let localnet = lock(&self.localnet);
        let mut builder = localnet.client.request(method, &request.url);
        for header in &request.headers {
            builder = builder.header(&header.name, &header.value);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body.clone());
        }

        let response = builder
            .send()
            .map_err(|error| host_error(HttpHostErrorKind::ConnectionLost, &error.to_string()))?;
        let status = response.status().as_u16();
        let final_url = response.url().to_string();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value.to_str().ok().map(|value| HttpHeader {
                    name: name.as_str().to_owned(),
                    value: value.to_owned(),
                })
            })
            .collect();
        let body = response
            .bytes()
            .map_err(|error| host_error(HttpHostErrorKind::ConnectionLost, &error.to_string()))?
            .to_vec();

        if request.method == HttpMethod::Post
            && serde_json::from_slice::<Value>(&request.body)
                .ok()
                .and_then(|value| {
                    value
                        .get("method")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .as_deref()
                == Some("sendBoc")
        {
            let encoded = serde_json::from_slice::<Value>(&request.body)
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/params/boc")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .ok_or_else(|| host_error(HttpHostErrorKind::Other, "sendBoc has no BOC"))?;
            let boc = STANDARD
                .decode(&encoded)
                .map_err(|error| host_error(HttpHostErrorKind::Other, &error.to_string()))?;
            let cell = TonCell::from_boc(boc)
                .map_err(|error| host_error(HttpHostErrorKind::Other, &error.to_string()))?;
            let message = Msg::<TonCell>::from_cell(&cell)
                .map_err(|error| host_error(HttpHostErrorKind::Other, &error.to_string()))?;
            let message_hash = message
                .cell_hash_normalized()
                .map(|hash| STANDARD.encode(hash.as_slice()))
                .map_err(|error| host_error(HttpHostErrorKind::Other, &error.to_string()))?;
            let (body, _) = WalletExtMsgBody::read_signed(&mut message.body.parser())
                .map_err(|error| host_error(HttpHostErrorKind::Other, &error.to_string()))?;
            let comment = decode_submitted_comment(&body)?;
            *lock(&self.submitted_message) = Some(SubmittedMessage {
                contains_state_init: message.state_init().is_some(),
                send_modes: body.msgs_modes,
                comment,
                message_hash,
            });
            *lock(&self.submitted_boc_base64) = Some(encoded);
            localnet
                .mine()
                .map_err(|error| host_error(HttpHostErrorKind::Other, &error))?;
        }

        Ok(HttpResponse {
            status,
            headers,
            body,
            final_url,
        })
    }
}

#[async_trait]
impl WalletHttpHost for LocalnetHttpHost {
    async fn execute_http(&self, request: HttpRequest) -> Result<HttpResponse, HttpHostError> {
        if request.url.contains("getAddressInformation") {
            self.wait_at_request_gate(RequestKind::Account, request.id)?;
        } else if request.url.contains("getTransactions") {
            self.wait_at_request_gate(RequestKind::Activity, request.id)?;
        } else if request
            .body
            .windows(b"runGetMethod".len())
            .any(|window| window == b"runGetMethod")
        {
            self.wait_at_request_gate(RequestKind::Seqno, request.id)?;
        } else if request.url.contains("/api/emulate/v1/emulateTrace") {
            self.wait_at_request_gate(RequestKind::Emulation, request.id)?;
        }
        self.execute(&request)
    }

    async fn cancel_http(&self, request_id: HttpRequestId) {
        lock(&self.cancelled).insert(request_id);
    }
}

struct Localnet {
    child: Child,
    base_url: String,
    client: Client,
    _directory: TempDir,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

impl Localnet {
    /// Reads one Toncenter-compatible account balance as nanograms.
    #[allow(
        dead_code,
        reason = "used by the separate TON Connect integration-test target"
    )]
    fn balance(&self, address: &str) -> Result<u128, String> {
        let (status, body) = request(
            &self.client,
            Method::GET,
            &format!(
                "{}/api/v2/getAddressInformation?address={address}",
                self.base_url
            ),
            None,
        )?;
        if !(200..300).contains(&status) {
            return Err(format!(
                "account balance request failed with HTTP {status}: {body}"
            ));
        }
        body.pointer("/result/balance")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("account response has no balance: {body}"))?
            .parse::<u128>()
            .map_err(|error| format!("account balance is invalid: {error}"))
    }

    fn account_state(&self, address: &str) -> Result<String, String> {
        let (status, body) = request(
            &self.client,
            Method::GET,
            &format!(
                "{}/api/v2/getAddressInformation?address={address}",
                self.base_url
            ),
            None,
        )?;
        if !(200..300).contains(&status) {
            return Err(format!(
                "account state request failed with HTTP {status}: {body}"
            ));
        }

        body.pointer("/result/state")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("account state response has no state: {body}"))
    }

    fn start() -> Result<Self, String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        std::fs::write(
            directory.path().join("Acton.toml"),
            concat!(
                "[package]\n",
                "name = \"wallet-engine-localnet-test\"\n",
                "description = \"Wallet engine localnet integration tests\"\n",
                "version = \"0.0.0\"\n",
                "license = \"MIT\"\n",
                "\n[localnet]\n",
            ),
        )
        .map_err(|error| error.to_string())?;

        let port = available_port()?;
        let stdout_path = directory.path().join("localnet.stdout.log");
        let stderr_path = directory.path().join("localnet.stderr.log");
        let stdout = File::create(&stdout_path).map_err(|error| error.to_string())?;
        let stderr = File::create(&stderr_path).map_err(|error| error.to_string())?;
        let binary = env::var_os("WALLET_ENGINE_ACTON_BIN")
            .map_or_else(|| PathBuf::from("acton"), PathBuf::from);

        let child = Command::new(&binary)
            .arg("--project-root")
            .arg(directory.path())
            .arg("localnet")
            .arg("start")
            .arg("--port")
            .arg(port.to_string())
            .arg("--block-interval-ms")
            .arg("50")
            .arg("--no-mining")
            .current_dir(directory.path())
            .env("NO_COLOR", "1")
            // Acton builds a multi-thread Tokio runtime for each localnet process.
            // Its default is one worker per CPU, although these tests use one small
            // HTTP client and a separate single-threaded node loop. Keep one server
            // worker so parallel scenarios do not multiply the machine's CPU count.
            .env("TOKIO_WORKER_THREADS", "1")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|error| format!("failed to start `{}`: {error}", binary.to_string_lossy()))?;
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| error.to_string())?;

        let mut localnet = Self {
            child,
            base_url: format!("http://127.0.0.1:{port}"),
            client,
            _directory: directory,
            stdout_path,
            stderr_path,
        };
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if localnet
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err(localnet.failure("Acton localnet exited before it became ready"));
            }
            if request(
                &localnet.client,
                Method::GET,
                &format!("{}/api/v2/getMasterchainInfo", localnet.base_url),
                None,
            )
            .is_ok_and(|(status, _)| (200..300).contains(&status))
            {
                localnet.install_wallet_bytecode()?;
                return Ok(localnet);
            }
            thread::sleep(Duration::from_millis(50));
        }

        Err(localnet.failure("Acton localnet did not become ready"))
    }

    /// Publishes the real wallet bytecode into the localnet blockchain config.
    ///
    /// Deployed accounts carry only the wallet trampoline, whose code jumps
    /// into the bytecode stored at config param -123 (already present on
    /// testnet). A fresh localnet config lacks that param, so every wallet
    /// execution would fail without this patch.
    fn install_wallet_bytecode(&self) -> Result<(), String> {
        const WALLET_TG_CODE_B64: &str = include_str!("wallet_tg_rev00.code");
        const BYTECODE_CONFIG_KEY: i32 = -123;

        let (status, mut state) = request(
            &self.client,
            Method::GET,
            &format!("{}/acton_dumpState", self.base_url),
            None,
        )?;
        if !(200..300).contains(&status) {
            return Err(format!("localnet state dump failed with HTTP {status}"));
        }

        let config_hash = state
            .pointer("/globals/config_boc_hash")
            .and_then(Value::as_str)
            .ok_or("localnet state dump has no config hash")?
            .to_owned();
        let config_entry = state
            .pointer_mut("/cas_entries")
            .and_then(Value::as_array_mut)
            .ok_or("localnet state dump has no cell storage")?
            .iter_mut()
            .filter_map(Value::as_array_mut)
            .find(|pair| pair.first().and_then(Value::as_str) == Some(config_hash.as_str()))
            .ok_or("localnet cell storage has no config cell")?;
        let config_boc = STANDARD
            .decode(
                config_entry[1]
                    .as_str()
                    .ok_or("localnet config cell is not base64")?,
            )
            .map_err(|error| format!("localnet config cell decoding failed: {error}"))?;

        let config_cell = TonCell::from_boc(config_boc).map_err(|error| error.to_string())?;
        let dict_codec =
            TLBHashMap::<DictKeyAdapterUint<u32>, DictValAdapterTLB<TLBRef<TonCell>>>::new(32);
        let mut params = dict_codec
            .read(&mut config_cell.parser())
            .map_err(|error| format!("localnet config parsing failed: {error}"))?;
        let wallet_code = TonCell::from_boc_base64(WALLET_TG_CODE_B64.trim())
            .map_err(|error| error.to_string())?;
        let key = u32::from_be_bytes(BYTECODE_CONFIG_KEY.to_be_bytes());
        params.insert(key, TLBRef::new(wallet_code));
        let mut patched_builder = TonCell::builder();
        dict_codec
            .write(&mut patched_builder, &params)
            .map_err(|error| format!("localnet config serialization failed: {error}"))?;
        let patched = patched_builder.build().map_err(|error| error.to_string())?;

        let patched_hash: String = patched
            .cell_hash()
            .map_err(|error| error.to_string())?
            .as_slice()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let patched_boc = STANDARD.encode(patched.to_boc().map_err(|error| error.to_string())?);
        config_entry[0] = Value::String(patched_hash.clone());
        config_entry[1] = Value::String(patched_boc);
        *state
            .pointer_mut("/globals/config_boc_hash")
            .ok_or("localnet state dump lost its config hash")? = Value::String(patched_hash);

        let (status, body) = request(
            &self.client,
            Method::POST,
            &format!("{}/acton_loadState", self.base_url),
            Some(&state),
        )?;
        if (200..300).contains(&status) && body.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            Err(format!(
                "localnet config update failed with HTTP {status}: {body}"
            ))
        }
    }

    fn fund(&self, address: &str, amount: u64) -> Result<(), String> {
        let (status, body) = request(
            &self.client,
            Method::POST,
            &format!("{}/acton_fundAccount", self.base_url),
            Some(&json!({ "address": address, "amount": amount })),
        )?;
        if (200..300).contains(&status)
            && (body.get("ok").and_then(Value::as_bool) == Some(true)
                || body.get("success").and_then(Value::as_bool) == Some(true))
        {
            Ok(())
        } else {
            Err(format!(
                "localnet funding failed with HTTP {status}: {body}"
            ))
        }
    }

    fn mine(&self) -> Result<(), String> {
        let (status, body) = request(
            &self.client,
            Method::POST,
            &format!("{}/acton_mine", self.base_url),
            Some(&json!({})),
        )?;
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(format!("localnet mining failed with HTTP {status}: {body}"))
        }
    }

    fn increase_time(&self, seconds: u64) -> Result<(), String> {
        let (status, body) = request(
            &self.client,
            Method::POST,
            &format!("{}/acton_increaseTime", self.base_url),
            Some(&json!({ "seconds": seconds })),
        )?;
        if (200..300).contains(&status)
            && (body.get("ok").and_then(Value::as_bool) == Some(true)
                || body.get("success").and_then(Value::as_bool) == Some(true))
        {
            Ok(())
        } else {
            Err(format!(
                "localnet time increase failed with HTTP {status}: {body}"
            ))
        }
    }

    fn wait_for_state(
        &self,
        address: &str,
        expected_state: &str,
        expected_seqno: Option<u32>,
    ) -> Result<(), String> {
        let deadline = Instant::now() + CONFIRMATION_TIMEOUT;
        let mut last_state = None;
        let mut last_seqno = None;

        while Instant::now() < deadline {
            let account_url = format!(
                "{}/api/v2/getAddressInformation?address={address}",
                self.base_url
            );
            if let Ok((status, body)) = request(&self.client, Method::GET, &account_url, None)
                && (200..300).contains(&status)
            {
                last_state = body
                    .pointer("/result/state")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }

            if last_state.as_deref() == Some(expected_state) {
                if let Some(expected_seqno) = expected_seqno {
                    last_seqno = self.seqno(address).ok();
                    if last_seqno == Some(expected_seqno) {
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            }
            thread::sleep(Duration::from_millis(50));
        }

        Err(format!(
            concat!(
                "wallet did not reach state={}, seqno={:?}; ",
                "last state={:?}, seqno={:?}\n",
                "transactions: {}\nlocalnet stdout:\n{}\nlocalnet stderr:\n{}",
            ),
            expected_state,
            expected_seqno,
            last_state,
            last_seqno,
            self.transactions(address),
            read_log(&self.stdout_path),
            read_log(&self.stderr_path),
        ))
    }

    fn transactions(&self, address: &str) -> String {
        let url = format!(
            "{}/api/v2/getTransactions?address={address}&limit=10",
            self.base_url
        );
        request(&self.client, Method::GET, &url, None).map_or_else(
            |error| format!("request failed: {error}"),
            |(status, body)| format!("HTTP {status}: {body}"),
        )
    }

    /// Serves a wallet get method from the account's storage cell.
    ///
    /// Localnet's `runGetMethod` executes the on-account trampoline without
    /// the chain-state config, so `CONFIGOPTPARAM -123` fails there even
    /// though transactions see the patched config. Real networks run get
    /// methods against the full masterchain config, so tests answer the
    /// wallet's storage-backed get methods locally instead. Returns `None`
    /// for other methods or accounts without parsable wallet storage.
    fn wallet_get_method(&self, address: &str, method: &str) -> Result<Option<Value>, String> {
        if !matches!(method, "seqno" | "get_subwallet_id" | "get_public_key") {
            return Ok(None);
        }

        let (status, body) = request(
            &self.client,
            Method::GET,
            &format!(
                "{}/api/v2/getAddressInformation?address={address}",
                self.base_url
            ),
            None,
        )?;
        if !(200..300).contains(&status) {
            return Err(format!(
                "account data request failed with HTTP {status}: {body}"
            ));
        }
        let Some(data) = body.pointer("/result/data").and_then(Value::as_str) else {
            return Ok(None);
        };
        let Ok(data) = STANDARD.decode(data) else {
            return Ok(None);
        };
        let Ok(storage) = WalletData::from_boc(data) else {
            return Ok(None);
        };

        let value = match method {
            "seqno" => format!("0x{:x}", storage.seqno),
            "get_subwallet_id" => format!("0x{:x}", storage.wallet_id),
            _ => {
                let mut encoded = String::with_capacity(66);
                encoded.push_str("0x");
                for byte in storage.public_key.as_slice() {
                    encoded.push_str(&format!("{byte:02x}"));
                }
                encoded
            }
        };
        Ok(Some(json!({
            "ok": true,
            "result": {
                "@type": "smc.runResult",
                "exit_code": 0,
                "gas_used": 0,
                "stack": [["num", value]]
            }
        })))
    }

    fn seqno(&self, address: &str) -> Result<u32, String> {
        let encoded = self.get_method_number(address, "seqno")?;
        u32::from_str_radix(encoded.trim_start_matches("0x"), 16).map_err(|error| error.to_string())
    }

    fn get_method_number(&self, address: &str, method: &str) -> Result<String, String> {
        let body = self
            .wallet_get_method(address, method)?
            .ok_or_else(|| format!("{method} is not a wallet storage get method"))?;
        body.pointer("/result/stack/0/1")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{method} response has no numeric stack value: {body}"))
    }

    fn failure(&mut self, message: &str) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        format!(
            "{message}\nstdout:\n{}\nstderr:\n{}",
            read_log(&self.stdout_path),
            read_log(&self.stderr_path)
        )
    }
}

impl Drop for Localnet {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn available_port() -> Result<u16, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| error.to_string())?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| error.to_string())
}

fn request(
    client: &Client,
    method: Method,
    url: &str,
    body: Option<&Value>,
) -> Result<(u16, Value), String> {
    let mut request = client.request(method, url);
    if let Some(body) = body {
        request = request.json(body);
    }

    let response = request.send().map_err(|error| error.to_string())?;
    let status = response.status().as_u16();
    let text = response.text().map_err(|error| error.to_string())?;
    let body = serde_json::from_str(&text)
        .map_err(|error| format!("HTTP {status} returned invalid JSON: {error}; body: {text}"))?;
    Ok((status, body))
}

fn read_log(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| format!("<failed to read log: {error}>"))
}

fn host_error(kind: HttpHostErrorKind, diagnostic: &str) -> HttpHostError {
    HttpHostError::Failed {
        kind,
        diagnostic: diagnostic.to_owned(),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn normalize_u256_hex(value: &str) -> Result<String, String> {
    let digits = value.strip_prefix("0x").unwrap_or(value);
    if digits.is_empty()
        || digits.len() > 64
        || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!(
            "get_public_key returned an invalid uint256: {value}"
        ));
    }
    Ok(format!("{:0>64}", digits.to_ascii_lowercase()))
}
