use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::{
    AccountAddress, AccountVerificationError, Base64Value, CellBoc, DecimalString,
    Ed25519PublicKey, Ed25519Signature, FriendlyAddress, NetworkId, NonEmptyVec, RawAccountAddress,
    SignDataSigningPayload, SignDataType, SignatureDomain, SigningError, StructuredItemType,
    TonAddressItemReply, sign_data_signing_hash, verify_signature,
};

/// Extra-currency identifier to non-negative elementary-unit amount.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExtraCurrencies(BTreeMap<u32, DecimalString>);

impl ExtraCurrencies {
    /// Creates a map from already typed currency identifiers.
    #[must_use]
    pub fn new(values: BTreeMap<u32, DecimalString>) -> Self {
        Self(values)
    }

    /// Returns the typed currency map.
    #[must_use]
    pub const fn as_map(&self) -> &BTreeMap<u32, DecimalString> {
        &self.0
    }

    /// Reports whether the map contains no extra currencies.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Serialize for ExtraCurrencies {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap as _;

        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (currency_id, amount) in &self.0 {
            map.serialize_entry(&currency_id.to_string(), amount)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ExtraCurrencies {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BTreeMap::<String, DecimalString>::deserialize(deserializer)?;
        let mut values = BTreeMap::new();
        for (currency_id, amount) in wire {
            let parsed = currency_id
                .parse::<u32>()
                .map_err(|_| de::Error::custom("extra-currency id must be an unsigned integer"))?;
            if parsed.to_string() != currency_id || values.insert(parsed, amount).is_some() {
                return Err(de::Error::custom(
                    "extra-currency id must use canonical decimal form",
                ));
            }
        }
        Ok(Self(values))
    }
}

/// A raw outgoing message in `sendTransaction` or `signMessage`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawMessage {
    /// Destination in TEP-2 user-friendly form.
    pub address: FriendlyAddress,
    /// Nanocoins represented as a non-negative decimal string.
    pub amount: DecimalString,
    /// Optional base64 one-cell body `BoC`.
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub payload: Option<CellBoc>,
    /// Optional base64 one-cell `StateInit` `BoC`.
    #[serde(
        rename = "stateInit",
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub state_init: Option<CellBoc>,
    /// Optional TEP-92 extra currencies.
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub extra_currency: Option<ExtraCurrencies>,
}

/// A wallet-built structured transfer item.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum StructuredItem {
    /// Native TON transfer.
    Ton {
        /// Destination address.
        address: FriendlyAddress,
        /// Nanocoins to transfer.
        amount: DecimalString,
        /// Optional base64 one-cell body `BoC`.
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        payload: Option<CellBoc>,
        /// Optional base64 one-cell `StateInit` `BoC`.
        #[serde(
            rename = "stateInit",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        state_init: Option<CellBoc>,
        /// Optional TEP-92 extra currencies.
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        extra_currency: Option<ExtraCurrencies>,
    },
    /// TEP-74 jetton transfer.
    Jetton {
        /// Jetton master contract address.
        master: AccountAddress,
        /// Recipient address.
        destination: AccountAddress,
        /// Jetton elementary-unit amount.
        amount: DecimalString,
        /// Optional attached TON amount in nanocoins.
        #[serde(
            rename = "attachAmount",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        attach_amount: Option<DecimalString>,
        /// Optional application query identifier.
        #[serde(
            rename = "queryId",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        query_id: Option<String>,
        /// Optional excess-TON refund address.
        #[serde(
            rename = "responseDestination",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        response_destination: Option<AccountAddress>,
        /// Optional base64 `custom_payload` cell `BoC`.
        #[serde(
            rename = "customPayload",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        custom_payload: Option<CellBoc>,
        /// Optional forwarded TON amount in nanocoins.
        #[serde(
            rename = "forwardAmount",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        forward_amount: Option<DecimalString>,
        /// Optional base64 `forward_payload` cell `BoC`.
        #[serde(
            rename = "forwardPayload",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        forward_payload: Option<CellBoc>,
    },
    /// TEP-62 NFT transfer.
    Nft {
        /// NFT item contract address.
        #[serde(rename = "nftAddress")]
        nft_address: AccountAddress,
        /// New owner address.
        #[serde(rename = "newOwner")]
        new_owner: AccountAddress,
        /// Optional attached TON amount in nanocoins.
        #[serde(
            rename = "attachAmount",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        attach_amount: Option<DecimalString>,
        /// Optional application query identifier.
        #[serde(
            rename = "queryId",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        query_id: Option<String>,
        /// Optional excess-TON refund address.
        #[serde(
            rename = "responseDestination",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        response_destination: Option<AccountAddress>,
        /// Optional base64 `custom_payload` cell `BoC`.
        #[serde(
            rename = "customPayload",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        custom_payload: Option<CellBoc>,
        /// Optional forwarded TON amount in nanocoins.
        #[serde(
            rename = "forwardAmount",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        forward_amount: Option<DecimalString>,
        /// Optional base64 `forward_payload` cell `BoC`.
        #[serde(
            rename = "forwardPayload",
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        forward_payload: Option<CellBoc>,
    },
}

