use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;

use crate::{
    AccountVerificationError, Ed25519PublicKey, Ed25519Signature, EmbeddedResponse, EmptyObject,
    HttpsUrl, NetworkId, NonEmptyVec, RawAccountAddress, ResponseValidationError, SignatureDomain,
    SigningError, StandardWalletState, Uint64String, WalletStateError, WalletStateInit,
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
///
/// Unknown item names are preserved so a wallet can return the mandatory
/// per-item error `400` instead of rejecting the complete connect request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectItem {
    /// Connected account address information.
    TonAddr {
        /// Optional desired network global ID.
        network: Option<NetworkId>,
    },
    /// Wallet ownership proof.
    TonProof {
        /// Opaque application-provided challenge.
        payload: String,
    },
    /// A forward-compatible item unsupported by this protocol revision.
    Unsupported {
        /// Exact requested item name echoed in the error reply.
        name: String,
        /// Unknown item fields retained without interpretation.
        fields: BTreeMap<String, Value>,
    },
}

impl ConnectItem {
    /// Returns the exact connect-item discriminator.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::TonAddr { .. } => "ton_addr",
            Self::TonProof { .. } => "ton_proof",
            Self::Unsupported { name, .. } => name,
        }
    }

    /// Creates a forward-compatible item unknown to this crate revision.
    pub fn unsupported(
        name: String,
        fields: BTreeMap<String, Value>,
    ) -> Result<Self, UnsupportedConnectItemError> {
        if name.is_empty() || fields.contains_key("name") {
            return Err(UnsupportedConnectItemError);
        }
        Ok(Self::Unsupported { name, fields })
    }
}

/// Invalid construction of a forward-compatible connect item.
#[derive(Clone, Copy, Debug, Eq, thiserror::Error, PartialEq)]
#[error("unsupported connect item requires a non-empty name outside its fields")]
pub struct UnsupportedConnectItemError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TonAddressItemWire {
    network: Option<NetworkId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TonProofItemWire {
    payload: String,
}

impl Serialize for ConnectItem {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut object = serde_json::Map::new();
        let _ = object.insert("name".to_owned(), Value::String(self.name().to_owned()));
        match self {
            Self::TonAddr { network } => {
                if let Some(network) = network {
                    let _ = object.insert(
                        "network".to_owned(),
                        serde_json::to_value(network).map_err(serde::ser::Error::custom)?,
                    );
                }
            }
            Self::TonProof { payload } => {
                let _ = object.insert("payload".to_owned(), Value::String(payload.clone()));
            }
            Self::Unsupported { fields, .. } => {
                object.extend(fields.clone());
            }
        }
        Value::Object(object).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ConnectItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let Value::Object(mut object) = Value::deserialize(deserializer)? else {
            return Err(de::Error::custom("connect item must be an object"));
        };
        let name = object
            .remove("name")
            .and_then(|value| value.as_str().map(str::to_owned))
            .filter(|name| !name.is_empty())
            .ok_or_else(|| de::Error::custom("connect item name must be a non-empty string"))?;
        let fields = object.into_iter().collect::<BTreeMap<_, _>>();
        match name.as_str() {
            "ton_addr" => serde_json::from_value::<TonAddressItemWire>(
                serde_json::to_value(fields).map_err(de::Error::custom)?,
            )
            .map(|wire| Self::TonAddr {
                network: wire.network,
            })
            .map_err(de::Error::custom),
            "ton_proof" => serde_json::from_value::<TonProofItemWire>(
                serde_json::to_value(fields).map_err(de::Error::custom)?,
            )
            .map(|wire| Self::TonProof {
                payload: wire.payload,
            })
            .map_err(de::Error::custom),
            _ => Ok(Self::Unsupported { name, fields }),
        }
    }
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
    max_messages: u32,
    /// Whether TEP-92 extra currencies are supported.
    extra_currency_supported: Option<bool>,
    /// Structured item kinds accepted by the wallet.
    item_types: Option<Vec<StructuredItemType>>,
}

impl SendTransactionFeature {
    /// Creates validated transaction capability limits.
    pub fn new(
        max_messages: u32,
        extra_currency_supported: Option<bool>,
        item_types: Option<Vec<StructuredItemType>>,
    ) -> Result<Self, FeatureValidationError> {
        validate_message_feature(max_messages, item_types.as_deref())?;
        Ok(Self {
            max_messages,
            extra_currency_supported,
            item_types,
        })
    }

