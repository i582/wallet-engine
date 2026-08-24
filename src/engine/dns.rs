//! TON DNS wallet-address resolution through the configured chain provider.

use serde::Deserialize;
use serde_json::Value;

use super::WalletClient;
use super::provider::{decode_envelope, invalid_response};
use super::send_http::build_json_rpc_request;
use super::state::{OperationFamily, ensure_running};
use crate::domain::bounded_diagnostic;
use crate::{
    Base64Hash, DomainError, HttpRequest, HttpRequestId, Network, TonAddressString,
    WalletClientConfig, WalletClientError,
};

const DNS_RECURSION_TTL: u8 = 10;
const MAX_DNS_NAME_BYTES: usize = 126;
const WALLET_CATEGORY_HASH: &str = "6NRAUIc9uoZap8Fwq0zOZNkIOaNNz9bPcdFOAgVEOxs=";

#[derive(Deserialize)]
struct RawDnsResolved {
    #[serde(rename = "@type")]
    kind: String,
    entries: Vec<RawDnsEntry>,
}

#[derive(Deserialize)]
struct RawDnsEntry {
    category: String,
    entry: Value,
}

#[uniffi::export]
impl WalletClient {
    /// Resolves the standard TON DNS `wallet` record for a `.ton` name.
    ///
    /// The operation is read-only and never requests protected wallet secrets.
    /// It uses the configured or network-default root resolver, then asks the
    /// configured provider to perform bounded DNS recursion.
    pub async fn resolve_dns(
        &self,
        name: String,
    ) -> Result<Option<TonAddressString>, WalletClientError> {
        let name = normalize_dns_name(&name).map_err(dns_error)?;
        let (generation, network, resolve_request) = {
            let mut state = self.lock()?;
            ensure_running(&state)?;
            if state.active_send.is_some() || state.active_resolution.is_some() {
                return Err(WalletClientError::SendAlreadyInProgress);
            }
            state.resolution_generation = state
                .resolution_generation
                .checked_add(1)
                .ok_or(WalletClientError::IdentifierExhausted)?;
            let generation = state.resolution_generation;
            let config = state.config.clone();
            let root = config.providers.effective_dns_root_address(config.network);
            let resolve_request =
                build_dns_resolve_request(&config, state.allocate_request_id()?, root, &name)?;
            state.active_resolution = Some((generation, Vec::new()));
            (generation, config.network, resolve_request)
        };

        let resolution_body = self
            .execute_dns_request(generation, &resolve_request)
            .await?;
        let address = parse_dns_wallet_address(&resolution_body, network)
            .map_err(|error| self.fail_dns(generation, error.developer_message))?;
        self.complete_dns_operation(generation)?;
        Ok(address)
    }
}

impl WalletClient {
    async fn execute_dns_request(
        &self,
        generation: u64,
        request: &HttpRequest,
    ) -> Result<Vec<u8>, WalletClientError> {
        match self
            .execute_tracked_standalone_resolution_request(generation, request)
            .await
        {
            Ok(Ok(body)) => Ok(body),
            Ok(Err(error)) => Err(self.fail_dns(generation, error.developer_message)),
            Err(error) => {
                self.discard_dns_operation(generation);
                Err(error)
            }
        }
    }

    fn complete_dns_operation(&self, generation: u64) -> Result<(), WalletClientError> {
        let mut state = self.lock()?;
        if !state.is_current(OperationFamily::Resolution, generation) {
            return Err(WalletClientError::StateUnavailable);
        }
        state.active_resolution = None;
        Ok(())
    }

    fn discard_dns_operation(&self, generation: u64) {
        if let Ok(mut state) = self.lock()
            && state.is_current(OperationFamily::Resolution, generation)
        {
            state.active_resolution = None;
        }
    }

    fn fail_dns(&self, generation: u64, message: impl AsRef<str>) -> WalletClientError {
        self.discard_dns_operation(generation);
        dns_error(message)
    }
}

fn build_dns_resolve_request(
    config: &WalletClientConfig,
    id: HttpRequestId,
    root: &str,
    name: &str,
) -> Result<HttpRequest, WalletClientError> {
    build_json_rpc_request(
        config,
        id,
        "dnsResolve",
        &serde_json::json!({
            "address": root,
            "name": name,
            "ttl": DNS_RECURSION_TTL,
        }),
    )
}

fn normalize_dns_name(name: &str) -> Result<String, &'static str> {
    if name.is_empty() || name.len() > MAX_DNS_NAME_BYTES {
        return Err("TON DNS name must contain 1 to 126 ASCII bytes");
    }
    if !name.is_ascii() || name.bytes().any(|byte| byte <= b' ' || byte == 0x7f) {
        return Err("TON DNS name must contain printable ASCII without spaces");
    }

    let normalized = name.to_ascii_lowercase();
    let mut labels = normalized.split('.');
    let Some(first) = labels.next() else {
        return Err("TON DNS name is empty");
    };
    if first.is_empty() {
        return Err("TON DNS labels must not be empty");
    }
    let remaining = labels.collect::<Vec<_>>();
    if remaining.is_empty()
        || remaining.iter().any(|label| label.is_empty())
        || remaining.last().copied() != Some("ton")
    {
        return Err("TON DNS name must be a valid .ton domain");
    }
    Ok(normalized)
}

