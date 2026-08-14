//! V5R1 transfer construction and signing.
//!
//! The wallet client passes mnemonic bytes to this private module only after
//! host authorization. The module returns a signed BOC and its normalized
//! external-message hash in standard padded Base64.

use std::str::FromStr;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ton::block_tlb::{CommonMsgInfoInt, Msg};
use ton::ton_core::cell::TonCell;
use ton::ton_core::traits::tlb::TLB;
use ton::ton_core::types::TonAddress;
use ton::ton_core::types::tlb_core::TLBCoins;

use crate::send::{FreshSendAccount, PreparedTransfer};
use crate::wallet_crypto::derive_v5r1_wallet;
use crate::{Network, SendRequest};

pub(crate) fn prepare_transfer(
    mnemonic_bytes: &[u8],
    record_id: &str,
    source: &str,
    network: Network,
    request: &SendRequest,
    account: &FreshSendAccount,
    valid_until: u64,
) -> Result<PreparedTransfer, String> {
    let mnemonic = std::str::from_utf8(mnemonic_bytes).map_err(sanitize)?;
    let wallet = derive_v5r1_wallet(mnemonic, network).map_err(sanitize)?;
    let destination = TonAddress::from_str(&request.destination).map_err(sanitize)?;

    // The friendly-address tag determines whether the internal message can bounce.
    let tag = URL_SAFE_NO_PAD
        .decode(&request.destination)
        .map_err(sanitize)?
        .first()
        .copied();

    let mut info = CommonMsgInfoInt::new(
        destination.to_msg_address(),
        TLBCoins::new(request.amount_nanograms.parse::<u128>().map_err(sanitize)?),
    );
    info.bounce = matches!(tag, Some(0x11 | 0x91));

    let internal = Msg::new(info, TonCell::builder().build().map_err(sanitize)?)
        .to_cell()
        .map_err(sanitize)?;

    let valid_until_u32 = u32::try_from(valid_until).map_err(sanitize)?;
    let external = wallet
        .create_ext_in_msg(
            vec![internal],
            account.seqno,
            valid_until_u32,
            account.needs_state_init(),
        )
        .map_err(sanitize)?;

    let normalized = Msg::<TonCell>::from_cell(&external).map_err(sanitize)?;
    let message_hash = STANDARD.encode(normalized.cell_hash_normalized().map_err(sanitize)?);
    let signed_boc = external.to_boc().map_err(sanitize)?;

    Ok(PreparedTransfer {
        operation_id: request.operation_id.clone(),
        record_id: record_id.to_owned(),
        source: source.to_owned(),
        destination: request.destination.clone(),
        amount_nanograms: request.amount_nanograms.clone(),
        seqno: account.seqno,
        needs_state_init: account.needs_state_init(),
        valid_until,
        signed_boc,
        message_hash,
    })
}

pub(crate) fn derive_source(mnemonic_bytes: &[u8], network: Network) -> Result<TonAddress, String> {
    let mnemonic = std::str::from_utf8(mnemonic_bytes).map_err(sanitize)?;
    let wallet = derive_v5r1_wallet(mnemonic, network).map_err(sanitize)?;

    Ok(wallet.address)
}

fn sanitize(error: impl std::fmt::Display) -> String {
    error
        .to_string()
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}