    /// Returns the maximum accepted outgoing message count.
    #[must_use]
    pub const fn max_messages(&self) -> u32 {
        self.max_messages
    }

    /// Returns whether extra currencies are supported when explicitly declared.
    #[must_use]
    pub const fn extra_currency_supported(&self) -> Option<bool> {
        self.extra_currency_supported
    }

    /// Returns the supported structured item kinds.
    #[must_use]
    pub fn item_types(&self) -> Option<&[StructuredItemType]> {
        self.item_types.as_deref()
    }
}

/// Advertised `SignMessage` limits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignMessageFeature {
    /// Maximum number of outgoing messages accepted in one request.
    max_messages: u32,
    /// Whether TEP-92 extra currencies are supported.
    extra_currency_supported: Option<bool>,
    /// Structured item kinds accepted by the wallet.
    item_types: Option<Vec<StructuredItemType>>,
}

impl SignMessageFeature {
    /// Creates validated message-signing capability limits.
    pub fn new(
        max_messages: u32,
        extra_currency_supported: Option<bool>,
        item_types: Option<Vec<StructuredItemType>>,
    ) -> Result<Self, FeatureValidationError> {
        validate_message_feature(max_messages, item_types.as_deref())?;
        Ok(Self {
            max_messages,
            extra_currency_supported,
            item_types,
        })
    }

    /// Returns the maximum accepted outgoing message count.
    #[must_use]
    pub const fn max_messages(&self) -> u32 {
        self.max_messages
    }

    /// Returns whether extra currencies are supported when explicitly declared.
    #[must_use]
    pub const fn extra_currency_supported(&self) -> Option<bool> {
        self.extra_currency_supported
    }

    /// Returns the supported structured item kinds.
    #[must_use]
    pub fn item_types(&self) -> Option<&[StructuredItemType]> {
        self.item_types.as_deref()
    }
}

/// Advertised `SignData` variants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignDataFeature {
    /// Payload variants accepted by the wallet.
    types: Vec<SignDataType>,
}

impl SignDataFeature {
    /// Creates a non-empty feature with unique payload variants.
    pub fn new(types: Vec<SignDataType>) -> Result<Self, FeatureValidationError> {
        if types.is_empty() || !all_unique(&types) {
            return Err(FeatureValidationError);
        }
        Ok(Self { types })
    }

    /// Returns the supported payload variants.
    #[must_use]
    pub fn types(&self) -> &[SignDataType] {
        &self.types
    }
}

/// Feature capability violates the normative schema constraints.
#[derive(Clone, Copy, Debug, Eq, thiserror::Error, PartialEq)]
#[error("feature arrays must be non-empty and unique, and maxMessages must be positive")]
pub struct FeatureValidationError;

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
            } => SendTransactionFeature::new(max_messages, extra_currency_supported, item_types)
                .map(Self::SendTransaction)
                .map_err(de::Error::custom),
            DetailedFeature::SignData { types } => SignDataFeature::new(types)
                .map(Self::SignData)
                .map_err(de::Error::custom),
            DetailedFeature::SignMessage {
                max_messages,
                extra_currency_supported,
                item_types,
            } => SignMessageFeature::new(max_messages, extra_currency_supported, item_types)
                .map(Self::SignMessage)
                .map_err(de::Error::custom),
            DetailedFeature::EmbeddedRequest => Ok(Self::EmbeddedRequest),
        }
    }
}

fn validate_message_feature(
    max_messages: u32,
    item_types: Option<&[StructuredItemType]>,
) -> Result<(), FeatureValidationError> {
    if max_messages == 0
        || item_types.is_some_and(|values| values.is_empty() || !all_unique(values))
    {
        Err(FeatureValidationError)
    } else {
        Ok(())
    }
}

fn all_unique<T: Eq>(values: &[T]) -> bool {
    values.iter().enumerate().all(|(index, value)| {
        values
            .iter()
            .skip(index.saturating_add(1))
            .all(|other| other != value)
    })
}

/// Wallet self-description returned in a successful connect event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeviceInfoWire {
    platform: DevicePlatform,
    app_name: String,
    app_version: String,
    max_protocol_version: u32,
    features: Vec<Feature>,
}

