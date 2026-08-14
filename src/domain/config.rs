//! Network, provider, credential, and wallet-client configuration.

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

/// An opaque reference to an HTTP credential stored by the host.
///
/// The value identifies a credential. It never contains the credential itself.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CredentialRef {
    /// The host-defined lookup key for the credential.
    pub value: String,
}

/// Configures the Toncenter endpoint and its optional host-owned credential.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// The HTTPS base URL for Toncenter API v2 requests.
    pub toncenter_base_url: String,
    /// The optional reference that the HTTP host resolves before a request.
    pub toncenter_credential: Option<CredentialRef>,
    /// Normalized HTTPS origin allowed to receive `toncenter_credential`.
    /// It includes the effective port, for example
    /// `https://testnet.toncenter.com:443`.
    pub toncenter_credential_origin: Option<String>,
}

impl ProviderConfig {
    /// Returns the standard Toncenter configuration for `network`.
    ///
    /// The credential origin includes the effective HTTPS port. The host must
    /// compare this origin exactly before it adds the credential.
    #[must_use]
    pub fn standard(network: Network, credential: Option<CredentialRef>) -> Self {
        let toncenter_base_url = match network {
            Network::Mainnet => "https://toncenter.com/api/v2",
            Network::Testnet => "https://testnet.toncenter.com/api/v2",
        };
        let toncenter_credential_origin = credential.as_ref().map(|_| match network {
            Network::Mainnet => "https://toncenter.com:443".to_owned(),
            Network::Testnet => "https://testnet.toncenter.com:443".to_owned(),
        });
        Self {
            toncenter_base_url: toncenter_base_url.to_owned(),
            toncenter_credential: credential,
            toncenter_credential_origin,
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
    /// The network for the address and all provider requests.
    pub network: Network,
    /// Lifetime of a newly signed external message, in seconds.
    ///
    /// The engine adds this value to the synchronization timestamp returned by
    /// the fresh account-state request. It does not use the host device clock.
    /// A short value can expire before the network includes the message. A long
    /// value extends the period in which the signed message can be submitted.
    pub send_validity_seconds: u32,
    /// The provider endpoints and credential references.
    pub providers: ProviderConfig,
}
