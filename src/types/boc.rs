//! Validated Bag of Cells bytes used by signed wallet messages.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserializer, Serialize, Serializer, de::Error as _};
use ton::ton_core::cell::TonCell;
use ton::ton_core::errors::TonCoreError;
use ton::ton_core::traits::tlb::TLB;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Boc(Vec<u8>);

impl Boc {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<Vec<u8>> for Boc {
    type Error = BocError;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        TonCell::from_boc(bytes.clone()).map_err(BocError)?;
        Ok(Self(bytes))
    }
}

impl Serialize for Boc {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(self.as_bytes()))
    }
}

impl<'de> serde::Deserialize<'de> for Boc {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = <String as serde::Deserialize>::deserialize(deserializer)?;
        let bytes = STANDARD.decode(encoded).map_err(D::Error::custom)?;
        Self::try_from(bytes).map_err(D::Error::custom)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("invalid single-root BOC")]
pub(crate) struct BocError(#[source] TonCoreError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_preserves_a_valid_boc() {
        let Ok(boc) = Boc::try_from(TonCell::EMPTY_BOC.to_vec()) else {
            panic!("the TON empty-cell BOC must be valid");
        };
        let Ok(encoded) = serde_json::to_string(&boc) else {
            panic!("a valid BOC must serialize");
        };
        let Ok(decoded) = serde_json::from_str::<Boc>(&encoded) else {
            panic!("a serialized BOC must deserialize");
        };

        assert_eq!(decoded, boc);
    }

    #[test]
    fn serde_rejects_invalid_boc_bytes() {
        let Ok(encoded) = serde_json::to_string(&STANDARD.encode([0_u8; 4])) else {
            panic!("string serialization must work");
        };
        assert!(serde_json::from_str::<Boc>(&encoded).is_err());
    }
}