/// A transaction payload containing caller-built raw messages.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawTransactionPayload {
    /// Optional Unix expiration time in seconds.
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub valid_until: Option<u64>,
    /// Optional target network global ID.
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub network: Option<NetworkId>,
    /// Optional fixed sender address.
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub from: Option<AccountAddress>,
    /// One or more raw outgoing messages.
    pub messages: NonEmptyVec<RawMessage>,
}

/// A transaction payload containing wallet-built structured items.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredTransactionPayload {
    /// Optional Unix expiration time in seconds.
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub valid_until: Option<u64>,
    /// Optional target network global ID.
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub network: Option<NetworkId>,
    /// Optional fixed sender address.
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub from: Option<AccountAddress>,
    /// One or more structured transfer items.
    pub items: NonEmptyVec<StructuredItem>,
}

/// Payload shared by `sendTransaction` and `signMessage`.
///
/// The untagged representation enforces the protocol's exclusive choice:
/// exactly one of `messages` or `items` must be present.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum TransactionPayload {
    /// Raw caller-built messages.
    Raw(RawTransactionPayload),
    /// Wallet-built structured items.
    Structured(StructuredTransactionPayload),
}

impl<'de> Deserialize<'de> for TransactionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let Value::Object(object) = &value else {
            return Err(de::Error::custom("transaction payload must be an object"));
        };
        match (
            object.contains_key("messages"),
            object.contains_key("items"),
        ) {
            (true, false) => serde_json::from_value::<RawTransactionPayload>(value)
                .map(Self::Raw)
                .map_err(de::Error::custom),
            (false, true) => serde_json::from_value::<StructuredTransactionPayload>(value)
                .map(Self::Structured)
                .map_err(de::Error::custom),
            (true, true) | (false, false) => Err(de::Error::custom(
                "transaction payload requires exactly one of messages or items",
            )),
        }
    }
}

impl TransactionPayload {
    /// Returns the number of raw messages or structured items in the request.
    #[must_use]
    pub fn message_count(&self) -> usize {
        match self {
            Self::Raw(payload) => payload.messages.as_slice().len(),
            Self::Structured(payload) => payload.items.as_slice().len(),
        }
    }

    /// Reports whether the request transfers any TEP-92 extra currency.
    #[must_use]
    pub fn uses_extra_currency(&self) -> bool {
        match self {
            Self::Raw(payload) => payload.messages.as_slice().iter().any(|message| {
                message
                    .extra_currency
                    .as_ref()
                    .is_some_and(|currencies| !currencies.is_empty())
            }),
            Self::Structured(payload) => payload.items.as_slice().iter().any(|item| {
                matches!(
                    item,
                    StructuredItem::Ton {
                        extra_currency: Some(currencies),
                        ..
                    } if !currencies.is_empty()
                )
            }),
        }
    }

    /// Returns the structured item kinds used by this request.
    ///
    /// `None` identifies the raw `messages` form. A non-empty slice identifies
    /// the `items` form and retains duplicates because each item still counts
    /// towards the wallet's advertised `maxMessages` limit.
    #[must_use]
    pub fn structured_item_types(&self) -> Option<Vec<StructuredItemType>> {
        let Self::Structured(payload) = self else {
            return None;
        };
        Some(
            payload
                .items
                .as_slice()
                .iter()
                .map(|item| match item {
                    StructuredItem::Ton { .. } => StructuredItemType::Ton,
                    StructuredItem::Jetton { .. } => StructuredItemType::Jetton,
                    StructuredItem::Nft { .. } => StructuredItemType::Nft,
                })
                .collect(),
        )
    }

