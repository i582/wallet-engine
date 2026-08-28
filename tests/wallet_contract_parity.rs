use std::error::Error;
use std::str::FromStr;

use ton::ton_core::cell::{TonCell, TonHash};
use ton::ton_core::traits::tlb::TLB as _;
use ton::ton_wallet::{WalletData, WalletExtMsgBody, WalletInternalSignedBody};

fn source_vector_messages() -> Result<Vec<TonCell>, Box<dyn Error>> {
    [0xa0_u8, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7]
        .into_iter()
        .map(|value| {
            let mut builder = TonCell::builder();
            builder.write_num(&value, 8)?;
            Ok(builder.build()?)
        })
        .collect()
}

fn assert_hash(cell: &TonCell, expected: &str) -> Result<(), Box<dyn Error>> {
    assert_eq!(cell.cell_hash()?, TonHash::from_str(expected)?);
    Ok(())
}

/// These hashes are produced independently by serializing the original Wallet
/// rev00 Tolk types with `toCell()`.
#[test]
fn rust_serialization_matches_original_tolk_vectors() -> Result<(), Box<dyn Error>> {
    let storage = WalletData {
        revision: 0,
        seqno: 0x0102_0304,
        wallet_id: 0x7fff_7ffd,
        public_key: TonHash::from([0x11; 32]),
    }
    .to_cell()?;
    assert_hash(
        &storage,
        "879617849c5eb8056d68eeb4fa84d090bfc11123e069740b854f3678f8678d64",
    )?;

    let msgs = source_vector_messages()?;
    let single_external = WalletExtMsgBody {
        wallet_id: 0x7fff_7ffd,
        valid_until: 0x7100_0000,
        msg_seqno: 0x0102_0304,
        msgs_modes: vec![3],
        msgs: vec![msgs[3].clone()],
    }
    .to_cell()?;
    assert_hash(
        &single_external,
        "b97c66d7258200b0afbe52c5cbef29085a21710ee5836ea2569ab59535f4e48d",
    )?;

    let single_internal = WalletInternalSignedBody {
        wallet_id: 0x7fff_7ffd,
        valid_until: 0x7100_0000,
        msg_seqno: 0x0102_0304,
        msgs_modes: vec![3],
        msgs: vec![msgs[3].clone()],
    }
    .to_cell()?;
    assert_hash(
        &single_internal,
        "9fc40d8bfdb14dfff5d3c0e7e33f0fb7a58a9cdf752eb80c7ded67676a10b325",
    )?;

    let modes = (0_u8..8).collect::<Vec<_>>();
    let bulk_external = WalletExtMsgBody {
        wallet_id: 0x7fff_7ffd,
        valid_until: 0x7100_0000,
        msg_seqno: 0x0102_0304,
        msgs_modes: modes.clone(),
        msgs: msgs.clone(),
    }
    .to_cell()?;
    assert_hash(
        &bulk_external,
        "e054c096b5d08ce2e644d7854de8d3aa19bac7e878da8e4c62801501f8ff7c8c",
    )?;

    let bulk_internal = WalletInternalSignedBody {
        wallet_id: 0x7fff_7ffd,
        valid_until: 0x7100_0000,
        msg_seqno: 0x0102_0304,
        msgs_modes: modes.clone(),
        msgs: msgs.clone(),
    }
    .to_cell()?;
    assert_hash(
        &bulk_internal,
        "05b6b7e5ecf5df8a3dd510fd0ccab79e6e36bbb90a1cbd81887a13f10edf7d38",
    )?;

    let mut signed_builder = TonCell::builder();
    signed_builder.write_bits([0; 64], 512)?;
    signed_builder.write_cell(&single_external)?;
    let signed_external = signed_builder.build()?;
    assert_hash(
        &signed_external,
        "034e26183219f8abe14d0ee15baed00b7deaf25052d4eb8266f3b8b53373445c",
    )?;

    let (parsed, signature) = WalletExtMsgBody::read_signed(&mut signed_external.parser())?;
    assert_eq!(
        parsed,
        WalletExtMsgBody {
            wallet_id: 0x7fff_7ffd,
            valid_until: 0x7100_0000,
            msg_seqno: 0x0102_0304,
            msgs_modes: vec![3],
            msgs: vec![source_vector_messages()?[3].clone()],
        }
    );
    assert_eq!(signature, vec![0; 64]);

    let mut signed_internal_builder = TonCell::builder();
    signed_internal_builder.write_bits([0; 64], 512)?;
    signed_internal_builder.write_cell(&single_internal)?;
    let signed_internal = signed_internal_builder.build()?;
    assert_hash(
        &signed_internal,
        "ef9fa6bb5b77b4dcdfa2dead593dd912c1e9a8b71fcfae8f98d6028995c55584",
    )?;
    let (parsed_internal, signature) =
        WalletInternalSignedBody::read_signed(&mut signed_internal.parser())?;
    assert_eq!(
        parsed_internal,
        WalletInternalSignedBody {
            wallet_id: 0x7fff_7ffd,
            valid_until: 0x7100_0000,
            msg_seqno: 0x0102_0304,
            msgs_modes: vec![3],
            msgs: vec![source_vector_messages()?[3].clone()],
        }
    );
    assert_eq!(signature, vec![0; 64]);

    let mut signed_bulk_builder = TonCell::builder();
    signed_bulk_builder.write_bits([0; 64], 512)?;
    signed_bulk_builder.write_cell(&bulk_external)?;
    let signed_bulk = signed_bulk_builder.build()?;
    let (parsed_bulk, signature) = WalletExtMsgBody::read_signed(&mut signed_bulk.parser())?;
    assert_eq!(
        parsed_bulk,
        WalletExtMsgBody {
            wallet_id: 0x7fff_7ffd,
            valid_until: 0x7100_0000,
            msg_seqno: 0x0102_0304,
            msgs_modes: modes,
            msgs,
        }
    );
    assert_eq!(signature, vec![0; 64]);

    let mut messages_255 = Vec::with_capacity(255);
    for index in 0_u8..=254 {
        let mut builder = TonCell::builder();
        builder.write_num(&index, 8)?;
        messages_255.push(builder.build()?);
    }
    let bulk_255 = WalletExtMsgBody {
        wallet_id: 0x7fff_7ffd,
        valid_until: 0x7100_0000,
        msg_seqno: 0x0102_0304,
        msgs_modes: (0_u8..=254).collect(),
        msgs: messages_255,
    }
    .to_cell()?;
    assert_hash(
        &bulk_255,
        "131f1b9ad39ae9fe40b2bfe64618b563aa11458c5c0382c26971a2c006bb7819",
    )?;
    Ok(())
}
