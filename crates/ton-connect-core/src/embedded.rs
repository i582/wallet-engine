use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;
use thiserror::Error;

use crate::{
    AccountAddress, Base64Value, CellBoc, DecimalString, ExtraCurrencies, FriendlyAddress,
    KnownAppRequest, KnownWalletResponse, NetworkId, NonEmptyVec, RawMessage,
    RawTransactionPayload, ResponseValidationError, SendTransactionRequest, SignDataPayload,
    SignDataRequest, SignMessageRequest, StructuredItem, StructuredTransactionPayload,
    TransactionPayload, WalletResponse, WalletResponseError, WalletResponseSuccess, WalletResult,
};

/// An RPC request embedded in a connect link, before a request ID exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmbeddedRequest {
    /// Sign and broadcast a transaction after connection approval.
    SendTransaction(TransactionPayload),
    /// Sign a message without broadcasting after connection approval.
    SignMessage(TransactionPayload),
    /// Sign application data after connection approval.
    SignData(SignDataPayload),
}

/// Method-specific response attached to a successful connect event.
///
/// Embedded requests have no dApp-assigned request ID, so this deliberately
/// differs from the normal [`crate::WalletResponse`] envelope.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EmbeddedResponse {
    /// Embedded action completed successfully.
    Success(EmbeddedResponseSuccess),
    /// Embedded action returned an error while connect still succeeded.
    Error(EmbeddedResponseError),
}

impl EmbeddedResponse {
    /// Validates the result shape and error catalogue against the embedded
    /// method that produced it.
    ///
    /// Embedded responses omit only the ordinary RPC `id`; every other result
    /// rule is identical, so validation deliberately reuses the normal
    /// request/response correlator with an internal empty identifier.
    pub fn validate_for(
        &self,
        request: &EmbeddedRequest,
    ) -> Result<KnownWalletResponse, ResponseValidationError> {
        let request = match request {
            EmbeddedRequest::SendTransaction(payload) => {
                KnownAppRequest::SendTransaction(SendTransactionRequest {
                    id: String::new(),
                    payload: payload.clone(),
                })
            }
            EmbeddedRequest::SignMessage(payload) => {
                KnownAppRequest::SignMessage(SignMessageRequest {
                    id: String::new(),
                    payload: payload.clone(),
                })
            }
            EmbeddedRequest::SignData(payload) => KnownAppRequest::SignData(SignDataRequest {
                id: String::new(),
                payload: payload.clone(),
            }),
        };
        let response = match self {
            Self::Success(response) => WalletResponse::Success(WalletResponseSuccess {
                result: response.result.clone(),
                id: String::new(),
            }),
            Self::Error(response) => WalletResponse::Error {
                error: response.error.clone(),
                id: String::new(),
            },
        };
        response.validate_for(&request)
    }
}

/// Success body for an embedded action.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedResponseSuccess {
    /// Method-specific result identical to a normal wallet response result.
    pub result: WalletResult,
}

/// Error body for an embedded action.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedResponseError {
    /// Method-specific error identical to a normal wallet response error.
    pub error: WalletResponseError,
}