    /// Validates time, network, and fixed-sender constraints common to
    /// `sendTransaction` and `signMessage`.
    pub fn validate_context(
        &self,
        now: u64,
        active_network: &NetworkId,
        active_account: &RawAccountAddress,
    ) -> Result<(), RequestContextError> {
        let (valid_until, network, from) = match self {
            Self::Raw(payload) => (
                payload.valid_until,
                payload.network.as_ref(),
                payload.from.as_ref(),
            ),
            Self::Structured(payload) => (
                payload.valid_until,
                payload.network.as_ref(),
                payload.from.as_ref(),
            ),
        };
        if valid_until.is_some_and(|valid_until| now > valid_until) {
            return Err(RequestContextError::Expired);
        }
        validate_network_and_account(network, from, active_network, active_account)
    }
}

/// Discriminated `signData` payload.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum SignDataPayload {
    /// UTF-8 text shown and signed verbatim.
    Text {
        /// Text to sign.
        text: String,
        /// Optional target network global ID.
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        network: Option<NetworkId>,
        /// Optional fixed signer address.
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        from: Option<AccountAddress>,
    },
    /// Opaque binary bytes.
    Binary {
        /// Base64-encoded bytes to sign.
        bytes: Base64Value,
        /// Optional target network global ID.
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        network: Option<NetworkId>,
        /// Optional fixed signer address.
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        from: Option<AccountAddress>,
    },
    /// A TVM cell interpreted with a TL-B schema.
    Cell {
        /// TL-B schema whose final declaration is the root.
        schema: String,
        /// Base64 cell `BoC`.
        cell: CellBoc,
        /// Optional target network global ID.
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        network: Option<NetworkId>,
        /// Optional fixed signer address.
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        from: Option<AccountAddress>,
    },
}

impl SignDataPayload {
    /// Returns the capability discriminator required to process this payload.
    #[must_use]
    pub const fn data_type(&self) -> SignDataType {
        match self {
            Self::Text { .. } => SignDataType::Text,
            Self::Binary { .. } => SignDataType::Binary,
            Self::Cell { .. } => SignDataType::Cell,
        }
    }

    /// Validates optional network and signer constraints for `signData`.
    pub fn validate_context(
        &self,
        active_network: &NetworkId,
        active_account: &RawAccountAddress,
    ) -> Result<(), RequestContextError> {
        let (network, from) = match self {
            Self::Text { network, from, .. }
            | Self::Binary { network, from, .. }
            | Self::Cell { network, from, .. } => (network.as_ref(), from.as_ref()),
        };
        validate_network_and_account(network, from, active_network, active_account)
    }
}

/// Request constraints conflict with the wallet's active session context.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RequestContextError {
    /// Transaction `valid_until` is before the observed clock.
    #[error("TON Connect request has expired")]
    Expired,
    /// Explicit request network differs from the active wallet network.
    #[error("TON Connect request network differs from the active network")]
    NetworkMismatch,
    /// Explicit `from` address differs from the session account.
    #[error("TON Connect request signer differs from the connected account")]
    AccountMismatch,
}

fn validate_network_and_account(
    network: Option<&NetworkId>,
    from: Option<&AccountAddress>,
    active_network: &NetworkId,
    active_account: &RawAccountAddress,
) -> Result<(), RequestContextError> {
    if network.is_some_and(|network| network != active_network) {
        return Err(RequestContextError::NetworkMismatch);
    }
    if from.is_some_and(|from| from.raw_address() != *active_account) {
        return Err(RequestContextError::AccountMismatch);
    }
    Ok(())
}

/// Raw `{ method, params, id }` request envelope received from a dApp.
///
/// `method` remains a string so a wallet can parse an unknown method and
/// return protocol error 400 instead of dropping an otherwise valid request.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppRequest {
    /// RPC method name.
    pub method: String,
    /// Method parameters. Complex payloads are JSON strings inside this array.
    pub params: Vec<String>,
    /// dApp-assigned request identifier.
    pub id: String,
}

/// A validated `sendTransaction` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendTransactionRequest {
    /// dApp request identifier.
    pub id: String,
    /// Transaction to sign and broadcast.
    pub payload: TransactionPayload,
}

/// A validated `signMessage` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignMessageRequest {
    /// dApp request identifier.
    pub id: String,
    /// Transaction-shaped message to sign without broadcasting.
    pub payload: TransactionPayload,
}

/// A validated `signData` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignDataRequest {
    /// dApp request identifier.
    pub id: String,
    /// Data and signer constraints.
    pub payload: SignDataPayload,
}

/// A validated `disconnect` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisconnectRequest {
    /// dApp request identifier.
    pub id: String,
}

