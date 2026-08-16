use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;

use crate::{
    Base64Value, Ed25519PublicKey, Ed25519Signature, HttpsUrl, NetworkId, NonEmptyVec,
    RawAccountAddress, SignatureDomain, SigningError, Uint64String, WalletResponse,
    rpc::numeric_enum_serde, ton_proof_signing_hash, verify_signature,
};

/// Request for the connected wallet address and optional target network hint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TonAddressItem {
    /// Optional desired network global ID.
    pub network: Option<NetworkId>,
}

/// Request for a wallet ownership proof bound to a dApp payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TonProofItem {
    /// Opaque application-provided challenge.
    pub payload: String,
}

/// Data item requested during the initial connect handshake.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "name", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConnectItem {
    /// Connected account address information.
    TonAddr {
        /// Optional desired network global ID.
        #[serde(skip_serializing_if = "Option::is_none")]
        network: Option<NetworkId>,
    },
    /// Wallet ownership proof.
    TonProof {
        /// Opaque application-provided challenge.
        payload: String,
    },
}

impl From<TonAddressItem> for ConnectItem {
    fn from(item: TonAddressItem) -> Self {
        Self::TonAddr {
            network: item.network,
        }
    }
}

impl From<TonProofItem> for ConnectItem {
    fn from(item: TonProofItem) -> Self {
        Self::TonProof {
            payload: item.payload,
        }
    }
}

/// Initial plaintext request carried by a TON Connect deep link.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectRequest {
    /// URL of the dApp's `tonconnect-manifest.json`.
    pub manifest_url: HttpsUrl,
    /// One or more requested account data items.
    pub items: NonEmptyVec<ConnectItem>,
}

/// Wallet runtime reported in `DeviceInfo`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DevicePlatform {
    /// iPhone wallet.
    Iphone,
    /// iPad wallet.
    Ipad,
    /// Android wallet.
    Android,
    /// Windows wallet.
    Windows,
    /// macOS wallet.
    Mac,
    /// Linux wallet.
    Linux,
    /// Browser wallet or extension.
    Browser,
}

/// Structured transfer kinds accepted by a wallet.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StructuredItemType {
    /// Native TON transfer.
    Ton,
    /// Jetton transfer.
    Jetton,
    /// NFT transfer.
    Nft,
}

/// Data variants accepted by the `signData` method.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SignDataType {
    /// UTF-8 text.
    Text,
    /// Opaque bytes.
    Binary,
    /// TL-B-described cell.
    Cell,
}

/// Advertised `SendTransaction` limits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendTransactionFeature {
    /// Maximum number of outgoing messages accepted in one request.
    pub max_messages: u32,
    /// Whether TEP-92 extra currencies are supported.
    pub extra_currency_supported: Option<bool>,
    /// Structured item kinds accepted by the wallet.
    pub item_types: Option<Vec<StructuredItemType>>,
}

/// Advertised `SignMessage` limits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignMessageFeature {
    /// Maximum number of outgoing messages accepted in one request.
    pub max_messages: u32,
    /// Whether TEP-92 extra currencies are supported.
    pub extra_currency_supported: Option<bool>,
    /// Structured item kinds accepted by the wallet.
    pub item_types: Option<Vec<StructuredItemType>>,
}

/// Advertised `SignData` variants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignDataFeature {
    /// Payload variants accepted by the wallet.
    pub types: Vec<SignDataType>,
}

/// Wallet capability advertised in `DeviceInfo.features`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Feature {
    /// Deprecated protocol-v2 string form retained for compatibility.
    LegacySendTransaction,
    /// Transaction signing and broadcasting.
    SendTransaction(SendTransactionFeature),
    /// Arbitrary data signing.
    SignData(SignDataFeature),
    /// Message signing without broadcasting.
    SignMessage(SignMessageFeature),
    /// Connect-link embedded requests.
    EmbeddedRequest,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "name", deny_unknown_fields)]
enum DetailedFeature {
    #[serde(rename = "SendTransaction")]
    SendTransaction {
        #[serde(rename = "maxMessages")]
        max_messages: u32,
        #[serde(
            rename = "extraCurrencySupported",
            skip_serializing_if = "Option::is_none"
        )]
        extra_currency_supported: Option<bool>,
        #[serde(rename = "itemTypes", skip_serializing_if = "Option::is_none")]
        item_types: Option<Vec<StructuredItemType>>,
    },
    #[serde(rename = "SignData")]
    SignData { types: Vec<SignDataType> },
    #[serde(rename = "SignMessage")]
    SignMessage {
        #[serde(rename = "maxMessages")]
        max_messages: u32,
        #[serde(
            rename = "extraCurrencySupported",
            skip_serializing_if = "Option::is_none"
        )]
        extra_currency_supported: Option<bool>,
        #[serde(rename = "itemTypes", skip_serializing_if = "Option::is_none")]
        item_types: Option<Vec<StructuredItemType>>,
    },
    #[serde(rename = "EmbeddedRequest")]
    EmbeddedRequest,
}

