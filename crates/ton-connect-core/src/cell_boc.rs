//! Validated one-root TON cell `BoC` used by TON Connect RPC fields.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;
use ton_core::{cell::TonCell, traits::tlb::TLB as _};

use crate::{Base64Value, ValueError};

/// Base64 wire value known to contain exactly one valid TON cell root.
#[derive(Clone, Eq, PartialEq)]
pub struct CellBoc {
    encoded: Base64Value,
    bytes: Vec<u8>,
}

impl CellBoc {
    /// Returns the original valid Base64 representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.encoded.as_str()
    }

    /// Returns the validated serialized `BoC` bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl TryFrom<Base64Value> for CellBoc {
    type Error = CellBocError;

    fn try_from(encoded: Base64Value) -> Result<Self, Self::Error> {
        let bytes = encoded.decode().map_err(CellBocError::Base64)?;
        let _ = TonCell::from_boc(bytes.clone()).map_err(|_| CellBocError::InvalidBoc)?;
        Ok(Self { encoded, bytes })
    }
}

impl TryFrom<&str> for CellBoc {
    type Error = CellBocError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Base64Value::try_from(value)
            .map_err(CellBocError::Base64)
            .and_then(Self::try_from)
    }
}

impl TryFrom<String> for CellBoc {
    type Error = CellBocError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl fmt::Debug for CellBoc {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CellBoc")
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl Serialize for CellBoc {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CellBoc {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = Base64Value::deserialize(deserializer)?;
        Self::try_from(encoded).map_err(de::Error::custom)
    }
}

/// Base64 text or decoded bytes are not a valid single-root cell `BoC`.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CellBocError {
    /// Wire text is not accepted Base64.
    #[error(transparent)]
    Base64(ValueError),
    /// Decoded bytes are malformed or contain zero/multiple roots.
    #[error("value must contain a valid single-root TON cell BoC")]
    InvalidBoc,
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    use super::*;

    #[test]
    fn validates_boc_semantics_at_json_boundary() -> Result<(), Box<dyn std::error::Error>> {
        let valid = STANDARD.encode(TonCell::EMPTY_BOC);
        let parsed = serde_json::from_str::<CellBoc>(&serde_json::to_string(&valid)?)?;
        assert_eq!(parsed.as_bytes(), TonCell::EMPTY_BOC);

        let invalid = STANDARD.encode([0_u8; 4]);
        assert!(serde_json::from_str::<CellBoc>(&serde_json::to_string(&invalid)?).is_err());
        Ok(())
    }
}