/// A known request with its nested JSON parameter parsed and validated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KnownAppRequest {
    /// Sign and broadcast a transaction.
    SendTransaction(SendTransactionRequest),
    /// Sign an internal message without broadcasting.
    SignMessage(SignMessageRequest),
    /// Sign application data.
    SignData(SignDataRequest),
    /// End the session.
    Disconnect(DisconnectRequest),
}

impl KnownAppRequest {
    /// Returns the protocol method discriminator for this request.
    #[must_use]
    pub fn method(&self) -> crate::RpcMethod {
        self.into()
    }

    /// Returns the exact dApp request identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::SendTransaction(request) => &request.id,
            Self::SignMessage(request) => &request.id,
            Self::SignData(request) => &request.id,
            Self::Disconnect(request) => &request.id,
        }
    }
}

/// Failure while decoding or encoding an RPC request.
#[derive(Debug, Error)]
pub enum RpcError {
    /// The method is not implemented by this protocol version.
    #[error("unsupported TON Connect RPC method: {0}")]
    UnsupportedMethod(String),
    /// The method carried an incorrect number of parameters.
    #[error("TON Connect RPC method {method} requires {expected} parameter(s), got {actual}")]
    InvalidParameterCount {
        /// Method being parsed.
        method: &'static str,
        /// Required parameter count.
        expected: usize,
        /// Received parameter count.
        actual: usize,
    },
    /// A nested JSON payload violates the method schema.
    #[error("invalid TON Connect RPC payload: {0}")]
    InvalidPayload(#[from] serde_json::Error),
}

impl AppRequest {
    /// Parses the method-specific parameter and enforces its exact schema.
    pub fn decode(self) -> Result<KnownAppRequest, RpcError> {
        match self.method.as_str() {
            "sendTransaction" => Ok(KnownAppRequest::SendTransaction(SendTransactionRequest {
                id: self.id,
                payload: parse_single_parameter("sendTransaction", self.params)?,
            })),
            "signMessage" => Ok(KnownAppRequest::SignMessage(SignMessageRequest {
                id: self.id,
                payload: parse_single_parameter("signMessage", self.params)?,
            })),
            "signData" => Ok(KnownAppRequest::SignData(SignDataRequest {
                id: self.id,
                payload: parse_single_parameter("signData", self.params)?,
            })),
            "disconnect" => {
                if self.params.is_empty() {
                    Ok(KnownAppRequest::Disconnect(DisconnectRequest {
                        id: self.id,
                    }))
                } else {
                    Err(RpcError::InvalidParameterCount {
                        method: "disconnect",
                        expected: 0,
                        actual: self.params.len(),
                    })
                }
            }
            _ => Err(RpcError::UnsupportedMethod(self.method)),
        }
    }

    /// Encodes a validated request into the exact bridge envelope shape.
    pub fn encode(request: KnownAppRequest) -> Result<Self, RpcError> {
        match request {
            KnownAppRequest::SendTransaction(request) => Ok(Self {
                method: "sendTransaction".to_owned(),
                params: vec![serde_json::to_string(&request.payload)?],
                id: request.id,
            }),
            KnownAppRequest::SignMessage(request) => Ok(Self {
                method: "signMessage".to_owned(),
                params: vec![serde_json::to_string(&request.payload)?],
                id: request.id,
            }),
            KnownAppRequest::SignData(request) => Ok(Self {
                method: "signData".to_owned(),
                params: vec![serde_json::to_string(&request.payload)?],
                id: request.id,
            }),
            KnownAppRequest::Disconnect(request) => Ok(Self {
                method: "disconnect".to_owned(),
                params: Vec::new(),
                id: request.id,
            }),
        }
    }
}

fn parse_single_parameter<T>(method: &'static str, params: Vec<String>) -> Result<T, RpcError>
where
    T: for<'de> Deserialize<'de>,
{
    if params.len() != 1 {
        return Err(RpcError::InvalidParameterCount {
            method,
            expected: 1,
            actual: params.len(),
        });
    }
    let Some(payload) = params.into_iter().next() else {
        return Err(RpcError::InvalidParameterCount {
            method,
            expected: 1,
            actual: 0,
        });
    };
    serde_json::from_str(&payload).map_err(RpcError::from)
}

macro_rules! numeric_enum_serde {
    ($name:ident { $($variant:ident = $value:literal),+ $(,)? }) => {
        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                let value = match self {
                    $(Self::$variant => $value,)+
                };
                serializer.serialize_u16(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                match u16::deserialize(deserializer)? {
                    $($value => Ok(Self::$variant),)+
                    value => Err(de::Error::custom(format_args!(
                        "unsupported TON Connect error code {value}"
                    ))),
                }
            }
        }
    };
}

pub(crate) use numeric_enum_serde;

/// Central TON Connect RPC error catalogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum RpcErrorCode {
    /// Unexpected wallet-side failure.
    Unknown = 0,
    /// Malformed request or violated method constraint.
    BadRequest = 1,
    /// Unknown or revoked app session.
    UnknownApp = 100,
    /// User declined the operation.
    UserDeclined = 300,
    /// Wallet does not implement the method.
    MethodNotSupported = 400,
}

