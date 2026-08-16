//! V5R1 transfer construction and signing.
//!
//! The wallet client passes mnemonic bytes to this private module only after
//! host authorization. The module returns a signed BOC and its normalized
//! external-message hash in standard padded Base64.

use std::str::Utf8Error;

use ton::block_tlb::{
    CommonMsgInfo, CommonMsgInfoExtIn, CommonMsgInfoInt, Msg, SEND_MODE_CARRY_ALL_BALANCE,
    SEND_MODE_IGNORE_ERRORS, SEND_MODE_PAY_FEES_SEPARATELY,
};
use ton::errors::TonError;
use ton::tep::snake_data::SnakeData;
use ton::ton_core::cell::TonCell;
use ton::ton_core::errors::TonCoreError;
use ton::ton_core::traits::tlb::TLB;
use ton::ton_core::types::TonAddress;
use ton::ton_core::types::tlb_core::{MsgAddressExt, TLBCoins, TLBEitherRef};
use ton::ton_wallet::{WALLET_V5R1_ID_DEFAULT, WALLET_V5R1_ID_DEFAULT_TESTNET, WalletVersion};

use crate::types::{Boc, BocError};
use crate::{
    Base64Hash, Base64HashError, Network, NonEmptyString, SendAmount, SendPreviewRequest,
    SendRequest, TonAddressString,
};

use super::crypto::{WalletCryptoError, derive_v5r1_public_state, derive_v5r1_wallet};
use super::send::{FreshSendAccount, PreparedTransfer};

const EXACT_AMOUNT_SEND_MODE: u8 = SEND_MODE_PAY_FEES_SEPARATELY | SEND_MODE_IGNORE_ERRORS;
const ALL_BALANCE_SEND_MODE: u8 = SEND_MODE_CARRY_ALL_BALANCE | SEND_MODE_IGNORE_ERRORS;