impl DeviceInfo {
    /// Creates a validated runtime wallet descriptor.
    pub fn new(
        platform: DevicePlatform,
        app_name: String,
        app_version: String,
        max_protocol_version: u32,
        features: Vec<Feature>,
    ) -> Result<Self, DeviceInfoValidationError> {
        let value = Self {
            platform,
            app_name,
            app_version,
            max_protocol_version,
            features,
        };
        value.validate()?;
        Ok(value)
    }

    /// Validates identity, protocol-version, and feature-set invariants.
    pub fn validate(&self) -> Result<(), DeviceInfoValidationError> {
        if self.app_name.is_empty() || self.app_version.is_empty() {
            return Err(DeviceInfoValidationError::EmptyIdentity);
        }
        if self.max_protocol_version < u32::from(crate::PROTOCOL_VERSION) {
            return Err(DeviceInfoValidationError::UnsupportedProtocolVersion);
        }

        let mut names = Vec::new();
        for feature in &self.features {
            let name = match feature {
                Feature::LegacySendTransaction | Feature::SendTransaction(_) => "SendTransaction",
                Feature::SignData(_) => "SignData",
                Feature::SignMessage(_) => "SignMessage",
                Feature::EmbeddedRequest => "EmbeddedRequest",
            };
            if names.contains(&name) {
                return Err(DeviceInfoValidationError::DuplicateFeature);
            }
            names.push(name);
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for DeviceInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = DeviceInfoWire::deserialize(deserializer)?;
        Self::new(
            wire.platform,
            wire.app_name,
            wire.app_version,
            wire.max_protocol_version,
            wire.features,
        )
        .map_err(de::Error::custom)
    }
}

/// Runtime wallet metadata is not a valid protocol-v2 `DeviceInfo` value.
#[derive(Clone, Copy, Debug, Eq, thiserror::Error, PartialEq)]
pub enum DeviceInfoValidationError {
    /// Wallet registry identity or application version is empty.
    #[error("DeviceInfo appName and appVersion must be non-empty")]
    EmptyIdentity,
    /// The wallet does not report support for the current protocol revision.
    #[error("DeviceInfo maxProtocolVersion does not support TON Connect v2")]
    UnsupportedProtocolVersion,
    /// More than one entry advertises the same runtime feature.
    #[error("DeviceInfo contains a duplicate feature")]
    DuplicateFeature,
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
    /// Canonical standard-base64 wallet `StateInit` `BoC`.
    #[serde(rename = "walletStateInit")]
    pub wallet_state_init: WalletStateInit,
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
        wallet_state_init: WalletStateInit,
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

    /// Verifies that `walletStateInit` derives this address and stores the
    /// advertised key under a recognized standard wallet data layout.
    pub fn verify_standard_wallet(&self) -> Result<StandardWalletState, WalletStateError> {
        self.wallet_state_init
            .verify_standard_wallet(&self.address, &self.public_key)
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
    ) -> Result<bool, SigningError> {
        verify_signature(
            &self.signing_hash(address)?,
            &self.signature,
            public_key,
            SignatureDomain::Empty,
        )
    }

    /// Verifies this proof using the address-bound key from a standard wallet
    /// `StateInit`, never trusting the advertised key by itself.
    pub fn verify_with_account(
        &self,
        account: &TonAddressItemReply,
    ) -> Result<bool, AccountVerificationError> {
        let wallet = account.verify_standard_wallet()?;
        self.verify(&account.address, wallet.public_key())
            .map_err(Into::into)
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

impl ConnectItemReply {
    /// Returns the connect-item discriminator echoed by this reply.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::TonAddress(_) => "ton_addr",
            Self::TonProof(_) => "ton_proof",
            Self::Error(error) => error.name(),
        }
    }

    /// Creates the protocol-required error for an unsupported requested item.
    #[must_use]
    pub fn unsupported(item: &ConnectItem, message: Option<String>) -> Self {
        Self::Error(ConnectItemError::new(
            item.name().to_owned(),
            ConnectItemErrorCode::MethodNotSupported,
            message,
        ))
    }
}

/// Payload of a successful connect event.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
        response: Option<EmbeddedResponse>,
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
        payload: EmptyObject,
    },
}

