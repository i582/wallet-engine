//! V5R1 transfer construction and signing.
//!
//! The wallet client passes mnemonic bytes to this private module only after
//! host authorization. The module returns a signed BOC and its normalized
//! external-message hash in standard padded Base64.

use std::str::FromStr;
use std::{num::ParseIntError, str::Utf8Error};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ton::block_tlb::{CommonMsgInfoInt, Msg};
use ton::errors::TonError;
use ton::ton_core::cell::TonCell;
use ton::ton_core::errors::TonCoreError;
use ton::ton_core::traits::tlb::TLB;
use ton::ton_core::types::TonAddress;
use ton::ton_core::types::tlb_core::TLBCoins;

use crate::send::{FreshSendAccount, PreparedTransfer};
use crate::{Network, SendRequest};

use super::crypto::{WalletCryptoError, derive_v5r1_wallet};

#[derive(Debug, thiserror::Error)]
pub(crate) enum TransferError {
    #[error("mnemonic is not valid UTF-8")]
    MnemonicEncoding(#[source] Utf8Error),
    #[error("wallet derivation failed")]
    WalletDerivation(#[source] WalletCryptoError),
    #[error("destination address is invalid")]
    InvalidDestination(#[source] TonCoreError),
    #[error("transfer amount is invalid")]
    InvalidAmount(#[source] ParseIntError),
    #[error("internal message construction failed")]
    InternalMessage(#[source] TonCoreError),
    #[error("external message signing failed")]
    ExternalMessage(#[source] TonError),
    #[error("signed message normalization failed")]
    MessageNormalization(#[source] TonCoreError),
    #[error("normalized message hash calculation failed")]
    MessageHash(#[source] TonCoreError),
    #[error("signed BOC encoding failed")]
    BocEncoding(#[source] TonCoreError),
}

pub(crate) fn prepare_transfer(
    mnemonic_bytes: &[u8],
    record_id: &str,
    source: &str,
    network: Network,
    request: &SendRequest,
    account: &FreshSendAccount,
    valid_until: u32,
) -> Result<PreparedTransfer, TransferError> {
    let mnemonic = std::str::from_utf8(mnemonic_bytes).map_err(TransferError::MnemonicEncoding)?;
    let wallet = derive_v5r1_wallet(mnemonic, network).map_err(TransferError::WalletDerivation)?;
    let destination =
        TonAddress::from_str(&request.destination).map_err(TransferError::InvalidDestination)?;

    let amount_nanograms = request
        .amount_nanograms
        .parse::<u128>()
        .map_err(TransferError::InvalidAmount)?;

    let info = CommonMsgInfoInt::new(
        destination.to_msg_address(),
        TLBCoins::new(amount_nanograms),
    );

    let internal = Msg::new(info, TonCell::empty().to_owned())
        .to_cell()
        .map_err(TransferError::InternalMessage)?;

    let external = wallet
        .create_ext_in_msg(
            vec![internal],
            account.seqno,
            valid_until,
            account.needs_state_init(),
        )
        .map_err(TransferError::ExternalMessage)?;

    let normalized =
        Msg::<TonCell>::from_cell(&external).map_err(TransferError::MessageNormalization)?;
    let message_hash = STANDARD.encode(
        normalized
            .cell_hash_normalized()
            .map_err(TransferError::MessageHash)?,
    );
    let signed_boc = external.to_boc().map_err(TransferError::BocEncoding)?;

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

pub(crate) fn derive_source(
    mnemonic_bytes: &[u8],
    network: Network,
) -> Result<TonAddress, TransferError> {
    let mnemonic = std::str::from_utf8(mnemonic_bytes).map_err(TransferError::MnemonicEncoding)?;
    let wallet = derive_v5r1_wallet(mnemonic, network).map_err(TransferError::WalletDerivation)?;
    Ok(wallet.address)
}
