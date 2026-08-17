//! Runtime capability checks for requests sent to a connected wallet.

use thiserror::Error;

use crate::{
    DeviceInfo, EmbeddedRequest, Feature, KnownAppRequest, SignDataType, StructuredItemType,
    TransactionPayload,
};

impl DeviceInfo {
    /// Checks that this runtime `DeviceInfo` advertises every capability needed
    /// by an ordinary RPC request.
    ///
    /// The connected wallet's `DeviceInfo.features` is authoritative. Calling
    /// this before `send()` prevents an SDK from presenting an operation that
    /// the wallet has already declared it cannot process.
    pub fn validate_request(&self, request: &KnownAppRequest) -> Result<(), CapabilityError> {
        match request {
            KnownAppRequest::SendTransaction(request) => {
                self.validate_transaction("sendTransaction", &request.payload, false)
            }
            KnownAppRequest::SignMessage(request) => {
                self.validate_transaction("signMessage", &request.payload, true)
            }
            KnownAppRequest::SignData(request) => {
                let required = request.payload.data_type();
                let supported = self.features.iter().find_map(|feature| match feature {
                    Feature::SignData(feature) => Some(feature.types()),
                    Feature::LegacySendTransaction
                    | Feature::SendTransaction(_)
                    | Feature::SignMessage(_)
                    | Feature::EmbeddedRequest => None,
                });
                let Some(supported) = supported else {
                    return Err(CapabilityError::MethodNotSupported("signData"));
                };
                if !supported.contains(&required) {
                    return Err(CapabilityError::SignDataTypeNotSupported(required));
                }
                Ok(())
            }
            // Disconnect is part of the session lifecycle and has no feature
            // entry of its own in DeviceInfo.
            KnownAppRequest::Disconnect(_) => Ok(()),
        }
    }

    /// Checks both the `EmbeddedRequest` feature and the underlying method's
    /// runtime capability.
    pub fn validate_embedded_request(
        &self,
        request: &EmbeddedRequest,
    ) -> Result<(), CapabilityError> {
        if !self
            .features
            .iter()
            .any(|feature| matches!(feature, Feature::EmbeddedRequest))
        {
            return Err(CapabilityError::EmbeddedRequestNotSupported);
        }

        match request {
            EmbeddedRequest::SendTransaction(payload) => {
                self.validate_transaction("sendTransaction", payload, false)
            }
            EmbeddedRequest::SignMessage(payload) => {
                self.validate_transaction("signMessage", payload, true)
            }
            EmbeddedRequest::SignData(payload) => {
                let required = payload.data_type();
                let supported = self.features.iter().find_map(|feature| match feature {
                    Feature::SignData(feature) => Some(feature.types()),
                    Feature::LegacySendTransaction
                    | Feature::SendTransaction(_)
                    | Feature::SignMessage(_)
                    | Feature::EmbeddedRequest => None,
                });
                let Some(supported) = supported else {
                    return Err(CapabilityError::MethodNotSupported("signData"));
                };
                if !supported.contains(&required) {
                    return Err(CapabilityError::SignDataTypeNotSupported(required));
                }
                Ok(())
            }
        }
    }

    fn validate_transaction(
        &self,
        method: &'static str,
        payload: &TransactionPayload,
        sign_only: bool,
    ) -> Result<(), CapabilityError> {
        let detailed = self.features.iter().find_map(|feature| match feature {
            Feature::SendTransaction(feature) if !sign_only => Some((
                feature.max_messages(),
                feature.extra_currency_supported(),
                feature.item_types(),
            )),
            Feature::SignMessage(feature) if sign_only => Some((
                feature.max_messages(),
                feature.extra_currency_supported(),
                feature.item_types(),
            )),
            Feature::LegacySendTransaction
            | Feature::SendTransaction(_)
            | Feature::SignData(_)
            | Feature::SignMessage(_)
            | Feature::EmbeddedRequest => None,
        });

        let Some((max_messages, extra_currency_supported, item_types)) = detailed else {
            // The deprecated string form predates explicit limits. It remains
            // usable only for a basic raw sendTransaction, matching the
            // compatibility behaviour of the reference SDK. Structured items
            // and extra currencies require an explicit modern capability.
            let legacy_send = !sign_only
                && self
                    .features
                    .iter()
                    .any(|feature| matches!(feature, Feature::LegacySendTransaction));
            if !legacy_send {
                return Err(CapabilityError::MethodNotSupported(method));
            }
            if payload.structured_item_types().is_some() {
                return Err(CapabilityError::StructuredItemsNotSupported);
            }
            if payload.uses_extra_currency() {
                return Err(CapabilityError::ExtraCurrencyNotSupported);
            }
            return Ok(());
        };

        if u64::try_from(payload.message_count())
            .is_ok_and(|actual| actual > u64::from(max_messages))
        {
            return Err(CapabilityError::MessageLimitExceeded {
                max_messages,
                actual: payload.message_count(),
            });
        }
        if payload.uses_extra_currency() && extra_currency_supported != Some(true) {
            return Err(CapabilityError::ExtraCurrencyNotSupported);
        }
        if let Some(required_types) = payload.structured_item_types() {
            let Some(supported_types) = item_types else {
                return Err(CapabilityError::StructuredItemsNotSupported);
            };
            if let Some(unsupported) = required_types
                .into_iter()
                .find(|required| !supported_types.contains(required))
            {
                return Err(CapabilityError::StructuredItemTypeNotSupported(unsupported));
            }
        }
        Ok(())
    }
}

