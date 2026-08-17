//! Validated one-root TON cell `BoC` used by TON Connect RPC fields.

use std::fmt;

use crc::{CRC_32_ISCSI, Crc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;
use ton_core::{cell::TonCell, traits::tlb::TLB as _};

use crate::{Base64Value, ValueError};

const GENERIC_BOC_MAGIC: [u8; 4] = [0xb5, 0xee, 0x9c, 0x72];
const CRC_32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

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
        let _ = parse_single_root(&bytes)?;
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

/// Validates the untrusted `BoC` envelope before entering `ton_core`.
///
/// `ton_core` 0.1.4 assumes root and reference indices are in range and sizes
/// its allocations from header counters. Checking those invariants here keeps
/// malformed dApp payloads on the ordinary `Err` path instead of allowing an
/// index panic or a header-amplified allocation.
pub(crate) fn parse_single_root(bytes: &[u8]) -> Result<TonCell, CellBocError> {
    validate_single_root_boc(bytes)?;
    TonCell::from_boc(bytes.to_vec()).map_err(|_| CellBocError::InvalidBoc)
}

fn validate_single_root_boc(bytes: &[u8]) -> Result<(), CellBocError> {
    let mut reader = BocReader::new(bytes);
    if reader.take(GENERIC_BOC_MAGIC.len()) != Some(GENERIC_BOC_MAGIC.as_slice()) {
        return Err(CellBocError::InvalidBoc);
    }

    let header = reader.byte().ok_or(CellBocError::InvalidBoc)?;
    let has_index = header & 0x80 != 0;
    let has_crc32c = header & 0x40 != 0;
    let flags = header & 0x18;
    let reference_bytes = usize::from(header & 0x07);
    if flags != 0 || !(1..=4).contains(&reference_bytes) {
        return Err(CellBocError::InvalidBoc);
    }

    let offset_bytes = usize::from(reader.byte().ok_or(CellBocError::InvalidBoc)?);
    if !(1..=8).contains(&offset_bytes) {
        return Err(CellBocError::InvalidBoc);
    }

    let cells = reader
        .unsigned(reference_bytes)
        .ok_or(CellBocError::InvalidBoc)?;
    let roots = reader
        .unsigned(reference_bytes)
        .ok_or(CellBocError::InvalidBoc)?;
    let absent = reader
        .unsigned(reference_bytes)
        .ok_or(CellBocError::InvalidBoc)?;
    let cell_bytes = reader
        .unsigned(offset_bytes)
        .ok_or(CellBocError::InvalidBoc)?;
    if cells == 0 || roots != 1 || absent != 0 || cells > cell_bytes / 2 {
        return Err(CellBocError::InvalidBoc);
    }

    let root = reader
        .unsigned(reference_bytes)
        .ok_or(CellBocError::InvalidBoc)?;
    if root >= cells {
        return Err(CellBocError::InvalidBoc);
    }

    let expected_offsets = if has_index {
        let mut offsets = Vec::with_capacity(cells);
        let mut previous = 0_usize;
        for _ in 0..cells {
            let offset = reader
                .unsigned(offset_bytes)
                .ok_or(CellBocError::InvalidBoc)?;
            if offset <= previous || offset > cell_bytes {
                return Err(CellBocError::InvalidBoc);
            }
            offsets.push(offset);
            previous = offset;
        }
        if previous != cell_bytes {
            return Err(CellBocError::InvalidBoc);
        }
        Some(offsets)
    } else {
        None
    };

    let serialized_cells = reader.take(cell_bytes).ok_or(CellBocError::InvalidBoc)?;
    validate_cells(
        serialized_cells,
        cells,
        reference_bytes,
        expected_offsets.as_deref(),
    )?;

    let checksum_start = reader.position();
    if has_crc32c {
        let checksum = reader.take(4).ok_or(CellBocError::InvalidBoc)?;
        let checksum = <[u8; 4]>::try_from(checksum).map_err(|_| CellBocError::InvalidBoc)?;
        let covered = bytes
            .get(..checksum_start)
            .ok_or(CellBocError::InvalidBoc)?;
        if u32::from_le_bytes(checksum) != CRC_32C.checksum(covered) {
            return Err(CellBocError::InvalidBoc);
        }
    }
    if !reader.is_empty() {
        return Err(CellBocError::InvalidBoc);
    }
    Ok(())
}

fn validate_cells(
    bytes: &[u8],
    cells: usize,
    reference_bytes: usize,
    expected_offsets: Option<&[usize]>,
) -> Result<(), CellBocError> {
    let mut reader = BocReader::new(bytes);
    for cell_index in 0..cells {
        let descriptor = reader.byte().ok_or(CellBocError::InvalidBoc)?;
        let bits_descriptor = reader.byte().ok_or(CellBocError::InvalidBoc)?;
        let references = usize::from(descriptor & 0x07);
        if references > 4 {
            return Err(CellBocError::InvalidBoc);
        }

        if descriptor & 0x10 != 0 {
            let hash_count = usize::try_from((descriptor >> 5).count_ones())
                .ok()
                .and_then(|count| count.checked_add(1))
                .ok_or(CellBocError::InvalidBoc)?;
            let hash_bytes = hash_count.checked_mul(34).ok_or(CellBocError::InvalidBoc)?;
            let _ = reader.take(hash_bytes).ok_or(CellBocError::InvalidBoc)?;
        }

        let data_bytes = usize::from(bits_descriptor >> 1)
            .checked_add(usize::from(bits_descriptor & 1))
            .ok_or(CellBocError::InvalidBoc)?;
        let data = reader.take(data_bytes).ok_or(CellBocError::InvalidBoc)?;
        if bits_descriptor & 1 != 0 && data.last().is_none_or(|byte| *byte == 0) {
            return Err(CellBocError::InvalidBoc);
        }
        if descriptor & 0x08 != 0 && !matches!(data.first().copied(), Some(1_u8..=4_u8)) {
            return Err(CellBocError::InvalidBoc);
        }

        for _ in 0..references {
            let reference = reader
                .unsigned(reference_bytes)
                .ok_or(CellBocError::InvalidBoc)?;
            if reference <= cell_index || reference >= cells {
                return Err(CellBocError::InvalidBoc);
            }
        }

        if expected_offsets
            .and_then(|offsets| offsets.get(cell_index))
            .is_some_and(|expected| *expected != reader.position())
        {
            return Err(CellBocError::InvalidBoc);
        }
    }
    if !reader.is_empty() {
        return Err(CellBocError::InvalidBoc);
    }
    Ok(())
}

struct BocReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BocReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    const fn position(&self) -> usize {
        self.position
    }

    fn is_empty(&self) -> bool {
        self.position == self.bytes.len()
    }

    fn byte(&mut self) -> Option<u8> {
        let value = self.bytes.get(self.position).copied()?;
        self.position = self.position.checked_add(1)?;
        Some(value)
    }

    fn take(&mut self, length: usize) -> Option<&'a [u8]> {
        let end = self.position.checked_add(length)?;
        let value = self.bytes.get(self.position..end)?;
        self.position = end;
        Some(value)
    }

    fn unsigned(&mut self, length: usize) -> Option<usize> {
        let bytes = self.take(length)?;
        bytes.iter().try_fold(0_usize, |value, byte| {
            value.checked_mul(256)?.checked_add(usize::from(*byte))
        })
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use proptest::prelude::*;
    use ton_core::cell::BoC;

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

    #[test]
    fn rejects_out_of_range_root_and_reference_indices_without_panicking() {
        let bad_root = [
            0xb5, 0xee, 0x9c, 0x72, 0x01, 0x01, 0x01, 0x01, 0x00, 0x02, 0x01, 0x00, 0x00,
        ];
        let bad_reference = [
            0xb5, 0xee, 0x9c, 0x72, 0x01, 0x01, 0x01, 0x01, 0x00, 0x03, 0x00, 0x01, 0x00, 0x01,
        ];

        for malformed in [&bad_root[..], &bad_reference[..]] {
            let encoded = STANDARD.encode(malformed);
            assert!(CellBoc::try_from(encoded).is_err());
        }
    }

    #[test]
    fn accepts_indexed_and_crc32c_bocs_and_rejects_a_corrupt_checksum()
    -> Result<(), Box<dyn std::error::Error>> {
        let indexed_empty = [
            0xb5, 0xee, 0x9c, 0x72, 0x81, 0x01, 0x01, 0x01, 0x00, 0x02, 0x00, 0x02, 0x00, 0x00,
        ];
        assert!(CellBoc::try_from(STANDARD.encode(indexed_empty)).is_ok());

        let crc_boc = BoC::new(TonCell::empty().to_owned()).to_bytes(true)?;
        assert!(CellBoc::try_from(STANDARD.encode(&crc_boc)).is_ok());
        let mut corrupt = crc_boc;
        if let Some(last) = corrupt.last_mut() {
            *last ^= 1;
        }
        assert!(CellBoc::try_from(STANDARD.encode(corrupt)).is_err());
        Ok(())
    }

    proptest! {
        #[test]
        fn arbitrary_generic_boc_bytes_never_escape_as_a_panic(
            tail in proptest::collection::vec(any::<u8>(), 0..512)
        ) {
            let mut bytes = GENERIC_BOC_MAGIC.to_vec();
            bytes.extend(tail);
            let encoded = STANDARD.encode(bytes);
            let outcome = std::panic::catch_unwind(|| CellBoc::try_from(encoded));
            prop_assert!(outcome.is_ok());
        }
    }
}