impl ConnectEvent {
    /// Returns the monotonic wallet event identifier.
    #[must_use]
    pub const fn id(&self) -> u64 {
        match self {
            Self::Connect { id, .. }
            | Self::ConnectError { id, .. }
            | Self::Disconnect { id, .. } => *id,
        }
    }

    /// Reports whether this event ends or rejects the session.
    #[must_use]
    pub const fn terminates_session(&self) -> bool {
        matches!(self, Self::ConnectError { .. } | Self::Disconnect { .. })
    }

    /// Validates a connect response against the exact request and optional
    /// embedded action carried by the link.
    ///
    /// Replies may be reordered, but their names and multiplicities must match
    /// the requested items. A successful connect always needs exactly one
    /// successful `ton_addr` reply because that address becomes the immutable
    /// session identity.
    pub fn validate_for_connect(
        &self,
        request: &ConnectRequest,
        embedded_request: Option<&crate::EmbeddedRequest>,
    ) -> Result<(), ConnectValidationError> {
        let Self::Connect {
            payload, response, ..
        } = self
        else {
            return if matches!(self, Self::ConnectError { .. }) {
                Ok(())
            } else {
                Err(ConnectValidationError::NotAConnectResponse)
            };
        };

        payload.device.validate()?;
        let requested = request.items.as_slice();
        if requested
            .iter()
            .filter(|item| matches!(item, ConnectItem::TonAddr { .. }))
            .count()
            != 1
        {
            return Err(ConnectValidationError::InvalidTonAddressRequest);
        }
        if requested.len() != payload.items.len()
            || requested.iter().any(|item| {
                let requested_count = requested
                    .iter()
                    .filter(|candidate| candidate.name() == item.name())
                    .count();
                let reply_count = payload
                    .items
                    .iter()
                    .filter(|reply| reply.name() == item.name())
                    .count();
                requested_count != reply_count
            })
        {
            return Err(ConnectValidationError::ItemReplyMismatch);
        }

        let mut accounts = payload.items.iter().filter_map(|reply| match reply {
            ConnectItemReply::TonAddress(account) => Some(account),
            ConnectItemReply::TonProof(_) | ConnectItemReply::Error(_) => None,
        });
        let Some(account) = accounts.next() else {
            return Err(ConnectValidationError::MissingTonAddressReply);
        };
        if accounts.next().is_some() {
            return Err(ConnectValidationError::MultipleTonAddressReplies);
        }
        let requested_network = requested.iter().find_map(|item| match item {
            ConnectItem::TonAddr { network } => network.as_ref(),
            ConnectItem::TonProof { .. } | ConnectItem::Unsupported { .. } => None,
        });
        if requested_network.is_some_and(|network| network != &account.network) {
            return Err(ConnectValidationError::NetworkMismatch);
        }

        for proof in payload.items.iter().filter_map(|reply| match reply {
            ConnectItemReply::TonProof(reply) => Some(&reply.proof),
            ConnectItemReply::TonAddress(_) | ConnectItemReply::Error(_) => None,
        }) {
            if !requested.iter().any(|item| {
                matches!(item, ConnectItem::TonProof { payload } if payload == &proof.payload)
            }) {
                return Err(ConnectValidationError::ProofPayloadMismatch);
            }
        }

        let supports_embedded = payload
            .device
            .features
            .iter()
            .any(|feature| matches!(feature, Feature::EmbeddedRequest));
        match (embedded_request, response, supports_embedded) {
            (None, None, false | true) | (Some(_), None, false) => Ok(()),
            (None, Some(_), false | true) | (Some(_), Some(_), false) => {
                Err(ConnectValidationError::UnexpectedEmbeddedResponse)
            }
            (Some(_), None, true) => Err(ConnectValidationError::MissingEmbeddedResponse),
            (Some(request), Some(response), true) => {
                let _ = response.validate_for(request)?;
                Ok(())
            }
        }
    }
}