numeric_enum_serde!(RpcErrorCode {
    Unknown = 0,
    BadRequest = 1,
    UnknownApp = 100,
    UserDeclined = 300,
    MethodNotSupported = 400,
});

/// Success result permitted by the generic wallet-response schema.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum WalletResult {
    /// String result, such as a signed external-message `BoC`.
    String(String),
    /// Object result, such as `signData` or `signMessage` output.
    Object(Map<String, Value>),
}

/// Successful wallet RPC response.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WalletResponseSuccess {
    /// Method-specific result.
    pub result: WalletResult,
    /// Exact request ID being answered.
    pub id: String,
}

/// Protocol RPC error body.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WalletResponseError {
    /// Protocol error code.
    pub code: RpcErrorCode,
    /// Human-readable diagnostic.
    pub message: String,
    /// Optional method-specific details.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Wallet response correlated to one dApp request ID.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum WalletResponse {
    /// Successful response.
    Success(WalletResponseSuccess),
    /// Error response.
    Error {
        /// Protocol error body.
        error: WalletResponseError,
        /// Exact request ID being answered.
        id: String,
    },
}

/// Typed success body returned by `signMessage`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignMessageResult {
    /// Base64 `BoC` of the signed internal message.
    pub internal_boc: CellBoc,
}

/// Typed success body returned by `signData`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignDataResult {
    /// Base64 Ed25519 signature.
    pub signature: Ed25519Signature,
    /// Raw wallet address.
    pub address: RawAccountAddress,
    /// Unix signing time in seconds.
    pub timestamp: u64,
    /// Application domain bound into the signature.
    pub domain: String,
    /// Exact payload echoed from the request.
    pub payload: SignDataPayload,
}

impl SignDataResult {
    /// Reconstructs the exact digest represented by this response.
    pub fn signing_hash(&self) -> Result<[u8; 32], SigningError> {
        match &self.payload {
            SignDataPayload::Text { text, .. } => sign_data_signing_hash(
                &self.address,
                &self.domain,
                self.timestamp,
                SignDataSigningPayload::Text(text),
            ),
            SignDataPayload::Binary { bytes, .. } => {
                let decoded = bytes
                    .decode()
                    .map_err(|_| SigningError::InvalidBase64Payload)?;
                sign_data_signing_hash(
                    &self.address,
                    &self.domain,
                    self.timestamp,
                    SignDataSigningPayload::Binary(&decoded),
                )
            }
            SignDataPayload::Cell { schema, cell, .. } => sign_data_signing_hash(
                &self.address,
                &self.domain,
                self.timestamp,
                SignDataSigningPayload::Cell {
                    schema,
                    boc: cell.as_bytes(),
                },
            ),
        }
    }

    /// Verifies this `signData` response with the trusted account public key.
    pub fn verify(&self, public_key: &Ed25519PublicKey) -> Result<bool, SigningError> {
        verify_signature(
            &self.signing_hash()?,
            &self.signature,
            public_key,
            SignatureDomain::Empty,
        )
    }

    /// Verifies this result with the address-bound key from the connected
    /// standard wallet rather than trusting a wallet-advertised key.
    pub fn verify_with_account(
        &self,
        account: &TonAddressItemReply,
    ) -> Result<bool, AccountVerificationError> {
        if self.address != account.address {
            return Err(AccountVerificationError::ResponseAddressMismatch);
        }
        let wallet = account.verify_standard_wallet()?;
        self.verify(wallet.public_key()).map_err(Into::into)
    }
}

/// Method-specific response after correlation and payload validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KnownWalletResponse {
    /// Signed and broadcast external-message `BoC`.
    SendTransaction(CellBoc),
    /// Signed internal message.
    SignMessage(SignMessageResult),
    /// Application data signature.
    SignData(Box<SignDataResult>),
    /// Successful disconnect acknowledgement.
    Disconnect,
    /// Protocol error valid for the requested method.
    Error(WalletResponseError),
}

