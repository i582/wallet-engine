//! Validated 256-bit hashes encoded as Base64.

use std::fmt::{Display, Formatter};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

const HASH_BYTES: usize = 32;

/// A validated 256-bit hash in standard padded Base64.
///
/// Parsing accepts standard and URL-safe Base64 with optional padding. The
/// stored representation is always standard padded Base64.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct Base64Hash(String);

impl Base64Hash {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, Base64HashError> {
        if bytes.len() != HASH_BYTES {
            return Err(Base64HashError);
        }

        Ok(Self(STANDARD.encode(bytes)))
    }

    /// Returns the canonical standard padded Base64 value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Base64Hash {
    type Error = Base64HashError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl TryFrom<&str> for Base64Hash {
    type Error = Base64HashError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let bytes = STANDARD
            .decode(value)
            .or_else(|_| STANDARD_NO_PAD.decode(value))
            .or_else(|_| URL_SAFE.decode(value))
            .or_else(|_| URL_SAFE_NO_PAD.decode(value))
            .map_err(|_| Base64HashError)?;

        Self::from_bytes(&bytes)
    }
}

impl From<Base64Hash> for String {
    fn from(value: Base64Hash) -> Self {
        value.0
    }
}

impl Display for Base64Hash {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Reports a malformed Base64 value or a value that is not 256 bits long.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("hash must be a 256-bit Base64 value")]
pub struct Base64HashError;

#[cfg(feature = "ffi")]
uniffi::custom_type!(Base64Hash, String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_supported_base64_variants() {
        let bytes = [0xfb; HASH_BYTES];
        let expected = STANDARD.encode(bytes);

        for encoded in [
            STANDARD_NO_PAD.encode(bytes),
            URL_SAFE.encode(bytes),
            URL_SAFE_NO_PAD.encode(bytes),
        ] {
            let Ok(hash) = Base64Hash::try_from(encoded) else {
                panic!("valid 256-bit Base64 hash was rejected");
            };
            assert_eq!(hash.as_str(), expected);
        }
    }

    #[test]
    fn rejects_values_that_are_not_256_bits() {
        assert!(Base64Hash::try_from(STANDARD.encode([0_u8; HASH_BYTES - 1])).is_err());
    }

    #[test]
    fn serde_rejects_an_invalid_hash() {
        let Ok(encoded) = serde_json::to_string("not-a-hash") else {
            panic!("string serialization must work");
        };
        assert!(serde_json::from_str::<Base64Hash>(&encoded).is_err());
    }
}