/// A connect event cannot be correlated safely with its initiating request.
#[derive(Clone, Debug, Eq, thiserror::Error, PartialEq)]
pub enum ConnectValidationError {
    /// A disconnect event was passed where a connect response was expected.
    #[error("disconnect event is not a response to a connect request")]
    NotAConnectResponse,
    /// A successful connection requires exactly one requested `ton_addr` item.
    #[error("connect request must contain exactly one ton_addr item")]
    InvalidTonAddressRequest,
    /// Reply names or counts do not match the requested items.
    #[error("connect item replies do not match the request")]
    ItemReplyMismatch,
    /// The wallet did not return the account required to establish a session.
    #[error("successful connect event has no ton_addr reply")]
    MissingTonAddressReply,
    /// More than one account identity was returned for one session.
    #[error("successful connect event has multiple ton_addr replies")]
    MultipleTonAddressReplies,
    /// The returned account network differs from the explicit request network.
    #[error("connected account network differs from the requested network")]
    NetworkMismatch,
    /// A proof does not echo any challenge from the connect request.
    #[error("ton_proof reply payload does not match the requested challenge")]
    ProofPayloadMismatch,
    /// A response appeared without a supported embedded request.
    #[error("connect event contains an unexpected embedded response")]
    UnexpectedEmbeddedResponse,
    /// The wallet advertised embedded support but omitted the action response.
    #[error("connect event omitted the embedded request response")]
    MissingEmbeddedResponse,
    /// Runtime wallet metadata violates protocol invariants.
    #[error(transparent)]
    InvalidDeviceInfo(#[from] DeviceInfoValidationError),
    /// Embedded action result does not match its method.
    #[error(transparent)]
    InvalidEmbeddedResponse(#[from] ResponseValidationError),
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
    fn unknown_connect_item_round_trips_and_can_receive_error_400()
    -> Result<(), Box<dyn std::error::Error>> {
        let json = r#"{"name":"future_identity","scope":"read","revision":3}"#;
        let item = serde_json::from_str::<ConnectItem>(json)?;
        assert_eq!(item.name(), "future_identity");
        assert!(matches!(item, ConnectItem::Unsupported { .. }));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&serde_json::to_string(&item)?)?,
            serde_json::from_str::<serde_json::Value>(json)?
        );

        let reply = ConnectItemReply::unsupported(&item, None);
        assert!(matches!(
            reply,
            ConnectItemReply::Error(error)
                if error.name() == "future_identity"
                    && error.code() == ConnectItemErrorCode::MethodNotSupported
        ));
        Ok(())
    }

    #[test]
    fn known_connect_items_still_reject_unknown_fields() {
        assert!(
            serde_json::from_str::<ConnectItem>(
                r#"{"name":"ton_addr","network":"-239","future":true}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<ConnectItem>(
                r#"{"name":"ton_proof","payload":"nonce","future":true}"#
            )
            .is_err()
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
    fn feature_rejects_zero_limits_and_empty_or_duplicate_arrays() {
        let invalid = [
            r#"{"name":"SendTransaction","maxMessages":0}"#,
            r#"{"name":"SendTransaction","maxMessages":1,"itemTypes":[]}"#,
            r#"{"name":"SendTransaction","maxMessages":1,"itemTypes":["ton","ton"]}"#,
            r#"{"name":"SignMessage","maxMessages":0}"#,
            r#"{"name":"SignData","types":[]}"#,
            r#"{"name":"SignData","types":["text","text"]}"#,
        ];
        for value in invalid {
            assert!(
                serde_json::from_str::<Feature>(value).is_err(),
                "accepted {value}"
            );
        }
    }

    #[test]
    fn device_info_rejects_invalid_identity_version_and_duplicate_features() {
        let invalid = [
            r#"{"platform":"browser","appName":"","appVersion":"1","maxProtocolVersion":2,"features":[]}"#,
            r#"{"platform":"browser","appName":"wallet","appVersion":"","maxProtocolVersion":2,"features":[]}"#,
            r#"{"platform":"browser","appName":"wallet","appVersion":"1","maxProtocolVersion":1,"features":[]}"#,
            r#"{"platform":"browser","appName":"wallet","appVersion":"1","maxProtocolVersion":2,"features":["SendTransaction",{"name":"SendTransaction","maxMessages":4}]}"#,
        ];
        for value in invalid {
            assert!(
                serde_json::from_str::<DeviceInfo>(value).is_err(),
                "accepted {value}"
            );
        }
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
        assert!(proof.verify(&address, &public_key)?);
        proof.payload.push_str("-changed");
        assert!(!proof.verify(&address, &public_key)?);
        Ok(())
    }
}
