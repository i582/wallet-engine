//! Hand-written `BoC` bytes for tests of untrusted-input handling.
//!
//! Most of these shapes cannot come from `ton_core`: it refuses malformed
//! cells, and building or dropping a tree deeper than the TVM allows is the
//! very recursion the validation guards against. Build well-formed cells with
//! `TonCell::builder` instead.
//!
//! Compiled for this crate's tests and, with the `test-support` feature, for
//! the tests of dependent crates. It is not part of the stable API.

#![allow(
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use,
    clippy::too_many_lines,
    reason = "fixtures panic when a test describes an impossible fixture"
)]

use ton_core::cell::{LevelMask, TonCell};
use ton_core::traits::tlb::TLB as _;

use crate::cell_boc::{
    GENERIC_BOC_MAGIC, HASH_BYTES, LIBRARY, LIBRARY_BYTES, MAX_CELL_DEPTH, MERKLE_PROOF,
    MERKLE_UPDATE, PRUNED_BRANCH,
};

/// One serialized cell of a hand-written `BoC`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RawCell {
    /// First descriptor byte without the reference count: the level mask in
    /// the top three bits and the exotic flag `0x08`.
    pub descriptor: u8,
    /// Second descriptor byte: `floor(bits / 8) + ceil(bits / 8)`.
    pub bits_descriptor: u8,
    /// Data bytes, ending in the completion tag when `bits_descriptor` is odd.
    pub data: Vec<u8>,
    /// Indices of the referenced cells, each greater than this cell's own.
    pub references: Vec<usize>,
}

impl RawCell {
    /// An ordinary level-0 cell of whole data bytes.
    pub fn ordinary(data: &[u8], references: Vec<usize>) -> Self {
        Self {
            descriptor: 0,
            bits_descriptor: whole_bytes(data.len()),
            data: data.to_vec(),
            references,
        }
    }

    /// An exotic cell of whole data bytes with the given level mask.
    pub fn exotic(level_mask: u8, data: Vec<u8>, references: Vec<usize>) -> Self {
        Self {
            descriptor: level(level_mask) | 0x08,
            bits_descriptor: whole_bytes(data.len()),
            data,
            references,
        }
    }

    /// The same cell with its references moved by `by`, for appending it after `by` other cells.
    pub fn shifted(self, by: usize) -> Self {
        let references = self
            .references
            .into_iter()
            .map(|reference| reference.checked_add(by).expect("small fixture"))
            .collect();
        Self { references, ..self }
    }
}

/// The descriptor bits of a cell with the given level mask.
pub fn level(mask: u8) -> u8 {
    mask.checked_mul(32).expect("a level mask has three bits")
}

fn whole_bytes(length: usize) -> u8 {
    u8::try_from(length.checked_mul(2).expect("small fixture")).expect("a cell has 128 bytes")
}