fn parse_dns_wallet_address(
    body: &[u8],
    network: Network,
) -> Result<Option<TonAddressString>, DomainError> {
    let result: RawDnsResolved = decode_envelope(body)?;
    if result.kind != "dns.resolved" {
        return Err(invalid_response("invalid TON DNS result type"));
    }

    let mut address = None;
    for entry in result.entries {
        let category = Base64Hash::try_from(entry.category.as_str())
            .map_err(|_| invalid_response("invalid TON DNS category hash"))?;
        if category.as_str() != WALLET_CATEGORY_HASH {
            continue;
        }
        if address.is_some() {
            return Err(invalid_response("duplicate TON DNS wallet records"));
        }
        if entry.entry.pointer("/@type").and_then(Value::as_str) != Some("dns.entryDataSmcAddress")
        {
            return Err(invalid_response(
                "TON DNS wallet record has an invalid type",
            ));
        }
        let encoded = entry
            .entry
            .pointer("/smc_address/account_address")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid_response("TON DNS wallet record has no address"))?;
        let parsed = TonAddressString::try_from(encoded)
            .map_err(|_| invalid_response("TON DNS wallet record has an invalid address"))?;
        address = Some(TonAddressString::from_address(parsed.as_address(), network));
    }
    Ok(address)
}