/// A request exceeds the capabilities advertised by the connected wallet.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum CapabilityError {
    /// The wallet did not advertise the requested RPC method.
    #[error("wallet does not advertise TON Connect method {0}")]
    MethodNotSupported(&'static str),
    /// The batch is larger than the runtime-advertised limit.
    #[error("request has {actual} messages, but wallet advertises at most {max_messages}")]
    MessageLimitExceeded {
        /// Runtime-advertised maximum.
        max_messages: u32,
        /// Number of messages or items in the request.
        actual: usize,
    },
    /// The request carries extra currencies without explicit wallet support.
    #[error("wallet does not advertise extra-currency support")]
    ExtraCurrencyNotSupported,
    /// The wallet only advertises raw message input for this method.
    #[error("wallet does not advertise structured transaction items")]
    StructuredItemsNotSupported,
    /// A structured item kind is absent from the runtime capability.
    #[error("wallet does not advertise structured item type {0:?}")]
    StructuredItemTypeNotSupported(StructuredItemType),
    /// A `signData` payload kind is absent from the runtime capability.
    #[error("wallet does not advertise signData type {0:?}")]
    SignDataTypeNotSupported(SignDataType),
    /// The wallet does not advertise one-tap embedded requests.
    #[error("wallet does not advertise embedded TON Connect requests")]
    EmbeddedRequestNotSupported,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppRequest, DevicePlatform};

    const FRIENDLY: &str = "Ef8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADAU";

    fn request(method: &str, payload: &serde_json::Value) -> KnownAppRequest {
        AppRequest {
            method: method.to_owned(),
            params: vec![payload.to_string()],
            id: "1".to_owned(),
        }
        .decode()
        .expect("capability fixture is a valid request")
    }

    fn device(features: &serde_json::Value) -> DeviceInfo {
        serde_json::from_value(serde_json::json!({
            "platform": "browser",
            "appName": "example",
            "appVersion": "1.0.0",
            "maxProtocolVersion": 2,
            "features": features,
        }))
        .expect("capability fixture is valid DeviceInfo")
    }

    #[test]
    fn enforces_message_limit_and_extra_currency_support() {
        let wallet = device(&serde_json::json!([{
            "name": "SendTransaction",
            "maxMessages": 1
        }]));
        let two_messages = request(
            "sendTransaction",
            &serde_json::json!({
                "messages": [
                    { "address": FRIENDLY, "amount": "1" },
                    { "address": FRIENDLY, "amount": "2" }
                ]
            }),
        );
        assert!(matches!(
            wallet.validate_request(&two_messages),
            Err(CapabilityError::MessageLimitExceeded {
                max_messages: 1,
                actual: 2
            })
        ));

        let extra_payload = serde_json::json!({
            "messages": [{
                "address": FRIENDLY,
                "amount": "1",
                "extra_currency": { "1": "5" }
            }]
        });
        let _: crate::RawTransactionPayload = serde_json::from_value(extra_payload.clone())
            .expect("raw extra-currency fixture is valid");
        let extra_currency = request("sendTransaction", &extra_payload);
        assert_eq!(
            wallet.validate_request(&extra_currency),
            Err(CapabilityError::ExtraCurrencyNotSupported)
        );
    }

    #[test]
    fn enforces_structured_and_sign_data_types() {
        let wallet = device(&serde_json::json!([
            { "name": "SendTransaction", "maxMessages": 4, "itemTypes": ["ton"] },
            { "name": "SignData", "types": ["text"] }
        ]));
        let jetton = request(
            "sendTransaction",
            &serde_json::json!({
                "items": [{
                    "type": "jetton",
                    "master": FRIENDLY,
                    "destination": FRIENDLY,
                    "amount": "1"
                }]
            }),
        );
        assert_eq!(
            wallet.validate_request(&jetton),
            Err(CapabilityError::StructuredItemTypeNotSupported(
                StructuredItemType::Jetton
            ))
        );

        let binary = request(
            "signData",
            &serde_json::json!({ "type": "binary", "bytes": "AA==" }),
        );
        assert_eq!(
            wallet.validate_request(&binary),
            Err(CapabilityError::SignDataTypeNotSupported(
                SignDataType::Binary
            ))
        );
    }

    #[test]
    fn embedded_requests_require_both_capabilities() {
        let wallet = DeviceInfo {
            platform: DevicePlatform::Browser,
            app_name: "example".to_owned(),
            app_version: "1.0.0".to_owned(),
            max_protocol_version: 2,
            features: vec![Feature::SignData(
                crate::SignDataFeature::new(vec![SignDataType::Text]).expect("feature is valid"),
            )],
        };
        let request = EmbeddedRequest::SignData(crate::SignDataPayload::Text {
            text: "hello".to_owned(),
            network: None,
            from: None,
        });
        assert_eq!(
            wallet.validate_embedded_request(&request),
            Err(CapabilityError::EmbeddedRequestNotSupported)
        );
    }
}
