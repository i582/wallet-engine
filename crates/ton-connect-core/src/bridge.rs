use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;

use crate::{Base64Value, ClientId, TraceId, ValueError};

/// One encrypted message delivered by the HTTP bridge over SSE.
///
/// Deployment-specific fields are preserved because the bridge schema
/// explicitly permits additional properties.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BridgeMessage {
    from: ClientId,
    message: Base64Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<TraceId>,
    #[serde(flatten)]
    extensions: BTreeMap<String, Value>,
}

impl BridgeMessage {
    /// Creates a schema-valid bridge envelope.
    pub fn new(
        from: ClientId,
        message: Base64Value,
        trace_id: Option<TraceId>,
    ) -> Result<Self, ValueError> {
        if message.as_str().is_empty() {
            return Err(ValueError::InvalidBase64);
        }
        Ok(Self {
            from,
            message,
            trace_id,
            extensions: BTreeMap::new(),
        })
    }

    /// Returns the sender's bridge client identifier.
    #[must_use]
    pub const fn from(&self) -> ClientId {
        self.from
    }

    /// Returns the base64-encoded `nonce || ciphertext` payload.
    #[must_use]
    pub const fn message(&self) -> &Base64Value {
        &self.message
    }

    /// Returns the optional analytics correlation identifier.
    #[must_use]
    pub const fn trace_id(&self) -> Option<&TraceId> {
        self.trace_id.as_ref()
    }

    /// Returns deployment-specific bridge fields.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<String, Value> {
        &self.extensions
    }
}

#[derive(Deserialize)]
struct BridgeMessageWire {
    from: ClientId,
    message: Base64Value,
    #[serde(
        default,
        deserialize_with = "crate::value::deserialize_optional_non_null"
    )]
    trace_id: Option<TraceId>,
    #[serde(flatten)]
    extensions: BTreeMap<String, Value>,
}

impl<'de> Deserialize<'de> for BridgeMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BridgeMessageWire::deserialize(deserializer)?;
        if wire.message.as_str().is_empty() {
            return Err(de::Error::custom("bridge message must not be empty"));
        }
        Ok(Self {
            from: wire.from,
            message: wire.message,
            trace_id: wire.trace_id,
            extensions: wire.extensions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_extensions_are_tolerated_and_preserved() {
        let json = r#"{
            "from":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "message":"AA==",
            "deployment":"edge-1"
        }"#;
        let parsed = serde_json::from_str::<BridgeMessage>(json);
        assert_eq!(
            parsed
                .as_ref()
                .ok()
                .and_then(|message| message.extensions().get("deployment")),
            Some(&Value::String("edge-1".to_owned()))
        );
        assert!(
            parsed
                .and_then(|message| serde_json::to_string(&message))
                .is_ok()
        );
    }

    #[test]
    fn bridge_rejects_empty_or_noncanonical_sender() {
        let empty = r#"{"from":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","message":""}"#;
        let uppercase = r#"{"from":"0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef","message":"AA=="}"#;
        assert!(serde_json::from_str::<BridgeMessage>(empty).is_err());
        assert!(serde_json::from_str::<BridgeMessage>(uppercase).is_err());
    }

    #[test]
    fn constructor_and_accessors_preserve_every_bridge_field()
    -> Result<(), Box<dyn std::error::Error>> {
        let from = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".parse()?;
        let message = Base64Value::try_from("AA==")?;
        let trace = TraceId::try_from("019d85ea-ca0e-7129-8155-05c7534ef894")?;
        let envelope = BridgeMessage::new(from, message.clone(), Some(trace.clone()))?;
        assert_eq!(envelope.from(), from);
        assert_eq!(envelope.message(), &message);
        assert_eq!(envelope.trace_id(), Some(&trace));
        assert!(envelope.extensions().is_empty());

        assert!(BridgeMessage::new(from, Base64Value::try_from("")?, None).is_err());
        Ok(())
    }
}