impl Serialize for Feature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::LegacySendTransaction => serializer.serialize_str("SendTransaction"),
            Self::SendTransaction(feature) => DetailedFeature::SendTransaction {
                max_messages: feature.max_messages,
                extra_currency_supported: feature.extra_currency_supported,
                item_types: feature.item_types.clone(),
            }
            .serialize(serializer),
            Self::SignData(feature) => DetailedFeature::SignData {
                types: feature.types.clone(),
            }
            .serialize(serializer),
            Self::SignMessage(feature) => DetailedFeature::SignMessage {
                max_messages: feature.max_messages,
                extra_currency_supported: feature.extra_currency_supported,
                item_types: feature.item_types.clone(),
            }
            .serialize(serializer),
            Self::EmbeddedRequest => DetailedFeature::EmbeddedRequest.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Feature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        if let Value::String(name) = &value {
            return if name == "SendTransaction" {
                Ok(Self::LegacySendTransaction)
            } else {
                Err(de::Error::custom("unsupported legacy TON Connect feature"))
            };
        }

        match serde_json::from_value::<DetailedFeature>(value).map_err(de::Error::custom)? {
            DetailedFeature::SendTransaction {
                max_messages,
                extra_currency_supported,
                item_types,
            } => Ok(Self::SendTransaction(SendTransactionFeature {
                max_messages,
                extra_currency_supported,
                item_types,
            })),
            DetailedFeature::SignData { types } => Ok(Self::SignData(SignDataFeature { types })),
            DetailedFeature::SignMessage {
                max_messages,
                extra_currency_supported,
                item_types,
            } => Ok(Self::SignMessage(SignMessageFeature {
                max_messages,
                extra_currency_supported,
                item_types,
            })),
            DetailedFeature::EmbeddedRequest => Ok(Self::EmbeddedRequest),
        }
    }
}

/// Wallet self-description returned in a successful connect event.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceInfo {
    /// Wallet runtime platform.
    pub platform: DevicePlatform,
    /// Wallet identifier matching its wallets-list `app_name`.
    pub app_name: String,
    /// Wallet application version.
    pub app_version: String,
    /// Highest supported TON Connect protocol version.
    pub max_protocol_version: u32,
    /// Runtime-authoritative capability list.
    pub features: Vec<Feature>,
}

/// Connected account information returned for `ton_addr`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TonAddressItemReply {
    #[serde(rename = "name")]
    name: TonAddressReplyName,
    /// Raw TON address (`workchain:hex`).
    pub address: RawAccountAddress,
    /// Connected network global ID.
    pub network: NetworkId,
    /// Base64 wallet `StateInit` `BoC`.
    #[serde(rename = "walletStateInit")]
    pub wallet_state_init: Base64Value,
    /// Untrusted wallet public key as hex without `0x`.
    #[serde(rename = "publicKey")]
    pub public_key: Ed25519PublicKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
enum TonAddressReplyName {
    #[serde(rename = "ton_addr")]
    TonAddr,
}

impl TonAddressItemReply {
    /// Creates a canonical `ton_addr` reply.
    #[must_use]
    pub fn new(
        address: RawAccountAddress,
        network: NetworkId,
        wallet_state_init: Base64Value,
        public_key: Ed25519PublicKey,
    ) -> Self {
        Self {
            name: TonAddressReplyName::TonAddr,
            address,
            network,
            wallet_state_init,
            public_key,
        }
    }
}

/// dApp domain bound into a `ton_proof` signature.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TonProofDomain {
    /// UTF-8 byte length of `value`.
    length_bytes: u32,
    /// Domain name without scheme or encoding.
    value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTonProofDomain {
    length_bytes: u32,
    value: String,
}

impl TonProofDomain {
    /// Creates a domain whose declared wire length matches its UTF-8 bytes.
    pub fn new(value: String) -> Result<Self, SigningError> {
        let length_bytes = u32::try_from(value.len()).map_err(|_| SigningError::LengthOverflow)?;
        Ok(Self {
            length_bytes,
            value,
        })
    }

    /// Returns the validated UTF-8 byte length carried on the wire.
    #[must_use]
    pub const fn length_bytes(&self) -> u32 {
        self.length_bytes
    }

