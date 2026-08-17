use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::{
    AccountVerificationError, Base64Value, DecimalString, Ed25519PublicKey, Ed25519Signature,
    NetworkId, NonEmptyVec, RawAccountAddress, SignDataSigningPayload, SignatureDomain,
    SigningError, TonAddressItemReply, sign_data_signing_hash, verify_signature,
};

/// Extra-currency identifier to non-negative elementary-unit amount.
pub type ExtraCurrencies = BTreeMap<u32, DecimalString>;

/// A raw outgoing message in `sendTransaction` or `signMessage`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawMessage {
    /// Destination in TEP-2 user-friendly form.
    pub address: String,
    /// Nanocoins represented as a non-negative decimal string.
    pub amount: DecimalString,
    /// Optional base64 one-cell body `BoC`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Base64Value>,
    /// Optional base64 one-cell `StateInit` `BoC`.
    #[serde(rename = "stateInit", skip_serializing_if = "Option::is_none")]
    pub state_init: Option<Base64Value>,
    /// Optional TEP-92 extra currencies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_currency: Option<ExtraCurrencies>,
}

/// A wallet-built structured transfer item.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum StructuredItem {
    /// Native TON transfer.
    Ton {
        /// Destination address.
        address: String,
        /// Nanocoins to transfer.
        amount: DecimalString,
        /// Optional base64 one-cell body `BoC`.
        #[serde(skip_serializing_if = "Option::is_none")]
        payload: Option<Base64Value>,
        /// Optional base64 one-cell `StateInit` `BoC`.
        #[serde(rename = "stateInit", skip_serializing_if = "Option::is_none")]
        state_init: Option<Base64Value>,
        /// Optional TEP-92 extra currencies.
        #[serde(skip_serializing_if = "Option::is_none")]
        extra_currency: Option<ExtraCurrencies>,
    },
    /// TEP-74 jetton transfer.
    Jetton {
        /// Jetton master contract address.
        master: String,
        /// Recipient address.
        destination: String,
        /// Jetton elementary-unit amount.
        amount: DecimalString,
        /// Optional attached TON amount in nanocoins.
        #[serde(rename = "attachAmount", skip_serializing_if = "Option::is_none")]
        attach_amount: Option<DecimalString>,
        /// Optional application query identifier.
        #[serde(rename = "queryId", skip_serializing_if = "Option::is_none")]
        query_id: Option<String>,
        /// Optional excess-TON refund address.
        #[serde(
            rename = "responseDestination",
            skip_serializing_if = "Option::is_none"
        )]
        response_destination: Option<String>,
        /// Optional base64 `custom_payload` cell `BoC`.
        #[serde(rename = "customPayload", skip_serializing_if = "Option::is_none")]
        custom_payload: Option<Base64Value>,
        /// Optional forwarded TON amount in nanocoins.
        #[serde(rename = "forwardAmount", skip_serializing_if = "Option::is_none")]
        forward_amount: Option<DecimalString>,
        /// Optional base64 `forward_payload` cell `BoC`.
        #[serde(rename = "forwardPayload", skip_serializing_if = "Option::is_none")]
        forward_payload: Option<Base64Value>,
    },
    /// TEP-62 NFT transfer.
    Nft {
        /// NFT item contract address.
        #[serde(rename = "nftAddress")]
        nft_address: String,
        /// New owner address.
        #[serde(rename = "newOwner")]
        new_owner: String,
        /// Optional attached TON amount in nanocoins.
        #[serde(rename = "attachAmount", skip_serializing_if = "Option::is_none")]
        attach_amount: Option<DecimalString>,
        /// Optional application query identifier.
        #[serde(rename = "queryId", skip_serializing_if = "Option::is_none")]
        query_id: Option<String>,
        /// Optional excess-TON refund address.
        #[serde(
            rename = "responseDestination",
            skip_serializing_if = "Option::is_none"
        )]
        response_destination: Option<String>,
        /// Optional base64 `custom_payload` cell `BoC`.
        #[serde(rename = "customPayload", skip_serializing_if = "Option::is_none")]
        custom_payload: Option<Base64Value>,
        /// Optional forwarded TON amount in nanocoins.
        #[serde(rename = "forwardAmount", skip_serializing_if = "Option::is_none")]
        forward_amount: Option<DecimalString>,
        /// Optional base64 `forward_payload` cell `BoC`.
        #[serde(rename = "forwardPayload", skip_serializing_if = "Option::is_none")]
        forward_payload: Option<Base64Value>,
    },
}

/// A transaction payload containing caller-built raw messages.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawTransactionPayload {
    /// Optional Unix expiration time in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<u64>,
    /// Optional target network global ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkId>,
    /// Optional fixed sender address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// One or more raw outgoing messages.
    pub messages: NonEmptyVec<RawMessage>,
}

/// A transaction payload containing wallet-built structured items.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredTransactionPayload {
    /// Optional Unix expiration time in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<u64>,
    /// Optional target network global ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkId>,
    /// Optional fixed sender address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// One or more structured transfer items.
    pub items: NonEmptyVec<StructuredItem>,
}

/// Payload shared by `sendTransaction` and `signMessage`.
///
/// The untagged representation enforces the protocol's exclusive choice:
/// exactly one of `messages` or `items` must be present.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum TransactionPayload {
    /// Raw caller-built messages.
    Raw(RawTransactionPayload),
    /// Wallet-built structured items.
    Structured(StructuredTransactionPayload),
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
        #[serde(skip_serializing_if = "Option::is_none")]
        network: Option<NetworkId>,
        /// Optional fixed signer address.
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    /// Opaque binary bytes.
    Binary {
        /// Base64-encoded bytes to sign.
        bytes: Base64Value,
        /// Optional target network global ID.
        #[serde(skip_serializing_if = "Option::is_none")]
        network: Option<NetworkId>,
        /// Optional fixed signer address.
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    /// A TVM cell interpreted with a TL-B schema.
    Cell {
        /// TL-B schema whose final declaration is the root.
        schema: String,
        /// Base64 cell `BoC`.
        cell: Base64Value,
        /// Optional target network global ID.
        #[serde(skip_serializing_if = "Option::is_none")]
        network: Option<NetworkId>,
        /// Optional fixed signer address.
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
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
    pub internal_boc: Base64Value,
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
            SignDataPayload::Cell { schema, cell, .. } => {
                let decoded = cell
                    .decode()
                    .map_err(|_| SigningError::InvalidBase64Payload)?;
                sign_data_signing_hash(
                    &self.address,
                    &self.domain,
                    self.timestamp,
                    SignDataSigningPayload::Cell {
                        schema,
                        boc: &decoded,
                    },
                )
            }
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
    SendTransaction(Base64Value),
    /// Signed internal message.
    SignMessage(SignMessageResult),
    /// Application data signature.
    SignData(SignDataResult),
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
            let boc = Base64Value::try_from(result.clone())
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
            Ok(KnownWalletResponse::SignData(parsed))
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
    use ed25519_dalek::{Signer as _, SigningKey};

    use super::*;

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
            "params":["{\"valid_until\":1764424242,\"network\":\"-239\",\"messages\":[{\"address\":\"EQD...\",\"amount\":\"100000000\"}]}"],
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
            payload: serde_json::from_str(r#"{"messages":[{"address":"EQ","amount":"0"}]}"#)?,
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
}