fn dns_error(message: impl AsRef<str>) -> WalletClientError {
    WalletClientError::DnsResolutionUnavailable {
        diagnostic: bounded_diagnostic(message),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures::executor::block_on;
    use serde_json::json;

    use super::*;
    use crate::wallet::crypto::derive_wallet;
    use crate::{
        HttpHostError, HttpResponse, JournalCompareExchange, JournalCompareExchangeResult,
        JournalHostError, JournalKey, JournalRecord, NonEmptyString, ProtectedSecretHostError,
        ProtectedSecretRead, ProtectedSecretRef, ProtectedSecretStore, ProviderConfig,
        WalletHttpHost, WalletPlatformHost,
    };

    const ROOT_RAW: &str = "-1:e56754f83426f69b09267bd876ac97c44821345b7e266bd956a7bfbfb98df35c";
    const WALLET_ADDRESS: &str = "EQCD39VS5jcptHL8vMjEXrzGaRcCVYto7HUn4bpAOg8xqB2N";
    const MNEMONIC: &str = "notice tortoise soup strong gun divide offer process salon siren general carry clump left year void clutch tool case burden fix income champion lounge";

    #[derive(Default)]
    struct DnsHost {
        requests: Mutex<Vec<Value>>,
    }

    #[async_trait::async_trait]
    impl WalletHttpHost for DnsHost {
        async fn execute_http(&self, request: HttpRequest) -> Result<HttpResponse, HttpHostError> {
            let payload: Value =
                serde_json::from_slice(&request.body).expect("request JSON must parse");
            self.requests
                .lock()
                .expect("request lock")
                .push(payload.clone());
            let body = match payload.get("method").and_then(Value::as_str) {
                Some("dnsResolve") => json!({
                    "ok": true,
                    "result": {
                        "@type": "dns.resolved",
                        "entries": [{
                            "category": WALLET_CATEGORY_HASH,
                            "entry": {
                                "@type": "dns.entryDataSmcAddress",
                                "smc_address": {
                                    "@type": "accountAddress",
                                    "account_address": WALLET_ADDRESS
                                }
                            }
                        }]
                    }
                }),
                method => panic!("unexpected provider method {method:?}"),
            };
            Ok(HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: serde_json::to_vec(&body).expect("response JSON must encode"),
                final_url: request.url,
            })
        }

        async fn cancel_http(&self, _request_id: HttpRequestId) {}
    }

    struct NoSecretsHost;

    #[async_trait::async_trait]
    impl WalletPlatformHost for NoSecretsHost {
        async fn read_protected_secret(
            &self,
            _request: ProtectedSecretRead,
        ) -> Result<Vec<u8>, ProtectedSecretHostError> {
            panic!("TON DNS must not read protected secrets")
        }

        async fn store_protected_secret(
            &self,
            _request: ProtectedSecretStore,
        ) -> Result<(), ProtectedSecretHostError> {
            panic!("TON DNS must not store protected secrets")
        }

        async fn delete_protected_secret(
            &self,
            _secret_ref: ProtectedSecretRef,
        ) -> Result<(), ProtectedSecretHostError> {
            panic!("TON DNS must not delete protected secrets")
        }

        async fn load_journal(
            &self,
            _key: JournalKey,
        ) -> Result<Option<JournalRecord>, JournalHostError> {
            panic!("TON DNS must not read the send journal")
        }

        async fn compare_exchange_journal(
            &self,
            _mutation: JournalCompareExchange,
        ) -> Result<JournalCompareExchangeResult, JournalHostError> {
            panic!("TON DNS must not change the send journal")
        }
    }

    #[test]
    fn normalizes_valid_dot_ton_names_and_rejects_ambiguous_input() {
        assert_eq!(
            normalize_dns_name("Foundation.TON"),
            Ok("foundation.ton".to_owned())
        );
        assert_eq!(
            normalize_dns_name("wallet.subdomain.ton"),
            Ok("wallet.subdomain.ton".to_owned())
        );

        let too_long = format!("{}.ton", "a".repeat(123));
        for invalid in [
            "",
            "ton",
            ".ton",
            "name..ton",
            "name.ton.",
            "name.com",
            "name .ton",
            "имя.ton",
            too_long.as_str(),
        ] {
            assert!(normalize_dns_name(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn parses_only_the_wallet_category_and_normalizes_network_flags() {
        let body = serde_json::to_vec(&json!({
            "ok": true,
            "result": {
                "@type": "dns.resolved",
                "entries": [
                    {
                        "category": "SaJfn+76/+ytD80wxQ3JMxz/i1Xs5T3vYoXAnhfm9dc=",
                        "entry": { "@type": "dns.entryDataStorageAddress", "bag_id": "00" }
                    },
                    {
                        "category": WALLET_CATEGORY_HASH,
                        "entry": {
                            "@type": "dns.entryDataSmcAddress",
                            "smc_address": {
                                "@type": "accountAddress",
                                "account_address": WALLET_ADDRESS
                            }
                        }
                    }
                ]
            }
        }))
        .expect("fixture encodes");

        let mainnet = parse_dns_wallet_address(&body, Network::Mainnet)
            .expect("response parses")
            .expect("wallet record exists");
        let testnet = parse_dns_wallet_address(&body, Network::Testnet)
            .expect("response parses")
            .expect("wallet record exists");
        assert_eq!(mainnet.as_address(), testnet.as_address());
        assert_ne!(mainnet.as_str(), testnet.as_str());

        let empty = serde_json::to_vec(&json!({
            "ok": true,
            "result": { "@type": "dns.resolved", "entries": [] }
        }))
        .expect("fixture encodes");
        assert!(
            parse_dns_wallet_address(&empty, Network::Mainnet)
                .expect("empty resolution parses")
                .is_none()
        );
    }

    #[test]
    fn rejects_ambiguous_or_malformed_wallet_records() {
        let wallet = json!({
            "category": WALLET_CATEGORY_HASH,
            "entry": {
                "@type": "dns.entryDataSmcAddress",
                "smc_address": { "account_address": WALLET_ADDRESS }
            }
        });
        let duplicate = serde_json::to_vec(&json!({
            "ok": true,
            "result": {
                "@type": "dns.resolved",
                "entries": [wallet.clone(), wallet]
            }
        }))
        .expect("fixture encodes");
        assert!(parse_dns_wallet_address(&duplicate, Network::Mainnet).is_err());

        for entry in [
            json!({ "category": "not-base64", "entry": {} }),
            json!({
                "category": WALLET_CATEGORY_HASH,
                "entry": { "@type": "dns.entryDataAdnlAddress" }
            }),
            json!({
                "category": WALLET_CATEGORY_HASH,
                "entry": {
                    "@type": "dns.entryDataSmcAddress",
                    "smc_address": { "account_address": "not-an-address" }
                }
            }),
        ] {
            let body = serde_json::to_vec(&json!({
                "ok": true,
                "result": { "@type": "dns.resolved", "entries": [entry] }
            }))
            .expect("fixture encodes");
            assert!(parse_dns_wallet_address(&body, Network::Mainnet).is_err());
        }
    }

    #[test]
    fn public_workflow_uses_the_configured_root_without_secrets() {
        let wallet = derive_wallet(MNEMONIC, Network::Testnet).expect("wallet derives");
        let config = WalletClientConfig {
            record_id: NonEmptyString::try_from("dns-test").expect("record ID"),
            address: TonAddressString::from_address(&wallet.address, Network::Testnet),
            public_key: wallet.key_pair.public_key.to_vec(),
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig {
                toncenter_base_url: "https://provider.example".to_owned(),
                dns_root_address: Some(
                    TonAddressString::try_from(ROOT_RAW).expect("DNS root address"),
                ),
                request_timeout_ms: 15_000,
            },
        };
        let http = Arc::new(DnsHost::default());
        let client = WalletClient::new(config, http.clone(), Arc::new(NoSecretsHost))
            .expect("client builds");

        let address = block_on(client.resolve_dns("Foundation.TON".to_owned()))
            .expect("DNS resolves")
            .expect("wallet record exists");
        let expected = TonAddressString::try_from(WALLET_ADDRESS).expect("fixture address");
        assert_eq!(address.as_address(), expected.as_address());
        assert!(address.as_str().starts_with('k') || address.as_str().starts_with('0'));

        let requests = http.requests.lock().expect("request lock");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["method"], "dnsResolve");
        assert_eq!(requests[0]["params"]["address"], ROOT_RAW);
        assert_eq!(requests[0]["params"]["name"], "foundation.ton");
        assert_eq!(requests[0]["params"]["ttl"], DNS_RECURSION_TTL);
    }
}