    /// Returns the exact domain name bound into the signature.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl<'de> Deserialize<'de> for TonProofDomain {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawTonProofDomain::deserialize(deserializer)?;
        let actual = u32::try_from(raw.value.len())
            .map_err(|_| de::Error::custom("ton_proof domain exceeds uint32 length"))?;
        if actual != raw.length_bytes {
            return Err(de::Error::custom(
                "ton_proof domain lengthBytes does not match UTF-8 byte length",
            ));
        }
        Ok(Self {
            length_bytes: raw.length_bytes,
            value: raw.value,
        })
    }
}

/// Wallet ownership proof returned during connect.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TonProof {
    /// Unix signing time in seconds.
    pub timestamp: Uint64String,
    /// Bound application domain.
    pub domain: TonProofDomain,
    /// Original dApp challenge.
    pub payload: String,
    /// Base64 Ed25519 signature.
    pub signature: Ed25519Signature,
}

impl TonProof {
    /// Reconstructs the digest bound by this proof.
    pub fn signing_hash(&self, address: &RawAccountAddress) -> Result<[u8; 32], SigningError> {
        ton_proof_signing_hash(
            address,
            self.domain.value(),
            self.timestamp.get(),
            &self.payload,
        )
    }

    /// Verifies the proof signature with the connected account's trusted key.
    pub fn verify(
        &self,
        address: &RawAccountAddress,
        public_key: &Ed25519PublicKey,
        network: &NetworkId,
    ) -> Result<bool, SigningError> {
        verify_signature(
            &self.signing_hash(address)?,
            &self.signature,
            public_key,
            SignatureDomain::for_network(network)?,
        )
    }
}

/// Successful `ton_proof` item reply.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TonProofItemReply {
    #[serde(rename = "name")]
    name: TonProofReplyName,
    /// Signed proof.
    pub proof: TonProof,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
enum TonProofReplyName {
    #[serde(rename = "ton_proof")]
    TonProof,
}

impl TonProofItemReply {
    /// Creates a canonical successful `ton_proof` reply.
    #[must_use]
    pub const fn new(proof: TonProof) -> Self {
        Self {
            name: TonProofReplyName::TonProof,
            proof,
        }
    }
}

/// Per-connect-item protocol error catalogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum ConnectItemErrorCode {
    /// Unexpected wallet-side failure.
    Unknown = 0,
    /// Wallet does not implement the requested item.
    MethodNotSupported = 400,
}

numeric_enum_serde!(ConnectItemErrorCode {
    Unknown = 0,
    MethodNotSupported = 400,
});

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConnectItemErrorBody {
    code: ConnectItemErrorCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

/// Error reply for a specific requested connect item.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectItemError {
    name: String,
    error: ConnectItemErrorBody,
}

impl ConnectItemError {
    /// Creates a per-item failure preserving the requested item name.
    #[must_use]
    pub fn new(name: String, code: ConnectItemErrorCode, message: Option<String>) -> Self {
        Self {
            name,
            error: ConnectItemErrorBody { code, message },
        }
    }

    /// Returns the requested item name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the protocol error code.
    #[must_use]
    pub const fn code(&self) -> ConnectItemErrorCode {
        self.error.code
    }

    /// Returns the optional wallet diagnostic.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.error.message.as_deref()
    }
}

/// Reply corresponding to one requested connect item.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ConnectItemReply {
    /// Connected account information.
    TonAddress(TonAddressItemReply),
    /// Successful ownership proof.
    TonProof(TonProofItemReply),
    /// Per-item failure, including unsupported future item names.
    Error(ConnectItemError),
}

/// Payload of a successful connect event.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ConnectEventPayload {
    /// Replies to the requested connect items.
    pub items: Vec<ConnectItemReply>,
    /// Wallet runtime and capabilities.
    pub device: DeviceInfo,
}

/// Connect-event error catalogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum ConnectEventErrorCode {
    /// Unexpected wallet-side failure.
    Unknown = 0,
    /// Malformed connect request.
    BadRequest = 1,
    /// Manifest could not be fetched.
    ManifestNotFound = 2,
    /// Manifest JSON or schema is invalid.
    ManifestContent = 3,
    /// Unknown or revoked app session.
    UnknownApp = 100,
    /// User declined the connection.
    UserDeclined = 300,
    /// Requested method is unsupported.
    MethodNotSupported = 400,
}

numeric_enum_serde!(ConnectEventErrorCode {
    Unknown = 0,
    BadRequest = 1,
    ManifestNotFound = 2,
    ManifestContent = 3,
    UnknownApp = 100,
    UserDeclined = 300,
    MethodNotSupported = 400,
});

/// Error details carried by a `connect_error` event.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectEventError {
    /// Protocol error code.
    pub code: ConnectEventErrorCode,
    /// Human-readable wallet diagnostic.
    pub message: String,
}

