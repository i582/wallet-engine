//! Validated Bag of Cells bytes used by signed wallet messages.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserializer, Serialize, Serializer, de::Error as _};
use ton::ton_core::cell::TonCell;
use ton::ton_core::errors::TonCoreError;
use ton::ton_core::traits::tlb::TLB;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Boc(Vec<u8>);

impl Boc {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the standard padded Base64 boundary representation.
    #[must_use]
    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.as_bytes())
    }
}

impl TryFrom<String> for Boc {
    type Error = BocError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let bytes = STANDARD
            .decode(value)
            .map_err(|error| BocError(BocErrorKind::Base64(error)))?;
        Self::try_from(bytes)
    }
}

impl TryFrom<&str> for Boc {
    type Error = BocError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl TryFrom<Vec<u8>> for Boc {
    type Error = BocError;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        TonCell::from_boc(bytes.clone()).map_err(|error| BocError(BocErrorKind::Ton(error)))?;
        Ok(Self(bytes))
    }
}

impl Serialize for Boc {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_base64())
    }
}

impl<'de> serde::Deserialize<'de> for Boc {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::try_from(encoded).map_err(D::Error::custom)
    }
}

impl From<Boc> for String {
    fn from(value: Boc) -> Self {
        value.to_base64()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid single-root BOC")]
pub struct BocError(#[source] BocErrorKind);

#[derive(Debug, thiserror::Error)]
enum BocErrorKind {
    #[error("invalid Base64")]
    Base64(#[source] base64::DecodeError),
    #[error("invalid BOC")]
    Ton(#[source] TonCoreError),
}

uniffi::custom_type!(Boc, String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_preserves_a_valid_boc() {
        let boc = Boc::try_from(TonCell::EMPTY_BOC.to_vec())
            .expect("the TON empty-cell BOC must be valid");
        let encoded = serde_json::to_string(&boc).expect("a valid BOC must serialize");
        let decoded =
            serde_json::from_str::<Boc>(&encoded).expect("a serialized BOC must deserialize");

        assert_eq!(decoded, boc);
    }

    #[test]
    fn serde_rejects_invalid_boc_bytes() {
        let encoded = serde_json::to_string(&STANDARD.encode([0_u8; 4]))
            .expect("string serialization must work");
        assert!(serde_json::from_str::<Boc>(&encoded).is_err());
    }
}
