//! Network, provider, and wallet-client configuration.

use super::ProtectedSecretRef;

/// Default end-to-end timeout for one provider request.
pub const DEFAULT_PROVIDER_REQUEST_TIMEOUT_MS: u64 = 15_000;

/// Largest provider timeout accepted by the engine.
pub const MAX_PROVIDER_REQUEST_TIMEOUT_MS: u64 = 300_000;

/// Default indexer-lag margin after a signed message validity window.
pub const DEFAULT_RESOLUTION_MARGIN_SECONDS: u32 = 60;

/// Default delay between active pending-resolution attempts.
pub const DEFAULT_RESOLUTION_POLL_INTERVAL_MS: u64 = 4_000;

/// Default maximum time spent actively resolving one pending send.
pub const DEFAULT_RESOLUTION_ACTIVE_BUDGET_MS: u64 = 60_000;

/// Selects the TON network used for addresses, providers, and wallet derivation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, uniffi::Enum,
)]
#[serde(rename_all = "camelCase")]
pub enum Network {
    /// The production TON network.
    Mainnet,
    /// The public TON test network.
    Testnet,
}

/// Configures the Toncenter endpoint.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// The HTTPS base URL for a Toncenter-compatible provider.
    ///
    /// The engine appends the path for the API used by each request. An optional
    /// deployment prefix is preserved, so `https://provider.example/toncenter`
    /// produces paths below `/toncenter/api/...`.
    /// Loopback HTTP URLs are accepted for local development networks.
    pub toncenter_base_url: String,
    /// End-to-end timeout applied to every provider request, in milliseconds.
    ///
    /// The embedding HTTP host must enforce this deadline across connection,
    /// response headers, and response-body reads. Values must be between 1 and
    /// [`MAX_PROVIDER_REQUEST_TIMEOUT_MS`].
    #[serde(default = "default_provider_request_timeout_ms")]
    pub request_timeout_ms: u64,
}

const fn default_provider_request_timeout_ms() -> u64 {
    DEFAULT_PROVIDER_REQUEST_TIMEOUT_MS
}

impl ProviderConfig {
    /// Returns the standard Toncenter configuration for `network`.
    ///
    #[must_use]
    pub fn standard(network: Network) -> Self {
        let toncenter_base_url = match network {
            Network::Mainnet => "https://toncenter.com",
            Network::Testnet => "https://testnet.toncenter.com",
        };
        Self {
            toncenter_base_url: toncenter_base_url.to_owned(),
            request_timeout_ms: DEFAULT_PROVIDER_REQUEST_TIMEOUT_MS,
        }
    }
}

/// Identifies one wallet client and its provider configuration.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletClientConfig {
    /// The stable application record identifier for this wallet.
    pub record_id: String,
    /// The friendly TON address that the client reads and sends from.
    pub address: String,
    /// The raw 32-byte Ed25519 public key stored in this V5R1 wallet.
    ///
    /// This value is public metadata. The engine uses it to build a faithful
    /// fake-signed message for preflight emulation without unlocking the mnemonic.
    pub public_key: Vec<u8>,
    /// The protected mnemonic reference used for local signing.
    ///
    /// `None` configures a public-key-only wallet. Read operations and send
    /// previews remain available, while [`crate::WalletClient::send`] returns
    /// [`crate::WalletClientError::LocalSigningUnavailable`].
    #[serde(default)]
    pub local_secret_ref: Option<ProtectedSecretRef>,
    /// The network for the address and all provider requests.
    pub network: Network,
    /// Lifetime of a newly signed external message, in seconds.
    ///
    /// The engine adds this value to the synchronization timestamp returned by
    /// the fresh account-state request. It does not use the host device clock.
    /// A short value can expire before the network includes the message. A long
    /// value extends the period in which the signed message can be submitted.
    pub send_validity_seconds: u32,
    /// Additional provider-time margin before an unseen message becomes expired.
    ///
    /// This protects against declaring expiration while the transaction index is
    /// still catching up with the account state used by the resolver.
    #[serde(default = "default_resolution_margin_seconds")]
    pub resolution_margin_seconds: u32,
    /// Delay between provider lookups during active pending resolution.
    #[serde(default = "default_resolution_poll_interval_ms")]
    pub resolution_poll_interval_ms: u64,
    /// Maximum delay budget for one active resolution run.
    ///
    /// This controls UX waiting only. Terminal conclusions continue to use
    /// provider time and chain evidence rather than this local duration.
    #[serde(default = "default_resolution_active_budget_ms")]
    pub resolution_active_budget_ms: u64,
    /// The provider endpoint.
    pub providers: ProviderConfig,
}

const fn default_resolution_margin_seconds() -> u32 {
    DEFAULT_RESOLUTION_MARGIN_SECONDS
}

const fn default_resolution_poll_interval_ms() -> u64 {
    DEFAULT_RESOLUTION_POLL_INTERVAL_MS
}

const fn default_resolution_active_budget_ms() -> u64 {
    DEFAULT_RESOLUTION_ACTIVE_BUDGET_MS
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_PROVIDER_REQUEST_TIMEOUT_MS, DEFAULT_RESOLUTION_ACTIVE_BUDGET_MS,
        DEFAULT_RESOLUTION_MARGIN_SECONDS, DEFAULT_RESOLUTION_POLL_INTERVAL_MS, Network,
        ProviderConfig, WalletClientConfig,
    };

    #[test]
    fn standard_provider_matches_each_network() {
        assert_eq!(
            ProviderConfig::standard(Network::Mainnet).toncenter_base_url,
            "https://toncenter.com"
        );
        assert_eq!(
            ProviderConfig::standard(Network::Testnet).toncenter_base_url,
            "https://testnet.toncenter.com"
        );
        assert_eq!(
            ProviderConfig::standard(Network::Mainnet).request_timeout_ms,
            DEFAULT_PROVIDER_REQUEST_TIMEOUT_MS
        );
    }

    #[test]
    fn provider_json_from_before_timeout_support_uses_the_standard_deadline() {
        let provider: ProviderConfig =
            serde_json::from_str(r#"{"toncenterBaseUrl":"https://testnet.toncenter.com"}"#)
                .expect("the previous provider JSON shape must remain readable");

        assert_eq!(
            provider,
            ProviderConfig {
                toncenter_base_url: "https://testnet.toncenter.com".to_owned(),
                request_timeout_ms: DEFAULT_PROVIDER_REQUEST_TIMEOUT_MS,
            }
        );
    }

    #[test]
    fn client_json_from_before_resolution_support_uses_the_default_margin() {
        let config: WalletClientConfig = serde_json::from_str(
            r#"{
                "recordId":"record",
                "address":"0:0000000000000000000000000000000000000000000000000000000000000000",
                "publicKey":[],
                "network":"testnet",
                "sendValiditySeconds":300,
                "providers":{"toncenterBaseUrl":"https://testnet.toncenter.com"}
            }"#,
        )
        .expect("the previous wallet config JSON shape must remain readable");

        assert_eq!(
            config.resolution_margin_seconds,
            DEFAULT_RESOLUTION_MARGIN_SECONDS
        );
        assert_eq!(
            config.resolution_poll_interval_ms,
            DEFAULT_RESOLUTION_POLL_INTERVAL_MS
        );
        assert_eq!(
            config.resolution_active_budget_ms,
            DEFAULT_RESOLUTION_ACTIVE_BUDGET_MS
        );
    }
}
