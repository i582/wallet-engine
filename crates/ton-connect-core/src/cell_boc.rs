//! Validated one-root TON cell `BoC` used by TON Connect RPC fields.

use std::fmt;

use crc::{CRC_32_ISCSI, Crc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;
use ton_core::{cell::TonCell, traits::tlb::TLB as _};

use crate::{Base64Value, ValueError};

pub(crate) const GENERIC_BOC_MAGIC: [u8; 4] = [0xb5, 0xee, 0x9c, 0x72];
const CRC_32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

/// Deepest cell accepted in a TON Connect `BoC`: TON's `vm::CellTraits::max_depth`.
///
/// A cell without references has depth 0; any other cell is one deeper than its deepest
/// reference. The TVM refuses cells deeper than 1024, so no valid cell is lost. `ton_core`
/// drops a parsed cell tree recursively, one stack frame chain per level, and a few thousand
/// levels overflow a 512 KiB thread stack and abort the process; the validation therefore
/// rejects deeper `BoC`s before `ton_core` builds the tree.
pub(crate) const MAX_CELL_DEPTH: u16 = 1024;

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
/// index panic or a header-amplified allocation. It also rejects cells deeper
/// than `MAX_CELL_DEPTH`, whose recursive drop would overflow the stack, and
/// exotic cells whose layout would make `ton_core` panic when hashing them.
pub(crate) fn parse_single_root(bytes: &[u8]) -> Result<TonCell, CellBocError> {
    validate_single_root_boc(bytes)?;
    TonCell::from_boc(bytes.to_vec()).map_err(|_| CellBocError::InvalidBoc)
}

/// Checks that untrusted bytes are one complete single-root cell `BoC` that
/// `ton_core` can parse without panicking, aborting or overflowing the stack.
///
/// Linear in the input and never allocates more than the input length: header
/// counts must be backed by the input, root and reference indices must be in
/// range and point forward, no cell may be deeper than `MAX_CELL_DEPTH`, and
/// every exotic cell has the layout TON requires.
/// It does not build the cell tree; [`CellBoc`] and the wallet engine's own
/// `BoC` type run it before they call `TonCell::from_boc`.
pub fn validate_single_root_boc(bytes: &[u8]) -> Result<(), CellBocError> {
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
        // The index and the cells must both be present, so the header cannot size an
        // allocation beyond the input.
        let needed = cells
            .checked_mul(offset_bytes)
            .and_then(|index_bytes| index_bytes.checked_add(cell_bytes))
            .ok_or(CellBocError::InvalidBoc)?;
        let remaining = bytes
            .len()
            .checked_sub(reader.position())
            .ok_or(CellBocError::InvalidBoc)?;
        if needed > remaining {
            return Err(CellBocError::InvalidBoc);
        }
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

/// Checks every serialized cell and measures the `BoC`'s cell depth in one pass.
///
/// References point strictly forward, so a cell's height (the longest reference
/// path leading to it) is final when the cell is read, and the largest height is
/// the depth of the whole `BoC`. Unreachable cells count too: `ton_core` builds
/// and drops every serialized cell, not only the ones under the root.
fn validate_cells(
    bytes: &[u8],
    cells: usize,
    reference_bytes: usize,
    expected_offsets: Option<&[usize]>,
) -> Result<(), CellBocError> {
    let mut reader = BocReader::new(bytes);
    // `cells <= bytes.len() / 2`, so this is bounded by the input length.
    let mut heights = vec![0_u16; cells];
    for cell_index in 0..cells {
        let descriptor = reader.byte().ok_or(CellBocError::InvalidBoc)?;
        let bits_descriptor = reader.byte().ok_or(CellBocError::InvalidBoc)?;
        let references = usize::from(descriptor & 0x07);
        if references > 4 {
            return Err(CellBocError::InvalidBoc);
        }
        let height = heights
            .get(cell_index)
            .copied()
            .ok_or(CellBocError::InvalidBoc)?;

        if descriptor & 0x10 != 0 {
            let hash_count = usize::try_from((descriptor >> 5).count_ones())
                .ok()
                .ok_or(CellBocError::InvalidBoc)?;
            let hash_count = hash_count.checked_add(1).ok_or(CellBocError::InvalidBoc)?;
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
        if descriptor & 0x08 != 0
            && !exotic_layout_is_valid(descriptor >> 5, bits_descriptor, data, references)
        {
            return Err(CellBocError::InvalidBoc);
        }

        for _ in 0..references {
            let reference = reader
                .unsigned(reference_bytes)
                .ok_or(CellBocError::InvalidBoc)?;
            if reference <= cell_index || reference >= cells {
                return Err(CellBocError::InvalidBoc);
            }
            let child = height
                .checked_add(1)
                .filter(|depth| *depth <= MAX_CELL_DEPTH)
                .ok_or(CellBocError::InvalidBoc)?;
            let slot = heights.get_mut(reference).ok_or(CellBocError::InvalidBoc)?;
            *slot = (*slot).max(child);
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

pub(crate) const PRUNED_BRANCH: u8 = 1;
pub(crate) const LIBRARY: u8 = 2;
pub(crate) const MERKLE_PROOF: u8 = 3;
pub(crate) const MERKLE_UPDATE: u8 = 4;

const TYPE_BYTES: usize = 1;
pub(crate) const HASH_BYTES: usize = 32;
pub(crate) const DEPTH_BYTES: usize = 2;
/// The type byte and the level mask precede a pruned branch's hashes and depths.
pub(crate) const PRUNED_BRANCH_HEADER_BYTES: usize = TYPE_BYTES + 1;
pub(crate) const LIBRARY_BYTES: usize = TYPE_BYTES + HASH_BYTES;
const MERKLE_PROOF_BYTES: usize = TYPE_BYTES + HASH_BYTES + DEPTH_BYTES;
const MERKLE_UPDATE_BYTES: usize = TYPE_BYTES + 2 * (HASH_BYTES + DEPTH_BYTES);

/// Checks an exotic cell against the layout TON's `DataCell::create` requires.
///
/// `ton_core` 0.1.4 hashes parsed exotic cells without validating them and
/// trusts the descriptor's level mask: a pruned branch shorter than that mask
/// panics when hashed, and a stored depth near `u16::MAX` overflows the depth
/// of every cell above it. Library and Merkle cells must match TON's size and
/// reference count too, so no exotic cell TON refuses reaches `ton_core`.
fn exotic_layout_is_valid(
    level_mask: u8,
    bits_descriptor: u8,
    data: &[u8],
    references: usize,
) -> bool {
    // Every TON exotic layout is a whole number of bytes.
    if bits_descriptor & 1 != 0 {
        return false;
    }
    match (data.first().copied(), references) {
        (Some(PRUNED_BRANCH), 0) => pruned_branch_is_valid(level_mask, data),
        (Some(LIBRARY), 0) => data.len() == LIBRARY_BYTES,
        (Some(MERKLE_PROOF), 1) => data.len() == MERKLE_PROOF_BYTES,
        (Some(MERKLE_UPDATE), 2) => data.len() == MERKLE_UPDATE_BYTES,
        _ => false,
    }
}

/// A pruned branch stores one hash and one depth per level of its mask, and
/// `ton_core` locates them by the descriptor's level mask, so the two masks
/// must agree (TON refuses a mismatch as well).
fn pruned_branch_is_valid(level_mask: u8, data: &[u8]) -> bool {
    let Some(mask) = data.get(TYPE_BYTES).copied() else {
        return false;
    };
    if mask == 0 || mask != level_mask {
        return false;
    }
    let Ok(levels) = usize::try_from(mask.count_ones()) else {
        return false;
    };
    let Some(depths) = levels
        .checked_mul(HASH_BYTES)
        .and_then(|hashes| hashes.checked_add(PRUNED_BRANCH_HEADER_BYTES))
        .and_then(|start| data.get(start..))
    else {
        return false;
    };
    levels.checked_mul(DEPTH_BYTES) == Some(depths.len())
        && depths.chunks_exact(DEPTH_BYTES).all(|depth| {
            <[u8; DEPTH_BYTES]>::try_from(depth)
                .is_ok_and(|depth| u16::from_be_bytes(depth) <= MAX_CELL_DEPTH)
        })
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
    use ton_core::cell::{BoC, CellType};

    use super::*;
    use crate::test_boc::{
        REFERENCE_PAST_THE_LAST_CELL, RawCell, boc, chain, chain_cells, filled, level,
        malformed_exotic_bocs, pruned, well_formed_exotic_bocs,
    };

    #[test]
    fn validates_boc_semantics_at_json_boundary() -> Result<(), Box<dyn std::error::Error>> {
        let valid = STANDARD.encode(TonCell::EMPTY_BOC);
        let parsed = serde_json::from_str::<CellBoc>(&serde_json::to_string(&valid)?)?;
        assert_eq!(parsed.as_bytes(), TonCell::EMPTY_BOC);
        let debug = format!("{parsed:?}");
        assert!(debug.contains("bytes: 13"));
        assert!(!debug.contains(&valid));

        let invalid = STANDARD.encode([0_u8; 4]);
        assert!(serde_json::from_str::<CellBoc>(&serde_json::to_string(&invalid)?).is_err());
        Ok(())
    }

    #[test]
    fn rejects_out_of_range_root_and_reference_indices_without_panicking() {
        let bad_root = [
            0xb5, 0xee, 0x9c, 0x72, 0x01, 0x01, 0x01, 0x01, 0x00, 0x02, 0x01, 0x00, 0x00,
        ];
        for malformed in [&bad_root[..], &REFERENCE_PAST_THE_LAST_CELL[..]] {
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

    fn validate(bytes: &[u8]) -> Result<CellBoc, CellBocError> {
        CellBoc::try_from(STANDARD.encode(bytes))
    }

    fn on_stack<T: Send + 'static>(size: usize, job: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(size)
            .spawn(job)
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn accepts_cells_at_the_depth_bound_and_rejects_deeper_ones() {
        // The accepted side builds and drops a 1 025-level tree (~465 KiB in debug).
        let (at_bound, deeper) =
            on_stack(4 << 20, || (validate(&chain(1025)), validate(&chain(1026))));
        assert!(at_bound.is_ok());
        assert_eq!(deeper, Err(CellBocError::InvalidBoc));
    }

    #[test]
    fn rejects_a_deep_chain_on_a_512_kib_stack_before_building_it() {
        let outcome = on_stack(512 * 1024, || validate(&chain(5000)));
        assert_eq!(outcome, Err(CellBocError::InvalidBoc));
    }

    #[test]
    fn measures_depth_over_every_serialized_cell() {
        let with_unreachable_chain = |length: usize| {
            let mut cells = vec![
                RawCell::ordinary(&[], vec![1]),
                RawCell::ordinary(&[], Vec::new()),
            ];
            cells.extend(chain_cells(2, length));
            boc(&cells)
        };
        let (at_bound, deeper) = on_stack(4 << 20, move || {
            (
                validate(&with_unreachable_chain(1025)),
                validate(&with_unreachable_chain(1026)),
            )
        });
        assert!(at_bound.is_ok());
        assert_eq!(deeper, Err(CellBocError::InvalidBoc));
    }

    #[test]
    fn depth_follows_the_longest_reference_path() {
        let mut deep_last = vec![
            RawCell::ordinary(&[], vec![1, 2]),
            RawCell::ordinary(&[], Vec::new()),
        ];
        deep_last.extend(chain_cells(2, 1025));
        assert_eq!(validate(&boc(&deep_last)), Err(CellBocError::InvalidBoc));

        // 4^1024 reference paths: only a linear walk over the cells finishes.
        let ladder = (0..1025_usize)
            .map(|index| {
                let next = index.checked_add(1).unwrap();
                let references = if next < 1025 {
                    vec![next; 4]
                } else {
                    Vec::new()
                };
                RawCell::ordinary(&[], references)
            })
            .collect::<Vec<_>>();
        let outcome = on_stack(4 << 20, move || validate(&boc(&ladder)));
        assert!(outcome.is_ok());
    }

    #[test]
    fn rejects_an_index_larger_than_the_input() {
        let mut overflowing = GENERIC_BOC_MAGIC.to_vec();
        overflowing.extend_from_slice(&[0x84, 0x08]);
        overflowing.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]);
        overflowing.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        overflowing.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        overflowing.extend_from_slice(&u64::MAX.to_be_bytes());
        overflowing.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        assert_eq!(validate(&overflowing), Err(CellBocError::InvalidBoc));

        let mut truncated = overflowing;
        let cell_bytes = truncated.get_mut(18..26).unwrap();
        cell_bytes.copy_from_slice(&0x2_0000_0000_u64.to_be_bytes());
        assert_eq!(validate(&truncated), Err(CellBocError::InvalidBoc));
    }

    #[test]
    fn accepts_well_formed_exotic_cells() -> Result<(), Box<dyn std::error::Error>> {
        for (name, bytes, hashes) in well_formed_exotic_bocs() {
            let parsed = validate(&bytes).map_err(|error| format!("{name}: {error}"))?;
            assert_eq!(parsed.as_bytes(), bytes.as_slice(), "{name}");
            if hashes {
                let _ = TonCell::from_boc(bytes)?.hash()?;
            }
        }

        // Cells `ton_core` builds itself, independent of the fixtures above.
        for (cell_type, data) in [
            (CellType::PrunedBranch, pruned(1, &[5])),
            (CellType::LibraryRef, filled(LIBRARY, LIBRARY_BYTES)),
        ] {
            let mut builder = TonCell::builder_extra(cell_type, data.len());
            builder.write_bits(&data, data.len().checked_mul(8).unwrap())?;
            let cell = builder.build()?;
            let bytes = cell.to_boc()?;
            assert!(validate(&bytes).is_ok(), "{cell_type:?}");
            assert_eq!(TonCell::from_boc(bytes)?.hash()?, cell.hash()?);
        }
        Ok(())
    }

    #[test]
    fn rejects_malformed_exotic_layouts() {
        for (name, bytes) in malformed_exotic_bocs() {
            assert_eq!(validate(&bytes), Err(CellBocError::InvalidBoc), "{name}");
        }
    }

    /// What `exotic_heavy_boc` draws for one cell.
    #[derive(Clone, Debug)]
    struct CellDraw {
        exotic: bool,
        level: u8,
        length: usize,
        /// Replaces the first data byte when set.
        type_byte: Option<u8>,
        /// Replaces the second data byte when set; `None` uses `level`.
        mask: Option<u8>,
        depths: Vec<u16>,
        bytes: Vec<u8>,
        references: Vec<prop::sample::Index>,
    }

    prop_compose! {
        fn exotic_heavy_cell()(
            exotic in prop::bool::weighted(0.75),
            level in prop_oneof![
                2 => Just(0_u8), 2 => Just(1), 1 => Just(2), 1 => Just(3), 1 => Just(7),
                1 => 0..8_u8,
            ],
            length in prop_oneof![
                Just(0_usize), Just(1), Just(2), Just(3), Just(33), Just(35), Just(36),
                Just(69), Just(70), Just(104), 0..128_usize,
            ],
            type_byte in prop_oneof![
                2 => Just(Some(1_u8)), 1 => Just(Some(2)), 1 => Just(Some(3)),
                1 => Just(Some(4)), 1 => Just(None),
            ],
            mask in prop_oneof![
                Just(Some(1_u8)), Just(Some(3)), Just(Some(7)), Just(None),
                (0..9_u8).prop_map(Some),
            ],
            depths in prop::collection::vec(
                prop::sample::select(&[0_u16, 5, 1024, 1025, u16::MAX][..]),
                3,
            ),
            bytes in prop::collection::vec(any::<u8>(), 128),
            references in prop::collection::vec(any::<prop::sample::Index>(), 0..=4),
        ) -> CellDraw {
            CellDraw { exotic, level, length, type_byte, mask, depths, bytes, references }
        }
    }

    /// Lays out cell `index` of `count`: stored depths go where a pruned
    /// branch of the drawn mask keeps them, references point to later cells.
    fn drawn_cell(index: usize, count: usize, draw: CellDraw) -> RawCell {
        let mut data = draw.bytes;
        data.truncate(draw.length);
        if let (Some(first), Some(type_byte)) = (data.first_mut(), draw.type_byte) {
            *first = type_byte;
        }
        if let Some(mask) = data.get_mut(1) {
            *mask = draw.mask.unwrap_or(draw.level);
        }
        let levels = data
            .get(1)
            .map_or(0, |mask| usize::try_from(mask.count_ones()).unwrap());
        let mut start = levels
            .checked_mul(HASH_BYTES)
            .and_then(|hashes| hashes.checked_add(PRUNED_BRANCH_HEADER_BYTES))
            .unwrap();
        for depth in draw.depths.iter().take(levels) {
            let end = start.checked_add(DEPTH_BYTES).unwrap();
            if let Some(slot) = data.get_mut(start..end) {
                slot.copy_from_slice(&depth.to_be_bytes());
            }
            start = end;
        }
        let first_later = index.checked_add(1).unwrap();
        let later = count.checked_sub(first_later).unwrap();
        let references = draw
            .references
            .iter()
            .take(later)
            .map(|reference| first_later.checked_add(reference.index(later)).unwrap())
            .collect::<Vec<_>>();
        if draw.exotic {
            RawCell::exotic(draw.level, data, references)
        } else {
            RawCell {
                descriptor: level(draw.level),
                ..RawCell::ordinary(&data, references)
            }
        }
    }

    /// One to four cells with exotic-heavy descriptors, lengths, masks and
    /// stored depths.
    fn exotic_heavy_boc() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(exotic_heavy_cell(), 1..=4).prop_map(|draws| {
            let count = draws.len();
            let cells = draws
                .into_iter()
                .enumerate()
                .map(|(index, draw)| drawn_cell(index, count, draw))
                .collect::<Vec<_>>();
            boc(&cells)
        })
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

        #[test]
        fn accepted_exotic_bocs_hash_without_panicking(bytes in exotic_heavy_boc()) {
            if validate_single_root_boc(&bytes).is_ok() {
                // A typed `ton_core` error is fine; a panic is what the validation prevents.
                let outcome = std::panic::catch_unwind(|| {
                    TonCell::from_boc(bytes.clone()).and_then(|cell| cell.hash().map(|_| ()))
                });
                prop_assert!(outcome.is_ok(), "{bytes:02x?}");
            }
        }
    }
}