/// Wallet-initiated event sent to the dApp.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConnectEvent {
    /// Successful connection.
    Connect {
        /// Monotonic wallet event identifier.
        id: u64,
        /// Requested item replies and wallet capabilities.
        payload: ConnectEventPayload,
        /// Optional response to an embedded request.
        #[serde(skip_serializing_if = "Option::is_none")]
        response: Option<WalletResponse>,
    },
    /// Failed connection.
    ConnectError {
        /// Monotonic wallet event identifier.
        id: u64,
        /// Protocol error details.
        payload: ConnectEventError,
    },
    /// Wallet-initiated session termination.
    Disconnect {
        /// Monotonic wallet event identifier.
        id: u64,
        /// Reserved object; currently empty.
        payload: BTreeMap<String, Value>,
    },
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer as _, SigningKey};

    use super::*;

    #[test]
    fn connect_request_uses_canonical_item_discriminators() {
        let request = r#"{
            "manifestUrl":"https://example.com/tonconnect-manifest.json",
            "items":[{"name":"ton_addr","network":"-239"},{"name":"ton_proof","payload":"nonce"}]
        }"#;
        let decoded = serde_json::from_str::<ConnectRequest>(request);
        assert!(decoded.is_ok());
        assert!(
            decoded
                .and_then(|value| serde_json::to_string(&value))
                .is_ok()
        );
    }

    #[test]
    fn feature_supports_legacy_and_current_wire_forms() {
        assert!(matches!(
            serde_json::from_str::<Feature>(r#""SendTransaction""#),
            Ok(Feature::LegacySendTransaction)
        ));
        let current = r#"{"name":"SendTransaction","maxMessages":4,"extraCurrencySupported":false,"itemTypes":["ton"]}"#;
        assert!(matches!(
            serde_json::from_str::<Feature>(current),
            Ok(Feature::SendTransaction(SendTransactionFeature {
                max_messages: 4,
                ..
            }))
        ));
    }

    #[test]
    fn event_ids_are_non_negative_in_rust_and_unknown_fields_are_rejected() {
        let negative = r#"{"event":"disconnect","id":-1,"payload":{}}"#;
        let extra =
            r#"{"event":"connect_error","id":1,"payload":{"code":1,"message":"bad"},"extra":true}"#;
        assert!(serde_json::from_str::<ConnectEvent>(negative).is_err());
        assert!(serde_json::from_str::<ConnectEvent>(extra).is_err());
    }

    #[test]
    fn ton_proof_uses_string_timestamp_and_checks_domain_byte_length() -> Result<(), SigningError> {
        let proof = TonProof {
            timestamp: Uint64String::from(1_700_000_000),
            domain: TonProofDomain::new("пример.рф".to_owned())?,
            payload: "nonce".to_owned(),
            signature: Ed25519Signature::from_bytes([0_u8; 64]),
        };
        let encoded = serde_json::to_string(&proof);
        assert!(
            encoded
                .as_ref()
                .is_ok_and(|json| json.contains(r#""timestamp":"1700000000""#))
        );

        let numeric_timestamp = r#"{
            "timestamp":1700000000,
            "domain":{"lengthBytes":11,"value":"example.com"},
            "payload":"nonce",
            "signature":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="
        }"#;
        let wrong_utf8_length = r#"{
            "timestamp":"1700000000",
            "domain":{"lengthBytes":9,"value":"пример.рф"},
            "payload":"nonce",
            "signature":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="
        }"#;
        assert!(serde_json::from_str::<TonProof>(numeric_timestamp).is_err());
        assert!(serde_json::from_str::<TonProof>(wrong_utf8_length).is_err());
        Ok(())
    }

    #[test]
    fn ton_proof_wrapper_reconstructs_and_verifies_the_signed_fields()
    -> Result<(), Box<dyn std::error::Error>> {
        let address = RawAccountAddress::new(0, [0x22; 32]);
        let signing_key = SigningKey::from_bytes(&[0x33; 32]);
        let public_key = Ed25519PublicKey::from_bytes(signing_key.verifying_key().to_bytes());
        let mut proof = TonProof {
            timestamp: Uint64String::from(1_700_000_000),
            domain: TonProofDomain::new("example.com".to_owned())?,
            payload: "single-use-nonce".to_owned(),
            signature: Ed25519Signature::from_bytes([0_u8; 64]),
        };
        proof.signature = Ed25519Signature::from_bytes(
            signing_key.sign(&proof.signing_hash(&address)?).to_bytes(),
        );
        let mainnet = NetworkId::try_from("-239")?;

        assert!(proof.verify(&address, &public_key, &mainnet)?);
        proof.payload.push_str("-changed");
        assert!(!proof.verify(&address, &public_key, &mainnet)?);
        Ok(())
    }
}