/// Failure to decode or encode the compact `e` connect-link parameter.
#[derive(Debug, Error)]
pub enum EmbeddedRequestError {
    /// The parameter is not unpadded URL-safe base64.
    #[error("embedded request must be unpadded URL-safe base64")]
    InvalidBase64,
    /// The decoded JSON violates the compact embedded-request schema.
    #[error("invalid embedded TON Connect request: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

/// Decodes the unpadded base64url `e` connect-link parameter.
pub fn decode_embedded_request_param(
    parameter: &str,
) -> Result<EmbeddedRequest, EmbeddedRequestError> {
    if parameter.is_empty() || parameter.contains('=') {
        return Err(EmbeddedRequestError::InvalidBase64);
    }
    let json = general_purpose::URL_SAFE_NO_PAD
        .decode(parameter)
        .map_err(|_| EmbeddedRequestError::InvalidBase64)?;
    let wire = serde_json::from_slice::<WireEmbeddedRequest>(&json)?;
    Ok(wire.into())
}

/// Encodes a request as the unpadded base64url `e` parameter.
pub fn encode_embedded_request_param(
    request: &EmbeddedRequest,
) -> Result<String, EmbeddedRequestError> {
    let wire = WireEmbeddedRequest::from(request);
    let json = serde_json::to_vec(&wire)?;
    Ok(general_purpose::URL_SAFE_NO_PAD.encode(json))
}

#[derive(Clone, Copy, Deserialize, Serialize)]
enum WireTransactionMethod {
    #[serde(rename = "st")]
    SendTransaction,
    #[serde(rename = "sm")]
    SignMessage,
}

#[derive(Deserialize, Serialize)]
struct WireTransactionRequest {
    m: WireTransactionMethod,
    #[serde(flatten)]
    payload: WireTransactionPayload,
}

#[derive(Serialize)]
#[serde(untagged)]
enum WireTransactionPayload {
    Raw(WireRawTransaction),
    Structured(WireStructuredTransaction),
}

impl<'de> Deserialize<'de> for WireTransactionPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let Value::Object(object) = &value else {
            return Err(de::Error::custom(
                "embedded transaction payload must be an object",
            ));
        };
        match (object.contains_key("ms"), object.contains_key("i")) {
            (true, false) => serde_json::from_value::<WireRawTransaction>(value)
                .map(Self::Raw)
                .map_err(de::Error::custom),
            (false, true) => serde_json::from_value::<WireStructuredTransaction>(value)
                .map(Self::Structured)
                .map_err(de::Error::custom),
            (true, true) | (false, false) => Err(de::Error::custom(
                "embedded transaction requires exactly one of ms or i",
            )),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireRawTransaction {
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    f: Option<AccountAddress>,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    n: Option<NetworkId>,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    vu: Option<u64>,
    ms: NonEmptyVec<WireMessage>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireStructuredTransaction {
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    f: Option<AccountAddress>,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    n: Option<NetworkId>,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    vu: Option<u64>,
    i: NonEmptyVec<WireItem>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WireMessage {
    a: FriendlyAddress,
    am: DecimalString,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    p: Option<CellBoc>,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    si: Option<CellBoc>,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    ec: Option<ExtraCurrencies>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "t", rename_all = "lowercase", deny_unknown_fields)]
enum WireItem {
    Ton {
        a: FriendlyAddress,
        am: DecimalString,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        p: Option<CellBoc>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        si: Option<CellBoc>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        ec: Option<ExtraCurrencies>,
    },
    Jetton {
        ma: AccountAddress,
        d: AccountAddress,
        am: DecimalString,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        aa: Option<DecimalString>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        qi: Option<String>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        rd: Option<AccountAddress>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        cp: Option<CellBoc>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        fa: Option<DecimalString>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        fp: Option<CellBoc>,
    },
    Nft {
        na: AccountAddress,
        no: AccountAddress,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        aa: Option<DecimalString>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        qi: Option<String>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        rd: Option<AccountAddress>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        cp: Option<CellBoc>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        fa: Option<DecimalString>,
        #[serde(
            default,
            deserialize_with = "crate::value::deserialize_optional_non_null",
            skip_serializing_if = "Option::is_none"
        )]
        fp: Option<CellBoc>,
    },
}

#[derive(Clone, Copy, Deserialize, Serialize)]
enum WireSignDataMethod {
    #[serde(rename = "sd")]
    SignData,
}

#[derive(Deserialize, Serialize)]
struct WireSignDataRequest {
    m: WireSignDataMethod,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    n: Option<NetworkId>,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    f: Option<AccountAddress>,
    #[serde(flatten)]
    payload: WireSignDataPayload,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "t", rename_all = "lowercase", deny_unknown_fields)]
