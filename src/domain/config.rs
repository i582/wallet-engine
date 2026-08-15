//! Network, provider, and wallet-client configuration.

use super::ProtectedSecretRef;

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
    /// The provider endpoint.
    pub providers: ProviderConfig,
}

#[cfg(test)]
mod tests {
    use super::{Network, ProviderConfig};

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
    }
}