/// Failure to correlate or validate a wallet response for a known request.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ResponseValidationError {
    /// Response and request IDs differ.
    #[error("wallet response id does not match request id")]
    RequestIdMismatch,
    /// The success result has the wrong JSON kind or fields for the method.
    #[error("wallet response result does not match the requested method")]
    InvalidResult,
    /// A result that must contain bytes decoded to an empty value.
    #[error("wallet response result must not be empty")]
    EmptyResult,
    /// `signData` did not echo the exact request payload.
    #[error("signData response payload does not match request payload")]
    SignDataPayloadMismatch,
    /// The method does not permit the returned protocol error code.
    #[error("wallet response error code is not valid for the requested method")]
    InvalidErrorCode,
}

impl WalletResponse {
    /// Correlates this response and validates its method-specific result.
    pub fn validate_for(
        &self,
        request: &KnownAppRequest,
    ) -> Result<KnownWalletResponse, ResponseValidationError> {
        let response_id = match self {
            Self::Success(response) => &response.id,
            Self::Error { id, .. } => id,
        };
        if response_id != request.id() {
            return Err(ResponseValidationError::RequestIdMismatch);
        }

        match self {
            Self::Error { error, .. } => {
                if matches!(request, KnownAppRequest::Disconnect(_))
                    && error.code == RpcErrorCode::UserDeclined
                {
                    return Err(ResponseValidationError::InvalidErrorCode);
                }
                Ok(KnownWalletResponse::Error(error.clone()))
            }
            Self::Success(response) => validate_success_result(&response.result, request),
        }
    }
}