#[derive(Debug, thiserror::Error)]
pub(crate) enum TransferError {
    #[error("mnemonic is not valid UTF-8")]
    MnemonicEncoding(#[source] Utf8Error),
    #[error("wallet derivation failed")]
    WalletDerivation(#[source] WalletCryptoError),
    #[error("transfer amount exceeds the TON coin representation")]
    AmountOutOfRange,
    #[error("transfer expiration timestamp exceeds the wallet uint32 field")]
    ExpirationOutOfRange,
    #[error("wallet public key does not derive the configured source address")]
    PublicKeyMismatch,
    #[error("internal message construction failed")]
    InternalMessage(#[source] TonCoreError),
    #[error("external message signing failed")]
    ExternalMessage(#[source] TonError),
    #[error("signed message normalization failed")]
    MessageNormalization(#[source] TonCoreError),
    #[error("normalized message hash calculation failed")]
    MessageHash(#[source] TonCoreError),
    #[error("normalized message hash has an invalid size")]
    InvalidMessageHash(#[source] Base64HashError),
    #[error("signed BOC encoding failed")]
    BocEncoding(#[source] TonCoreError),
    #[error("signed BOC validation failed")]
    InvalidBoc(#[source] BocError),
}

pub(crate) fn prepare_transfer(
    mnemonic_bytes: &[u8],
    record_id: &NonEmptyString,
    source: &TonAddressString,
    network: Network,
    request: &SendRequest,
    account: &FreshSendAccount,
    valid_until: u64,
) -> Result<PreparedTransfer, TransferError> {
    let mnemonic = std::str::from_utf8(mnemonic_bytes).map_err(TransferError::MnemonicEncoding)?;
    let wallet = derive_v5r1_wallet(mnemonic, network).map_err(TransferError::WalletDerivation)?;
    let destination = request.destination.clone();

    let (internal, send_mode) = build_internal_message(
        destination.as_address(),
        &request.amount,
        request.comment.as_deref(),
    )?;

    // Provider and journal timestamps remain u64. Narrow only at the protocol
    // boundary because wallet V5 serializes valid_until as uint32.
    let wallet_valid_until =
        u32::try_from(valid_until).map_err(|_| TransferError::ExpirationOutOfRange)?;

    let external = wallet
        .create_ext_in_msg_with_modes(
            vec![internal],
            vec![send_mode],
            account.seqno,
            wallet_valid_until,
            account.needs_state_init(),
        )
        .map_err(TransferError::ExternalMessage)?;

    let normalized =
        Msg::<TonCell>::from_cell(&external).map_err(TransferError::MessageNormalization)?;
    let message_hash_bytes = normalized
        .cell_hash_normalized()
        .map_err(TransferError::MessageHash)?;
    let message_hash = Base64Hash::from_bytes(message_hash_bytes.as_slice())
        .map_err(TransferError::InvalidMessageHash)?;
    let signed_boc = Boc::try_from(external.to_boc().map_err(TransferError::BocEncoding)?)
        .map_err(TransferError::InvalidBoc)?;

    Ok(PreparedTransfer {
        operation_id: request.operation_id.clone(),
        record_id: record_id.clone(),
        source: source.clone(),
        destination,
        amount: request.amount.clone(),
        comment: request.comment.clone(),
        seqno: account.seqno,
        needs_state_init: account.needs_state_init(),
        valid_until,
        signed_boc,
        message_hash,
    })
}

/// Builds a complete V5R1 transfer with a placeholder signature.
///
/// Toncenter validates the message body and actions with `ignore_chksig=true`.
/// An uninitialized wallet also receives its deterministic `StateInit`, derived
/// from the persisted public key. The mnemonic is never needed for this step.
pub(crate) fn prepare_transfer_emulation(
    source: &TonAddressString,
    public_key: &[u8],
    network: Network,
    request: &SendPreviewRequest,
    account: &FreshSendAccount,
    valid_until: u64,
) -> Result<Boc, TransferError> {
    let destination = request.destination.as_address();
    let (internal, send_mode) =
        build_internal_message(destination, &request.amount, request.comment.as_deref())?;
    let wallet_id = match network {
        Network::Mainnet => WALLET_V5R1_ID_DEFAULT,
        Network::Testnet => WALLET_V5R1_ID_DEFAULT_TESTNET,
    };

    // Keep the preview path identical to real signing: only wallet message
    // serialization narrows the provider-derived timestamp to uint32.
    let wallet_valid_until =
        u32::try_from(valid_until).map_err(|_| TransferError::ExpirationOutOfRange)?;

    let body = WalletVersion::build_ext_in_body_with_modes(
        WalletVersion::V5R1,
        wallet_valid_until,
        account.seqno,
        wallet_id,
        vec![internal],
        vec![send_mode],
    )
    .map_err(TransferError::ExternalMessage)?;

    // V5 stores the signature after the body. Zero bytes are deliberate: the
    // Emulate API skips only this cryptographic check and executes everything else.
    let mut signed = TonCell::builder();
    signed
        .write_cell(&body)
        .map_err(TransferError::InternalMessage)?;
    signed
        .write_bits([0_u8; 64], 512)
        .map_err(TransferError::InternalMessage)?;
    let signed = signed.build().map_err(TransferError::InternalMessage)?;
    let info = CommonMsgInfo::ExtIn(CommonMsgInfoExtIn {
        src: MsgAddressExt::NONE,
        dst: source.as_address().to_msg_address_int(),
        import_fee: TLBCoins::ZERO,
    });

    let mut external = Msg::new(info, signed);
    if account.needs_state_init() {
        let (derived_source, state_init) = derive_v5r1_public_state(public_key, network)
            .map_err(TransferError::WalletDerivation)?;
        if &derived_source != source.as_address() {
            return Err(TransferError::PublicKeyMismatch);
        }
        external.init = Some(TLBEitherRef::new(state_init));
    }
    let external = external.to_cell().map_err(TransferError::InternalMessage)?;

    Boc::try_from(external.to_boc().map_err(TransferError::BocEncoding)?)
        .map_err(TransferError::InvalidBoc)
}

fn build_internal_message(
    destination: &TonAddress,
    amount: &SendAmount,
    comment: Option<&str>,
) -> Result<(TonCell, u8), TransferError> {
    let (amount_nanograms, send_mode) = match amount {
        SendAmount::Exact { nanograms } => {
            let amount = nanograms
                .try_to::<u128>()
                .map_err(|_| TransferError::AmountOutOfRange)?;
            (amount, EXACT_AMOUNT_SEND_MODE)
        }
        SendAmount::All => (0, ALL_BALANCE_SEND_MODE),
    };
    let mut info = CommonMsgInfoInt::new(
        destination.to_msg_address(),
        TLBCoins::new(amount_nanograms),
    );

    // Use one conservative policy until destination metadata is preserved by
    // the address type. This also lets uninitialized recipients accept funds.
    info.bounce = false;

    let body = build_comment_body(comment)?;
    let message = Msg::new(info, body)
        .to_cell()
        .map_err(TransferError::InternalMessage)?;

    Ok((message, send_mode))
}

fn build_comment_body(comment: Option<&str>) -> Result<TonCell, TransferError> {
    let Some(comment) = comment else {
        return Ok(TonCell::empty().to_owned());
    };
    let mut body = TonCell::builder();
    body.write_bits([0_u8; 4], 32)
        .map_err(TransferError::InternalMessage)?;
    SnakeData::from(comment)
        .write(&mut body)
        .map_err(TransferError::InternalMessage)?;
    body.build().map_err(TransferError::InternalMessage)
}

pub(crate) fn derive_source(
    mnemonic_bytes: &[u8],
    network: Network,
) -> Result<TonAddress, TransferError> {
    let mnemonic = std::str::from_utf8(mnemonic_bytes).map_err(TransferError::MnemonicEncoding)?;
    let wallet = derive_v5r1_wallet(mnemonic, network).map_err(TransferError::WalletDerivation)?;
    Ok(wallet.address.clone())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use ton::block_tlb::CommonMsgInfo;

    use super::*;

    const DESTINATION: &str = "0:2222222222222222222222222222222222222222222222222222222222222222";

    #[test]
    fn serialized_internal_message_is_non_bounceable() {
        let destination = TonAddress::from_str(DESTINATION).expect("valid destination");
        let amount = SendAmount::exact("1").expect("valid exact amount");
        let (cell, mode) =
            build_internal_message(&destination, &amount, None).expect("internal message");
        let message = Msg::<TonCell>::from_cell(&cell).expect("decode internal message");

        let CommonMsgInfo::Int(info) = message.info else {
            panic!("transfer must contain an internal message");
        };

        assert!(!info.bounce, "the serialized BOC must disable bouncing");
        assert!(!info.bounced, "a new transfer cannot already be bounced");
        assert_eq!(info.dst, destination.to_msg_address());
        assert_eq!(info.value.coins, TLBCoins::new(1));
        assert_eq!(mode, EXACT_AMOUNT_SEND_MODE);
    }

    #[test]
    fn exact_zero_builds_a_zero_value_internal_message() {
        let destination = TonAddress::from_str(DESTINATION).expect("valid destination");
        let amount = SendAmount::exact("0").expect("zero is canonical");
        let (cell, mode) =
            build_internal_message(&destination, &amount, None).expect("internal message");
        let message = Msg::<TonCell>::from_cell(&cell).expect("decode internal message");

        let CommonMsgInfo::Int(info) = message.info else {
            panic!("transfer must contain an internal message");
        };

        assert_eq!(info.value.coins, TLBCoins::ZERO);
        assert_eq!(mode, EXACT_AMOUNT_SEND_MODE);
    }

    #[test]
    fn all_balance_transfer_uses_mode_130_and_zero_placeholder_value() {
        let destination = TonAddress::from_str(DESTINATION).expect("valid destination");
        let (cell, mode) =
            build_internal_message(&destination, &SendAmount::All, None).expect("internal message");
        let message = Msg::<TonCell>::from_cell(&cell).expect("decode internal message");

        let CommonMsgInfo::Int(info) = message.info else {
            panic!("transfer must contain an internal message");
        };

        assert_eq!(mode, ALL_BALANCE_SEND_MODE);
        assert_eq!(info.value.coins, TLBCoins::ZERO);
    }

    #[test]
    fn plaintext_comment_is_utf8_snake_data_after_the_zero_opcode() {
        let destination = TonAddress::from_str(DESTINATION).expect("valid destination");
        let comment = "Привет, TON! ".repeat(20);
        let amount = SendAmount::exact("1").expect("valid exact amount");
        let (cell, _) = build_internal_message(&destination, &amount, Some(&comment))
            .expect("commented internal message");
        let message = Msg::<TonCell>::from_cell(&cell).expect("decode internal message");
        let mut parser = message.body.parser();

        assert_eq!(parser.read_num::<u32>(32).expect("comment opcode"), 0);
        let snake = SnakeData::read(&mut parser).expect("comment snake data");
        assert_eq!(snake.as_slice(), comment.as_bytes());
        assert!(
            !message.body.refs().is_empty(),
            "a long comment must continue in a child cell"
        );
    }

    #[test]
    fn empty_comment_is_distinct_from_no_comment() {
        let destination = TonAddress::from_str(DESTINATION).expect("valid destination");
        let amount = SendAmount::exact("1").expect("valid exact amount");
        let (without_comment, _) = build_internal_message(&destination, &amount, None)
            .expect("internal message without comment");
        let (with_empty_comment, _) = build_internal_message(&destination, &amount, Some(""))
            .expect("internal message with empty comment");
        let without_comment =
            Msg::<TonCell>::from_cell(&without_comment).expect("decode internal message");
        let with_empty_comment =
            Msg::<TonCell>::from_cell(&with_empty_comment).expect("decode internal message");

        assert_eq!(without_comment.body.data_len_bits(), 0);
        assert_eq!(with_empty_comment.body.data_len_bits(), 32);
    }
}
