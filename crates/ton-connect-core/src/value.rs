use std::{fmt, ops::Deref, str::FromStr};

use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

/// A failure to validate a protocol scalar value.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ValueError {
    /// A bridge client identifier was not 32 lowercase-hex bytes.
    #[error("client id must be exactly 64 lowercase hexadecimal characters")]
    InvalidClientId,
    /// A network ID was not a stringified signed decimal integer.
    #[error("network id must be a stringified signed decimal integer")]
    InvalidNetworkId,
    /// A decimal string contained no digits or a non-decimal character.
    #[error("decimal string must contain one or more ASCII digits")]
    InvalidDecimalString,
    /// A value was not valid standard or URL-safe base64.
    #[error("value must be valid standard or URL-safe base64")]
    InvalidBase64,
    /// A trace identifier did not have the UUID wire shape.
    #[error("trace id must use the canonical UUID text shape")]
    InvalidTraceId,
    /// A URL was not an absolute HTTPS URL with a host.
    #[error("URL must be an absolute HTTPS URL with a host")]
    InvalidHttpsUrl,
    /// A protocol array that requires at least one element was empty.
    #[error("array must contain at least one element")]
    EmptyArray,
}

/// A 32-byte X25519 public key encoded as 64 lowercase hexadecimal characters.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClientId([u8; 32]);

impl ClientId {
    /// Creates an identifier from the public-key bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the public-key bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for ClientId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for ClientId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl FromStr for ClientId {
    type Err = ValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64
            || !value
                .as_bytes()
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(ValueError::InvalidClientId);
        }

        let mut bytes = [0_u8; 32];
        hex::decode_to_slice(value, &mut bytes).map_err(|_| ValueError::InvalidClientId)?;
        Ok(Self(bytes))
    }
}

impl Serialize for ClientId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ClientId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(de::Error::custom)
    }
}

macro_rules! validated_string {
    ($name:ident, $validator:ident, $error:expr, $docs:literal) => {
        #[doc = $docs]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Returns the validated wire representation.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consumes the value and returns its wire representation.
            #[must_use]
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = ValueError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                if $validator(&value) {
                    Ok(Self(value))
                } else {
                    Err($error)
                }
            }
        }

        impl TryFrom<&str> for $name {
            type Error = ValueError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::try_from(value.to_owned())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::try_from(value).map_err(de::Error::custom)
            }
        }
    };
}

fn valid_network_id(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.as_bytes().iter().all(u8::is_ascii_digit)
}

fn valid_decimal(value: &str) -> bool {
    !value.is_empty() && value.as_bytes().iter().all(u8::is_ascii_digit)
}

fn decode_base64(value: &str) -> Result<Vec<u8>, ValueError> {
    let has_standard = value.bytes().any(|byte| matches!(byte, b'+' | b'/'));
    let has_url_safe = value.bytes().any(|byte| matches!(byte, b'-' | b'_'));
    if has_standard && has_url_safe {
        return Err(ValueError::InvalidBase64);
    }

    let padded = value.ends_with('=');
    let decoded = match (has_url_safe, padded) {
        (true, true) => general_purpose::URL_SAFE.decode(value),
        (true, false) => general_purpose::URL_SAFE_NO_PAD.decode(value),
        (false, true) => general_purpose::STANDARD.decode(value),
        (false, false) => general_purpose::STANDARD_NO_PAD.decode(value),
    };
    decoded.map_err(|_| ValueError::InvalidBase64)
}

fn valid_base64(value: &str) -> bool {
    decode_base64(value).is_ok()
}

fn valid_trace_id(value: &str) -> bool {
    let mut parts = value.split('-');
    let lengths = [8_usize, 4, 4, 4, 12];
    for expected in lengths {
        let Some(part) = parts.next() else {
            return false;
        };
        if part.len() != expected || !part.as_bytes().iter().all(u8::is_ascii_hexdigit) {
            return false;
        }
    }
    parts.next().is_none()
}

fn valid_https_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| url.scheme() == "https" && url.has_host())
}