/// Serializes `cells` as a one-root `BoC` rooted at cell 0: 2-byte references,
/// 4-byte offsets, no index and no CRC-32C.
pub fn boc(cells: &[RawCell]) -> Vec<u8> {
    let mut body = Vec::new();
    for cell in cells {
        let references = u8::try_from(cell.references.len()).expect("small fixture");
        body.push(cell.descriptor | references);
        body.push(cell.bits_descriptor);
        body.extend_from_slice(&cell.data);
        for reference in &cell.references {
            body.extend_from_slice(&index(*reference).to_be_bytes());
        }
    }
    let mut bytes = GENERIC_BOC_MAGIC.to_vec();
    bytes.extend_from_slice(&[0x02, 0x04]);
    bytes.extend_from_slice(&index(cells.len()).to_be_bytes());
    bytes.extend_from_slice(&[0x00, 0x01, 0x00, 0x00]);
    bytes.extend_from_slice(
        &u32::try_from(body.len())
            .expect("small fixture")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(&[0x00, 0x00]);
    bytes.extend(body);
    bytes
}

fn index(value: usize) -> u16 {
    u16::try_from(value).expect("a fixture has at most 65535 cells")
}

/// Data-less cells `start..start + length`, each referencing the next one.
pub fn chain_cells(start: usize, length: usize) -> Vec<RawCell> {
    let end = start.checked_add(length).expect("small fixture");
    (start..end)
        .map(|position| {
            let next = position.checked_add(1).expect("small fixture");
            let references = if next < end { vec![next] } else { Vec::new() };
            RawCell::ordinary(&[], references)
        })
        .collect()
}

/// A `BoC` of `length` data-less cells, each referencing the next one: its
/// root is `length - 1` levels deep.
pub fn chain(length: usize) -> Vec<u8> {
    boc(&chain_cells(0, length))
}

/// One pruned branch whose descriptor (`0x28`) claims level 1 while its data
/// (bits descriptor `0x02`) is only the type byte `01`: `ton_core` 0.1.4
/// panics when it hashes it.
pub const TRUNCATED_PRUNED_BRANCH: [u8; 14] = [
    0xb5, 0xee, 0x9c, 0x72, 0x01, 0x01, 0x01, 0x01, 0x00, 0x03, 0x00, 0x28, 0x02, 0x01,
];

/// A `StateInit` whose `code` is `TRUNCATED_PRUNED_BRANCH`'s cell, so hashing
/// the `StateInit` hashes that cell.
pub const STATE_INIT_WITH_TRUNCATED_PRUNED_CODE: [u8; 18] = [
    0xb5, 0xee, 0x9c, 0x72, 0x01, 0x01, 0x02, 0x01, 0x00, 0x07, 0x00, 0x21, 0x01, 0x24, 0x01, 0x28,
    0x02, 0x01,
];

/// One cell whose only reference names cell 1, past the last cell.
pub const REFERENCE_PAST_THE_LAST_CELL: [u8; 14] = [
    0xb5, 0xee, 0x9c, 0x72, 0x01, 0x01, 0x01, 0x01, 0x00, 0x03, 0x00, 0x01, 0x00, 0x01,
];

/// No index, 4-byte counts, 2^32 - 1 cells claimed in 23 bytes.
pub fn huge_cell_count() -> Vec<u8> {
    let mut bytes = GENERIC_BOC_MAGIC.to_vec();
    bytes.extend_from_slice(&[0x04, 0x01]);
    bytes.extend_from_slice(&u32::MAX.to_be_bytes());
    bytes.extend_from_slice(&1_u32.to_be_bytes());
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes.push(0xff);
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes
}

/// Pruned branch data: type byte, mask, then one hash and one stored depth
/// per level of the mask.
pub fn pruned(mask: u8, depths: &[u16]) -> Vec<u8> {
    assert_eq!(
        depths.len(),
        usize::try_from(mask.count_ones()).expect("at most 8"),
        "one stored depth per level"
    );
    let mut data = vec![PRUNED_BRANCH, mask];
    for _ in depths {
        data.extend_from_slice(&[0x11; HASH_BYTES]);
    }
    for depth in depths {
        data.extend_from_slice(&depth.to_be_bytes());
    }
    data
}

/// `length` bytes of filler whose first byte is `type_byte`.
pub fn filled(type_byte: u8, length: usize) -> Vec<u8> {
    let mut data = vec![0x5a; length];
    if let Some(first) = data.first_mut() {
        *first = type_byte;
    }
    data
}

/// An ordinary level-1 cell over a mask-1 pruned branch, as a Merkle proof
/// carries it, and the level-0 hash and depth TON stores for that child.
fn merkle_child() -> ([RawCell; 2], Vec<u8>, Vec<u8>) {
    let child = [
        RawCell {
            descriptor: level(1),
            ..RawCell::ordinary(&[], vec![1])
        },
        RawCell::exotic(1, pruned(1, &[5]), Vec::new()),
    ];
    let root = TonCell::from_boc(boc(&child)).expect("the Merkle child parses");
    let hash = root
        .hash_for_level(LevelMask::new(0))
        .expect("the Merkle child hashes")
        .as_slice()
        .to_vec();
    let depth = root
        .depth_for_level(LevelMask::new(0))
        .expect("the Merkle child has a depth");
    (child, hash, depth.to_be_bytes().to_vec())
}

/// Well-formed exotic cells as `(name, BoC, ton_core can hash it)`.
///
/// `ton_core` reads `level()` hashes, one per level up to the highest, so it
/// hashes only pruned branches whose levels are contiguous from 1.
pub fn well_formed_exotic_bocs() -> Vec<(String, Vec<u8>, bool)> {
    let mut bocs = Vec::new();
    for mask in 1..=7_u8 {
        let levels = usize::try_from(mask.count_ones()).expect("at most 8");
        let depths = vec![MAX_CELL_DEPTH; levels];
        let cell = RawCell::exotic(mask, pruned(mask, &depths), Vec::new());
        bocs.push((
            format!("pruned mask {mask}"),
            boc(&[cell]),
            matches!(mask, 1 | 3 | 7),
        ));
    }
    let library = RawCell::exotic(0, filled(LIBRARY, LIBRARY_BYTES), Vec::new());
    bocs.push(("library".to_owned(), boc(&[library]), true));

    let (child, hash, depth) = merkle_child();
    let proof = RawCell::exotic(0, [&[MERKLE_PROOF][..], &hash, &depth].concat(), vec![1]);
    let cells = std::iter::once(proof)
        .chain(child.clone().map(|cell| cell.shifted(1)))
        .collect::<Vec<_>>();
    bocs.push(("merkle proof".to_owned(), boc(&cells), true));

    let update = RawCell::exotic(
        0,
        [&[MERKLE_UPDATE][..], &hash, &hash, &depth, &depth].concat(),
        vec![1, 3],
    );
    let cells = std::iter::once(update)
        .chain(child.clone().map(|cell| cell.shifted(1)))
        .chain(child.map(|cell| cell.shifted(3)))
        .collect::<Vec<_>>();
    bocs.push(("merkle update".to_owned(), boc(&cells), true));
    bocs
}

/// `cell` alone.
fn alone(cell: RawCell) -> Vec<u8> {
    boc(&[cell])
}

/// `cell` followed by one ordinary leaf per reference it has.
fn over_leaves(cell: RawCell) -> Vec<u8> {
    let leaves = cell.references.len();
    let mut cells = vec![cell];
    cells.extend((0..leaves).map(|_| RawCell::ordinary(&[], Vec::new())));
    boc(&cells)
}

/// Exotic cells whose layout TON refuses, as `(name, BoC)`.
pub fn malformed_exotic_bocs() -> Vec<(&'static str, Vec<u8>)> {
    let one = pruned(1, &[5]);
    let mut short = one.clone();
    let _ = short.pop();
    let mut long = one.clone();
    long.push(0);
    // The mask-1 layout (stored depth 0x0080) read as 35 whole bytes and a
    // completion-tagged last byte: not byte-aligned.
    let unaligned = RawCell {
        bits_descriptor: 71,
        ..RawCell::exotic(1, pruned(1, &[0x80]), Vec::new())
    };
    vec![
        (
            "the truncated pruned branch",
            TRUNCATED_PRUNED_BRANCH.to_vec(),
        ),
        (
            "pruned level 1, type byte only",
            alone(RawCell::exotic(1, vec![PRUNED_BRANCH], Vec::new())),
        ),
        (
            "pruned mask 0",
            alone(RawCell::exotic(0, vec![PRUNED_BRANCH, 0], Vec::new())),
        ),
        (
            "pruned level 0 over mask 1",
            alone(RawCell::exotic(0, one.clone(), Vec::new())),
        ),
        (
            "pruned level 1 over mask 3",
            alone(RawCell::exotic(1, pruned(3, &[5, 5]), Vec::new())),
        ),
        (
            "pruned one byte short",
            alone(RawCell::exotic(1, short, Vec::new())),
        ),
        (
            "pruned one byte long",
            alone(RawCell::exotic(1, long, Vec::new())),
        ),
        (
            "pruned with a reference",
            over_leaves(RawCell::exotic(1, one, vec![1])),
        ),
        (
            "pruned stored depth 1025",
            alone(RawCell::exotic(1, pruned(1, &[1025]), Vec::new())),
        ),
        (
            "pruned second stored depth 1025",
            alone(RawCell::exotic(3, pruned(3, &[5, 1025]), Vec::new())),
        ),
        ("pruned not byte-aligned", alone(unaligned)),
        (
            "library, 32 bytes",
            alone(RawCell::exotic(0, filled(LIBRARY, 32), Vec::new())),
        ),
        (
            "library, 34 bytes",
            alone(RawCell::exotic(0, filled(LIBRARY, 34), Vec::new())),
        ),
        (
            "library with a reference",
            over_leaves(RawCell::exotic(0, filled(LIBRARY, 33), vec![1])),
        ),
        (
            "merkle proof, 34 bytes",
            over_leaves(RawCell::exotic(0, filled(MERKLE_PROOF, 34), vec![1])),
        ),
        (
            "merkle proof, 36 bytes",
            over_leaves(RawCell::exotic(0, filled(MERKLE_PROOF, 36), vec![1])),
        ),
        (
            "merkle proof without a reference",
            alone(RawCell::exotic(0, filled(MERKLE_PROOF, 35), Vec::new())),
        ),
        (
            "merkle proof with two references",
            over_leaves(RawCell::exotic(0, filled(MERKLE_PROOF, 35), vec![1, 2])),
        ),
        (
            "merkle update, 68 bytes",
            over_leaves(RawCell::exotic(0, filled(MERKLE_UPDATE, 68), vec![1, 2])),
        ),
        (
            "merkle update, 70 bytes",
            over_leaves(RawCell::exotic(0, filled(MERKLE_UPDATE, 70), vec![1, 2])),
        ),
        (
            "merkle update with one reference",
            over_leaves(RawCell::exotic(0, filled(MERKLE_UPDATE, 69), vec![1])),
        ),
        (
            "merkle update with three references",
            over_leaves(RawCell::exotic(0, filled(MERKLE_UPDATE, 69), vec![1, 2, 3])),
        ),
        (
            "exotic without a type byte",
            alone(RawCell::exotic(0, Vec::new(), Vec::new())),
        ),
        (
            "exotic type byte 0",
            alone(RawCell::exotic(0, filled(0, 33), Vec::new())),
        ),
        (
            "exotic type byte 5",
            alone(RawCell::exotic(0, filled(5, 33), Vec::new())),
        ),
    ]
}