enum WireSignDataPayload {
    Text { tx: String },
    Binary { b: Base64Value },
    Cell { s: String, c: CellBoc },
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum WireEmbeddedRequest {
    Transaction(WireTransactionRequest),
    SignData(WireSignDataRequest),
}

impl From<WireMessage> for RawMessage {
    fn from(message: WireMessage) -> Self {
        Self {
            address: message.a,
            amount: message.am,
            payload: message.p,
            state_init: message.si,
            extra_currency: message.ec,
        }
    }
}

impl From<&RawMessage> for WireMessage {
    fn from(message: &RawMessage) -> Self {
        Self {
            a: message.address.clone(),
            am: message.amount.clone(),
            p: message.payload.clone(),
            si: message.state_init.clone(),
            ec: message.extra_currency.clone(),
        }
    }
}

impl From<WireItem> for StructuredItem {
    fn from(item: WireItem) -> Self {
        match item {
            WireItem::Ton { a, am, p, si, ec } => Self::Ton {
                address: a,
                amount: am,
                payload: p,
                state_init: si,
                extra_currency: ec,
            },
            WireItem::Jetton {
                ma,
                d,
                am,
                aa,
                qi,
                rd,
                cp,
                fa,
                fp,
            } => Self::Jetton {
                master: ma,
                destination: d,
                amount: am,
                attach_amount: aa,
                query_id: qi,
                response_destination: rd,
                custom_payload: cp,
                forward_amount: fa,
                forward_payload: fp,
            },
            WireItem::Nft {
                na,
                no,
                aa,
                qi,
                rd,
                cp,
                fa,
                fp,
            } => Self::Nft {
                nft_address: na,
                new_owner: no,
                attach_amount: aa,
                query_id: qi,
                response_destination: rd,
                custom_payload: cp,
                forward_amount: fa,
                forward_payload: fp,
            },
        }
    }
}

impl From<&StructuredItem> for WireItem {
    fn from(item: &StructuredItem) -> Self {
        match item {
            StructuredItem::Ton {
                address,
                amount,
                payload,
                state_init,
                extra_currency,
            } => Self::Ton {
                a: address.clone(),
                am: amount.clone(),
                p: payload.clone(),
                si: state_init.clone(),
                ec: extra_currency.clone(),
            },
            StructuredItem::Jetton {
                master,
                destination,
                amount,
                attach_amount,
                query_id,
                response_destination,
                custom_payload,
                forward_amount,
                forward_payload,
            } => Self::Jetton {
                ma: master.clone(),
                d: destination.clone(),
                am: amount.clone(),
                aa: attach_amount.clone(),
                qi: query_id.clone(),
                rd: response_destination.clone(),
                cp: custom_payload.clone(),
                fa: forward_amount.clone(),
                fp: forward_payload.clone(),
            },
            StructuredItem::Nft {
                nft_address,
                new_owner,
                attach_amount,
                query_id,
                response_destination,
                custom_payload,
                forward_amount,
                forward_payload,
            } => Self::Nft {
                na: nft_address.clone(),
                no: new_owner.clone(),
                aa: attach_amount.clone(),
                qi: query_id.clone(),
                rd: response_destination.clone(),
                cp: custom_payload.clone(),
                fa: forward_amount.clone(),
                fp: forward_payload.clone(),
            },
        }
    }
}

impl From<WireTransactionPayload> for TransactionPayload {
    fn from(payload: WireTransactionPayload) -> Self {
        match payload {
            WireTransactionPayload::Raw(payload) => Self::Raw(RawTransactionPayload {
                valid_until: payload.vu,
                network: payload.n,
                from: payload.f,
                messages: payload.ms.map(Into::into),
            }),
            WireTransactionPayload::Structured(payload) => {
                Self::Structured(StructuredTransactionPayload {
                    valid_until: payload.vu,
                    network: payload.n,
                    from: payload.f,
                    items: payload.i.map(Into::into),
                })
            }
        }
    }
}

impl From<&TransactionPayload> for WireTransactionPayload {
    fn from(payload: &TransactionPayload) -> Self {
        match payload {
            TransactionPayload::Raw(payload) => Self::Raw(WireRawTransaction {
                f: payload.from.clone(),
                n: payload.network.clone(),
                vu: payload.valid_until,
                ms: payload.messages.map_ref(WireMessage::from),
            }),
            TransactionPayload::Structured(payload) => {
                Self::Structured(WireStructuredTransaction {
                    f: payload.from.clone(),
                    n: payload.network.clone(),
                    vu: payload.valid_until,
                    i: payload.items.map_ref(WireItem::from),
                })
            }
        }
    }
}

impl From<WireEmbeddedRequest> for EmbeddedRequest {
    fn from(request: WireEmbeddedRequest) -> Self {
        match request {
            WireEmbeddedRequest::Transaction(request) => match request.m {
                WireTransactionMethod::SendTransaction => {
                    Self::SendTransaction(request.payload.into())
                }
                WireTransactionMethod::SignMessage => Self::SignMessage(request.payload.into()),
            },
            WireEmbeddedRequest::SignData(request) => {
                let payload = match request.payload {
                    WireSignDataPayload::Text { tx } => SignDataPayload::Text {
                        text: tx,
                        network: request.n,
                        from: request.f,
                    },
                    WireSignDataPayload::Binary { b } => SignDataPayload::Binary {
                        bytes: b,
                        network: request.n,
                        from: request.f,
                    },
                    WireSignDataPayload::Cell { s, c } => SignDataPayload::Cell {
                        schema: s,
                        cell: c,
                        network: request.n,
                        from: request.f,
                    },
                };
                Self::SignData(payload)
            }
        }
    }
}

impl From<&EmbeddedRequest> for WireEmbeddedRequest {
    fn from(request: &EmbeddedRequest) -> Self {
        match request {
            EmbeddedRequest::SendTransaction(payload) => {
                Self::Transaction(WireTransactionRequest {
                    m: WireTransactionMethod::SendTransaction,
                    payload: payload.into(),
                })
            }
            EmbeddedRequest::SignMessage(payload) => Self::Transaction(WireTransactionRequest {
                m: WireTransactionMethod::SignMessage,
                payload: payload.into(),
            }),
            EmbeddedRequest::SignData(payload) => {
                let (n, f, payload) = match payload {
                    SignDataPayload::Text {
                        text,
                        network,
                        from,
                    } => (
                        network.clone(),
                        from.clone(),
                        WireSignDataPayload::Text { tx: text.clone() },
                    ),
                    SignDataPayload::Binary {
                        bytes,
                        network,
                        from,
                    } => (
                        network.clone(),
                        from.clone(),
                        WireSignDataPayload::Binary { b: bytes.clone() },
                    ),
                    SignDataPayload::Cell {
                        schema,
                        cell,
                        network,
                        from,
                    } => (
                        network.clone(),
                        from.clone(),
                        WireSignDataPayload::Cell {
                            s: schema.clone(),
                            c: cell.clone(),
                        },
                    ),
                };
                Self::SignData(WireSignDataRequest {
                    m: WireSignDataMethod::SignData,
                    n,
                    f,
                    payload,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_typescript_send_transaction_vector() {
        let wire = r#"{"m":"st","n":"-239","vu":1761071945,"ms":[{"a":"EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs","am":"1000000000"}]}"#;
        let parameter = general_purpose::URL_SAFE_NO_PAD.encode(wire);
        assert!(matches!(
            decode_embedded_request_param(&parameter),
            Ok(EmbeddedRequest::SendTransaction(TransactionPayload::Raw(
                RawTransactionPayload {
                    valid_until: Some(1_761_071_945),
                    ..
                }
            )))
        ));
    }

    #[test]
    fn rejects_padded_or_ambiguous_transaction_payloads() {
        let both = r#"{"m":"st","ms":[{"a":"EQ","am":"1"}],"i":[{"t":"ton","a":"EQ","am":"1"}]}"#;
        let parameter = general_purpose::URL_SAFE_NO_PAD.encode(both);
        assert!(decode_embedded_request_param(&parameter).is_err());
        assert!(decode_embedded_request_param("e30=").is_err());
    }

    #[test]
    fn every_supported_embedded_shape_round_trips() {
        let samples = [
            concat!(
                "{\"m\":\"sm\",\"ms\":[{\"a\":\"",
                "Ef8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADAU",
                "\",\"am\":\"0\"}]}"
            ),
            r#"{"m":"sd","n":"-239","t":"text","tx":"Hello"}"#,
            r#"{"m":"sd","t":"binary","b":"AA=="}"#,
            r#"{"m":"sd","t":"cell","s":"value:uint32 = Value","c":"te6ccgEBAQEAAgAAAA=="}"#,
        ];
        for sample in samples {
            let parameter = general_purpose::URL_SAFE_NO_PAD.encode(sample);
            let round_trip = decode_embedded_request_param(&parameter)
                .and_then(|request| encode_embedded_request_param(&request))
                .and_then(|encoded| decode_embedded_request_param(&encoded));
            assert!(
                round_trip.is_ok(),
                "failed embedded sample {sample}: {round_trip:?}"
            );
        }
    }

    #[test]
    fn embedded_response_has_no_request_id() -> Result<(), Box<dyn std::error::Error>> {
        let success = EmbeddedResponse::Success(EmbeddedResponseSuccess {
            result: WalletResult::Object(serde_json::Map::new()),
        });
        assert_eq!(serde_json::to_string(&success)?, r#"{"result":{}}"#);
        assert!(serde_json::from_str::<EmbeddedResponse>(r#"{"result":{},"id":"1"}"#).is_err());

        let error = EmbeddedResponse::Error(EmbeddedResponseError {
            error: WalletResponseError {
                code: crate::RpcErrorCode::UserDeclined,
                message: "declined".to_owned(),
                data: None,
            },
        });
        assert_eq!(
            serde_json::to_string(&error)?,
            r#"{"error":{"code":300,"message":"declined"}}"#
        );
        Ok(())
    }
}