validated_string!(
    NetworkId,
    valid_network_id,
    ValueError::InvalidNetworkId,
    "A TON network `global_id` encoded as a signed decimal string."
);
validated_string!(
    DecimalString,
    valid_decimal,
    ValueError::InvalidDecimalString,
    "A non-negative integer in the decimal-string wire representation."
);
validated_string!(
    Base64Value,
    valid_base64,
    ValueError::InvalidBase64,
    "A validated standard or URL-safe base64 wire value."
);
validated_string!(
    TraceId,
    valid_trace_id,
    ValueError::InvalidTraceId,
    "A UUID-shaped trace identifier accepted by the TON Connect bridge schema."
);
validated_string!(
    HttpsUrl,
    valid_https_url,
    ValueError::InvalidHttpsUrl,
    "An absolute HTTPS URL with a host."
);

impl HttpsUrl {
    /// Parses the validated URL for component inspection.
    pub fn parsed(&self) -> Result<url::Url, ValueError> {
        url::Url::parse(self.as_str()).map_err(|_| ValueError::InvalidHttpsUrl)
    }
}

impl Base64Value {
    /// Decodes the validated value.
    pub fn decode(&self) -> Result<Vec<u8>, ValueError> {
        decode_base64(self.as_str())
    }
}

/// A protocol array statically known to contain at least one value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct NonEmptyVec<T>(Vec<T>);

impl<T> NonEmptyVec<T> {
    /// Returns the contained values.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }

    /// Consumes the wrapper and returns the contained values.
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }

    /// Maps every element while preserving the non-empty invariant.
    #[must_use]
    pub fn map<U, F>(self, map: F) -> NonEmptyVec<U>
    where
        F: FnMut(T) -> U,
    {
        NonEmptyVec(self.0.into_iter().map(map).collect())
    }

    /// Maps borrowed elements while preserving the non-empty invariant.
    #[must_use]
    pub fn map_ref<'a, U, F>(&'a self, map: F) -> NonEmptyVec<U>
    where
        F: FnMut(&'a T) -> U,
    {
        NonEmptyVec(self.0.iter().map(map).collect())
    }
}

impl<T> TryFrom<Vec<T>> for NonEmptyVec<T> {
    type Error = ValueError;

    fn try_from(values: Vec<T>) -> Result<Self, Self::Error> {
        if values.is_empty() {
            Err(ValueError::EmptyArray)
        } else {
            Ok(Self(values))
        }
    }
}

impl<'de, T> Deserialize<'de> for NonEmptyVec<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<T>::deserialize(deserializer)?;
        Self::try_from(values).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn client_id_requires_canonical_lowercase_hex() {
        let canonical = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(ClientId::from_str(canonical).is_ok());
        assert!(ClientId::from_str(&canonical.to_uppercase()).is_err());
        assert!(ClientId::from_str("00").is_err());
    }

    #[test]
    fn network_id_preserves_custom_global_id() {
        let parsed = NetworkId::try_from("-1234567890");
        assert_eq!(
            parsed.map(NetworkId::into_string),
            Ok("-1234567890".to_owned())
        );
        assert!(NetworkId::try_from("+3").is_err());
        assert!(NetworkId::try_from("-").is_err());
    }

    #[test]
    fn base64_accepts_both_protocol_alphabets() {
        assert!(Base64Value::try_from("+/8=").is_ok());
        assert!(Base64Value::try_from("-_8").is_ok());
        assert!(Base64Value::try_from("+/8_").is_err());
        assert!(Base64Value::try_from("a==").is_err());
    }

    #[test]
    fn non_empty_vec_rejects_empty_json_array() {
        let result = serde_json::from_str::<NonEmptyVec<String>>("[]");
        assert!(result.is_err());
    }

    proptest! {
        #[test]
        fn every_client_id_has_a_canonical_json_round_trip(bytes in any::<[u8; 32]>()) {
            let original = ClientId::from_bytes(bytes);
            let encoded = serde_json::to_string(&original);
            let decoded = encoded.and_then(|json| serde_json::from_str::<ClientId>(&json));
            prop_assert_eq!(decoded.ok(), Some(original));
        }

        #[test]
        fn every_non_empty_ascii_digit_sequence_is_a_decimal_string(
            digits in proptest::collection::vec(b'0'..=b'9', 1..512)
        ) {
            let text = digits.into_iter().map(char::from).collect::<String>();
            let value = DecimalString::try_from(text.clone());
            prop_assert_eq!(value.map(DecimalString::into_string), Ok(text));
        }
    }
}
