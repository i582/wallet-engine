//! Validated Bag of Cells bytes used by signed wallet messages.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserializer, Serialize, Serializer, de::Error as _};
use ton::ton_core::cell::TonCell;
use ton::ton_core::errors::TonCoreError;
use ton::ton_core::traits::tlb::TLB;
use ton_connect_core::{CellBocError, validate_single_root_boc};

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
        // `ton_core` sizes allocations from header counts, indexes references
        // unchecked and drops the tree recursively; validate the envelope first.
        validate_single_root_boc(&bytes)
            .map_err(|error| BocError(BocErrorKind::Envelope(error)))?;
        let _ =
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

impl BocError {
    /// Whether the envelope validation refused the bytes before `ton_core` saw them.
    #[cfg(test)]
    pub(crate) const fn rejected_before_ton_core(&self) -> bool {
        matches!(self.0, BocErrorKind::Envelope(_))
    }
}

#[derive(Debug, thiserror::Error)]
enum BocErrorKind {
    #[error("invalid Base64")]
    Base64(#[source] base64::DecodeError),
    #[error("invalid BOC envelope")]
    Envelope(#[source] CellBocError),
    #[error("invalid BOC")]
    Ton(#[source] TonCoreError),
}

uniffi::custom_type!(Boc, String);

#[cfg(test)]
mod tests {
    use ton_connect_core::test_boc;

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

    /// Asserts `bytes` are refused before `ton_core` sees them on every text
    /// boundary: direct conversion, JSON, and the UniFFI lift.
    fn assert_refused_on_every_boundary(name: &str, bytes: &[u8]) {
        use uniffi::{Lift, Lower};

        let encoded = STANDARD.encode(bytes);
        let direct = Boc::try_from(encoded.clone()).expect_err(name);
        assert!(direct.rejected_before_ton_core(), "{name}: {direct:?}");

        let json = serde_json::to_string(&encoded).expect("string serialization must work");
        let error = serde_json::from_str::<Boc>(&json).expect_err(name);
        assert!(
            error.to_string().contains("invalid single-root BOC"),
            "{name}: {error}"
        );

        let lowered = <String as Lower<crate::UniFfiTag>>::lower(encoded);
        assert!(
            <Boc as Lift<crate::UniFfiTag>>::try_lift(lowered).is_err(),
            "{name}"
        );
    }

    #[test]
    fn untrusted_boc_text_is_validated_before_ton_core_on_every_boundary() {
        assert_refused_on_every_boundary("huge cell count", &test_boc::huge_cell_count());
        assert_refused_on_every_boundary(
            "reference past the last cell",
            &test_boc::REFERENCE_PAST_THE_LAST_CELL,
        );
        assert_refused_on_every_boundary(
            "state init over a truncated pruned branch",
            &test_boc::STATE_INIT_WITH_TRUNCATED_PRUNED_CODE,
        );
        // `ton_core` panics hashing some of these; none may reach it.
        for (name, bytes) in test_boc::malformed_exotic_bocs() {
            assert_refused_on_every_boundary(name, &bytes);
        }
    }

    #[test]
    fn well_formed_exotic_cells_are_valid_bocs() {
        for (name, bytes, hashes) in test_boc::well_formed_exotic_bocs() {
            let boc = Boc::try_from(bytes.clone()).expect(&name);
            assert_eq!(boc.as_bytes(), bytes.as_slice(), "{name}");
            if hashes {
                let cell = TonCell::from_boc(bytes).expect("a valid BOC must parse");
                assert!(cell.hash().is_ok(), "{name}");
            }
        }
    }

    #[test]
    fn consuming_conversion_returns_the_complete_base64_boc() {
        let boc = Boc::try_from(TonCell::EMPTY_BOC.to_vec())
            .expect("the TON empty-cell BOC must be valid");
        let expected = boc.to_base64();

        assert_eq!(String::from(boc), expected);
    }
}