fn validate_success_result(
    result: &WalletResult,
    request: &KnownAppRequest,
) -> Result<KnownWalletResponse, ResponseValidationError> {
    match (request, result) {
        (KnownAppRequest::SendTransaction(_), WalletResult::String(result)) => {
            let boc = CellBoc::try_from(result.clone())
                .map_err(|_| ResponseValidationError::InvalidResult)?;
            if boc.as_str().is_empty() {
                return Err(ResponseValidationError::EmptyResult);
            }
            Ok(KnownWalletResponse::SendTransaction(boc))
        }
        (KnownAppRequest::SignMessage(_), WalletResult::Object(result)) => {
            let parsed = serde_json::from_value::<SignMessageResult>(Value::Object(result.clone()))
                .map_err(|_| ResponseValidationError::InvalidResult)?;
            if parsed.internal_boc.as_str().is_empty() {
                return Err(ResponseValidationError::EmptyResult);
            }
            Ok(KnownWalletResponse::SignMessage(parsed))
        }
        (KnownAppRequest::SignData(request), WalletResult::Object(result)) => {
            let parsed = serde_json::from_value::<SignDataResult>(Value::Object(result.clone()))
                .map_err(|_| ResponseValidationError::InvalidResult)?;
            if parsed.payload != request.payload {
                return Err(ResponseValidationError::SignDataPayloadMismatch);
            }
            Ok(KnownWalletResponse::SignData(Box::new(parsed)))
        }
        (KnownAppRequest::Disconnect(_), WalletResult::Object(result)) if result.is_empty() => {
            Ok(KnownWalletResponse::Disconnect)
        }
        (
            KnownAppRequest::SendTransaction(_)
            | KnownAppRequest::SignMessage(_)
            | KnownAppRequest::SignData(_)
            | KnownAppRequest::Disconnect(_),
            WalletResult::String(_) | WalletResult::Object(_),
        ) => Err(ResponseValidationError::InvalidResult),
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ed25519_dalek::{Signer as _, SigningKey};

    use super::*;

    const FRIENDLY_ADDRESS: &str = "Ef8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADAU";

    #[test]
    fn transaction_requires_exactly_one_non_empty_body_kind() {
        let neither = r#"{"network":"-239"}"#;
        let both = r#"{"messages":[{"address":"EQ","amount":"1"}],"items":[{"type":"ton","address":"EQ","amount":"1"}]}"#;
        let empty = r#"{"messages":[]}"#;
        assert!(serde_json::from_str::<TransactionPayload>(neither).is_err());
        assert!(serde_json::from_str::<TransactionPayload>(both).is_err());
        assert!(serde_json::from_str::<TransactionPayload>(empty).is_err());
    }

    #[test]
    fn app_request_decodes_json_string_parameter() {
        let json = r#"{
            "method":"sendTransaction",
            "params":["{\"valid_until\":1764424242,\"network\":\"-239\",\"messages\":[{\"address\":\"Ef8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADAU\",\"amount\":\"100000000\"}]}"],
            "id":"42"
        }"#;
        let decoded = serde_json::from_str::<AppRequest>(json).and_then(|request| {
            request
                .decode()
                .map_err(|error| serde_json::Error::io(std::io::Error::other(error)))
        });
        assert!(matches!(decoded, Ok(KnownAppRequest::SendTransaction(_))));
    }

    #[test]
    fn known_request_rejects_extra_nested_fields() {
        let request = AppRequest {
            method: "signData".to_owned(),
            params: vec![r#"{"type":"text","text":"hello","extra":true}"#.to_owned()],
            id: "1".to_owned(),
        };
        assert!(matches!(request.decode(), Err(RpcError::InvalidPayload(_))));
    }

    #[test]
    fn unknown_method_remains_answerable() {
        let request = AppRequest {
            method: "futureMethod".to_owned(),
            params: Vec::new(),
            id: "7".to_owned(),
        };
        assert!(matches!(
            request.decode(),
            Err(RpcError::UnsupportedMethod(method)) if method == "futureMethod"
        ));
    }

    #[test]
    fn error_catalogue_rejects_unknown_numeric_codes() {
        assert!(serde_json::from_str::<RpcErrorCode>("400").is_ok());
        assert!(serde_json::from_str::<RpcErrorCode>("401").is_err());
    }

    #[test]
    fn response_must_match_request_id_and_method_shape() -> Result<(), Box<dyn std::error::Error>> {
        let request = KnownAppRequest::SendTransaction(SendTransactionRequest {
            id: "42".to_owned(),
            payload: serde_json::from_str(&format!(
                r#"{{"messages":[{{"address":"{FRIENDLY_ADDRESS}","amount":"0"}}]}}"#
            ))?,
        });
        let wrong_id = WalletResponse::Success(WalletResponseSuccess {
            result: WalletResult::String("AA==".to_owned()),
            id: "43".to_owned(),
        });
        let wrong_shape = WalletResponse::Success(WalletResponseSuccess {
            result: WalletResult::Object(Map::new()),
            id: "42".to_owned(),
        });
        assert_eq!(
            wrong_id.validate_for(&request),
            Err(ResponseValidationError::RequestIdMismatch)
        );
        assert_eq!(
            wrong_shape.validate_for(&request),
            Err(ResponseValidationError::InvalidResult)
        );
        Ok(())
    }

    #[test]
    fn sign_data_result_verifies_the_exact_echoed_payload() -> Result<(), SigningError> {
        let signing_key = SigningKey::from_bytes(&[0x55; 32]);
        let public_key = Ed25519PublicKey::from_bytes(signing_key.verifying_key().to_bytes());
        let mut result = SignDataResult {
            signature: Ed25519Signature::from_bytes([0_u8; 64]),
            address: RawAccountAddress::new(0, [0x44; 32]),
            timestamp: 1_700_000_000,
            domain: "example.com".to_owned(),
            payload: SignDataPayload::Text {
                text: "Approve login".to_owned(),
                network: None,
                from: None,
            },
        };
        result.signature =
            Ed25519Signature::from_bytes(signing_key.sign(&result.signing_hash()?).to_bytes());

        assert!(result.verify(&public_key)?);
        result.payload = SignDataPayload::Text {
            text: "Approve transfer".to_owned(),
            network: None,
            from: None,
        };
        assert!(!result.verify(&public_key)?);
        Ok(())
    }

    #[test]
    fn every_normative_rpc_request_variant_decodes() -> Result<(), Box<dyn std::error::Error>> {
        let boc = "te6ccgEBAQEAAgAAAA==";
        let raw = serde_json::json!({
            "valid_until": 1_900_000_000_u64,
            "network": "-239",
            "from": "-1:0000000000000000000000000000000000000000000000000000000000000000",
            "messages": [{
                "address": FRIENDLY_ADDRESS,
                "amount": "1",
                "payload": boc,
                "stateInit": boc
            }]
        });
        let structured = serde_json::json!({
            "items": [
                {"type":"ton","address":FRIENDLY_ADDRESS,"amount":"1"},
                {"type":"jetton","master":FRIENDLY_ADDRESS,"destination":FRIENDLY_ADDRESS,"amount":"2"},
                {"type":"nft","nftAddress":FRIENDLY_ADDRESS,"newOwner":FRIENDLY_ADDRESS}
            ]
        });
        let requests = [
            app_request("sendTransaction", Some(raw.clone()), "1")?,
            app_request("sendTransaction", Some(structured), "2")?,
            app_request("signMessage", Some(raw), "3")?,
            app_request(
                "signData",
                Some(serde_json::json!({"type":"text","text":"hello"})),
                "4",
            )?,
            app_request(
                "signData",
                Some(serde_json::json!({"type":"binary","bytes":"AA=="})),
                "5",
            )?,
            app_request(
                "signData",
                Some(serde_json::json!({
                    "type":"cell",
                    "schema":"value:uint32 = Value",
                    "cell":boc
                })),
                "6",
            )?,
            app_request("disconnect", None, "7")?,
        ];

        for request in requests {
            assert!(request.decode().is_ok());
        }
        Ok(())
    }

    #[test]
    fn every_normative_success_response_matches_its_request()
    -> Result<(), Box<dyn std::error::Error>> {
        let boc = "te6ccgEBAQEAAgAAAA==";
        let transaction = app_request(
            "sendTransaction",
            Some(serde_json::json!({
                "messages":[{"address":FRIENDLY_ADDRESS,"amount":"1"}]
            })),
            "1",
        )?
        .decode()?;
        let sign_message = app_request(
            "signMessage",
            Some(serde_json::json!({
                "messages":[{"address":FRIENDLY_ADDRESS,"amount":"1"}]
            })),
            "2",
        )?
        .decode()?;
        let sign_data = app_request(
            "signData",
            Some(serde_json::json!({"type":"text","text":"hello"})),
            "3",
        )?
        .decode()?;
        let disconnect = app_request("disconnect", None, "4")?.decode()?;
        let signature = STANDARD.encode([0_u8; 64]);
        let responses = [
            (transaction, serde_json::json!({"result":boc,"id":"1"})),
            (
                sign_message,
                serde_json::json!({"result":{"internalBoc":boc},"id":"2"}),
            ),
            (
                sign_data,
                serde_json::json!({
                    "result":{
                        "signature":signature,
                        "address":"0:1111111111111111111111111111111111111111111111111111111111111111",
                        "timestamp":1_800_000_000_u64,
                        "domain":"example.com",
                        "payload":{"type":"text","text":"hello"}
                    },
                    "id":"3"
                }),
            ),
            (disconnect, serde_json::json!({"result":{},"id":"4"})),
        ];

        for (request, response) in responses {
            let response = serde_json::from_value::<WalletResponse>(response)?;
            assert!(response.validate_for(&request).is_ok());
        }
        Ok(())
    }

    #[test]
    fn request_context_enforces_expiry_network_and_fixed_account()
    -> Result<(), Box<dyn std::error::Error>> {
        let active_network = NetworkId::try_from("-239")?;
        let active_account = RawAccountAddress::new(-1, [0_u8; 32]);
        let request = app_request(
            "sendTransaction",
            Some(serde_json::json!({
                "valid_until":100,
                "network":"-239",
                "from":FRIENDLY_ADDRESS,
                "messages":[{"address":FRIENDLY_ADDRESS,"amount":"1"}]
            })),
            "1",
        )?
        .decode()?;
        let KnownAppRequest::SendTransaction(request) = request else {
            return Err("expected sendTransaction".into());
        };
        assert_eq!(
            request
                .payload
                .validate_context(101, &active_network, &active_account),
            Err(RequestContextError::Expired)
        );
        assert!(
            request
                .payload
                .validate_context(100, &active_network, &active_account)
                .is_ok()
        );

        let wrong_network = NetworkId::try_from("-3")?;
        assert_eq!(
            request
                .payload
                .validate_context(99, &wrong_network, &active_account),
            Err(RequestContextError::NetworkMismatch)
        );
        let wrong_account = RawAccountAddress::new(0, [0_u8; 32]);
        assert_eq!(
            request
                .payload
                .validate_context(99, &active_network, &wrong_account),
            Err(RequestContextError::AccountMismatch)
        );
        Ok(())
    }

    fn app_request(
        method: &str,
        payload: Option<Value>,
        id: &str,
    ) -> Result<AppRequest, serde_json::Error> {
        Ok(AppRequest {
            method: method.to_owned(),
            params: payload
                .map(|payload| serde_json::to_string(&payload))
                .transpose()?
                .into_iter()
                .collect(),
            id: id.to_owned(),
        })
    }
}
