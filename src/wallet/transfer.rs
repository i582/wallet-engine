//! Wallet transfer construction and signing.
//!
//! The wallet client passes mnemonic bytes to this private module only after
//! host authorization. The module returns a signed BOC and its normalized
//! external-message hash in standard padded Base64.

use std::str::Utf8Error;

use ton::block_tlb::{
    CommonMsgInfo, CommonMsgInfoExtIn, CommonMsgInfoInt, Msg, SEND_MODE_CARRY_ALL_BALANCE,
    SEND_MODE_IGNORE_ERRORS, SEND_MODE_PAY_FEES_SEPARATELY, StateInit,
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
    Base64Hash, Base64HashError, Network, NonEmptyString, SendAmount, SendIntentError, SendMessage,
    SendMessageBody, SendPreviewRequest, SendRequest, TonAddressString,
};

use super::crypto::{WalletCryptoError, derive_wallet, derive_wallet_public_state};
use super::send::{FreshSendAccount, PreparedTransfer, SignedMessageKind};

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
    #[error(transparent)]
    InvalidIntent(#[from] SendIntentError),
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
    let wallet = derive_wallet(mnemonic, network).map_err(TransferError::WalletDerivation)?;
    let _ = request.intent.exact_value_total()?;
    let messages = request.intent.messages.clone();
    let (internal, send_modes) = build_internal_messages(&messages)?;

    // Provider and journal timestamps remain u64. Narrow only at the protocol
    // boundary because wallet V5 serializes valid_until as uint32.
    let wallet_valid_until =
        u32::try_from(valid_until).map_err(|_| TransferError::ExpirationOutOfRange)?;

    let external = wallet
        .create_ext_in_msg_with_modes(
            internal,
            send_modes,
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
        kind: SignedMessageKind::External,
        messages,
        seqno: account.seqno,
        needs_state_init: account.needs_state_init(),
        valid_until,
        signed_boc,
        message_hash,
    })
}

/// Builds a complete owner-signed internal message without submitting it.
///
/// The returned BOC can be handed to a TON Connect dApp or another relayer
/// client. It contains the wallet destination, signed body, and deployment
/// `StateInit` when fresh account state requires it.
pub(crate) fn prepare_internal_signed_transfer(
    mnemonic_bytes: &[u8],
    record_id: &NonEmptyString,
    source: &TonAddressString,
    network: Network,
    request: &SendRequest,
    account: &FreshSendAccount,
    valid_until: u64,
) -> Result<PreparedTransfer, TransferError> {
    let mnemonic = std::str::from_utf8(mnemonic_bytes).map_err(TransferError::MnemonicEncoding)?;
    let wallet = derive_wallet(mnemonic, network).map_err(TransferError::WalletDerivation)?;
    let _ = request.intent.exact_value_total()?;
    let messages = request.intent.messages.clone();
    let (internal, send_modes) = build_internal_messages(&messages)?;
    let wallet_valid_until =
        u32::try_from(valid_until).map_err(|_| TransferError::ExpirationOutOfRange)?;

    let signed = wallet
        .create_internal_signed_msg_with_modes(
            internal,
            send_modes,
            account.seqno,
            wallet_valid_until,
            account.needs_state_init(),
        )
        .map_err(TransferError::ExternalMessage)?;
    let message =
        Msg::<TonCell>::from_cell(&signed).map_err(TransferError::MessageNormalization)?;
    let message_hash = Base64Hash::from_bytes(
        message
            .cell_hash()
            .map_err(TransferError::MessageHash)?
            .as_slice(),
    )
    .map_err(TransferError::InvalidMessageHash)?;
    let signed_boc = Boc::try_from(signed.to_boc().map_err(TransferError::BocEncoding)?)
        .map_err(TransferError::InvalidBoc)?;

    Ok(PreparedTransfer {
        operation_id: request.operation_id.clone(),
        record_id: record_id.clone(),
        source: source.clone(),
        kind: SignedMessageKind::Internal,
        messages,
        seqno: account.seqno,
        needs_state_init: account.needs_state_init(),
        valid_until,
        signed_boc,
        message_hash,
    })
}

/// Builds a complete wallet transfer with a placeholder signature.
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
    let _ = request.intent.exact_value_total()?;
    let (internal, send_modes) = build_internal_messages(&request.intent.messages)?;
    let wallet_id = match network {
        Network::Mainnet => WALLET_V5R1_ID_DEFAULT,
        Network::Testnet => WALLET_V5R1_ID_DEFAULT_TESTNET,
    };

    // Keep the preview path identical to real signing: only wallet message
    // serialization narrows the provider-derived timestamp to uint32.
    let wallet_valid_until =
        u32::try_from(valid_until).map_err(|_| TransferError::ExpirationOutOfRange)?;

    let body = WalletVersion::build_ext_in_body_with_modes(
        WalletVersion::Wallet,
        wallet_valid_until,
        account.seqno,
        wallet_id,
        internal,
        send_modes,
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
        let (derived_source, state_init) = derive_wallet_public_state(public_key, network)
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

/// Serializes one ordered message batch and selects each wallet send mode.
fn build_internal_messages(
    send_messages: &[SendMessage],
) -> Result<(Vec<TonCell>, Vec<u8>), TransferError> {
    let mut messages = Vec::with_capacity(send_messages.len());
    let mut modes = Vec::with_capacity(send_messages.len());
    for message in send_messages {
        let (message, mode) = build_internal_message(message)?;
        messages.push(message);
        modes.push(mode);
    }
    Ok((messages, modes))
}

/// Serializes one complete send message and selects its wallet send mode.
fn build_internal_message(send_message: &SendMessage) -> Result<(TonCell, u8), TransferError> {
    let (amount_nanograms, send_mode) = match &send_message.amount {
        SendAmount::Exact { nanograms } => {
            let amount = nanograms
                .try_to::<u128>()
                .map_err(|_| TransferError::AmountOutOfRange)?;
            (amount, EXACT_AMOUNT_SEND_MODE)
        }
        SendAmount::All => (0, ALL_BALANCE_SEND_MODE),
    };
    let mut info = CommonMsgInfoInt::new(
        send_message.destination.as_address().to_msg_address(),
        TLBCoins::new(amount_nanograms),
    );

    info.bounce = send_message.bounce;

    let body = match &send_message.body {
        SendMessageBody::Empty => TonCell::empty().to_owned(),
        SendMessageBody::Comment { text } => build_comment_body(text)?,
        SendMessageBody::RawPayload { boc } => {
            TonCell::from_boc(boc.as_bytes().to_vec()).map_err(TransferError::InternalMessage)?
        }
    };
    let mut message = Msg::new(info, body);
    if let Some(state_init) = &send_message.state_init {
        let state_init = StateInit::from_boc(state_init.as_bytes().to_vec())
            .map_err(TransferError::InternalMessage)?;
        message.init = Some(TLBEitherRef::new(state_init));
    }
    let message = message.to_cell().map_err(TransferError::InternalMessage)?;

    Ok((message, send_mode))
}

/// Encodes one plaintext comment as zero-opcode TON snake data.
pub(super) fn build_comment_body(comment: &str) -> Result<TonCell, TransferError> {
    let mut body = TonCell::builder();
    body.write_bits([0_u8; 4], 32)
        .map_err(TransferError::InternalMessage)?;
    SnakeData::from(comment)
        .write(&mut body)
        .map_err(TransferError::InternalMessage)?;
    body.build().map_err(TransferError::InternalMessage)
}

/// Derives the configured wallet address from protected mnemonic bytes.
pub(crate) fn derive_source(
    mnemonic_bytes: &[u8],
    network: Network,
) -> Result<TonAddress, TransferError> {
    let mnemonic = std::str::from_utf8(mnemonic_bytes).map_err(TransferError::MnemonicEncoding)?;
    let wallet = derive_wallet(mnemonic, network).map_err(TransferError::WalletDerivation)?;
    Ok(wallet.address.clone())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::{SendExpiration, SendIntent};
    use ton::block_tlb::CommonMsgInfo;
    use ton::ton_core::types::tlb_core::MsgAddress;

    use super::*;

    const DESTINATION: &str = "0:2222222222222222222222222222222222222222222222222222222222222222";

    #[test]
    fn serialized_internal_message_is_non_bounceable() {
        let destination = TonAddress::from_str(DESTINATION).expect("valid destination");
        let amount = SendAmount::exact("1").expect("valid exact amount");
        let message = send_message(amount, SendMessageBody::Empty, None);
        let (cell, mode) = build_internal_message(&message).expect("internal message");
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
    fn contract_call_can_be_serialized_as_bounceable() {
        let mut message = send_message(
            SendAmount::exact("1").expect("valid exact amount"),
            SendMessageBody::Empty,
            None,
        );
        message.bounce = true;
        let (cell, _) = build_internal_message(&message).expect("internal message");
        let message = Msg::<TonCell>::from_cell(&cell).expect("decode internal message");
        let CommonMsgInfo::Int(info) = message.info else {
            panic!("transfer must contain an internal message");
        };

        assert!(info.bounce);
        assert!(!info.bounced);
    }

    #[test]
    fn exact_zero_builds_a_zero_value_internal_message() {
        let amount = SendAmount::exact("0").expect("zero is canonical");
        let message = send_message(amount, SendMessageBody::Empty, None);
        let (cell, mode) = build_internal_message(&message).expect("internal message");
        let message = Msg::<TonCell>::from_cell(&cell).expect("decode internal message");

        let CommonMsgInfo::Int(info) = message.info else {
            panic!("transfer must contain an internal message");
        };

        assert_eq!(info.value.coins, TLBCoins::ZERO);
        assert_eq!(mode, EXACT_AMOUNT_SEND_MODE);
    }

    #[test]
    fn all_balance_transfer_uses_mode_130_and_zero_placeholder_value() {
        let message = send_message(SendAmount::All, SendMessageBody::Empty, None);
        let (cell, mode) = build_internal_message(&message).expect("internal message");
        let message = Msg::<TonCell>::from_cell(&cell).expect("decode internal message");

        let CommonMsgInfo::Int(info) = message.info else {
            panic!("transfer must contain an internal message");
        };

        assert_eq!(mode, ALL_BALANCE_SEND_MODE);
        assert_eq!(info.value.coins, TLBCoins::ZERO);
    }

    #[test]
    fn plaintext_comment_is_utf8_snake_data_after_the_zero_opcode() {
        let comment = "Привет, TON! ".repeat(20);
        let amount = SendAmount::exact("1").expect("valid exact amount");
        let message = send_message(
            amount,
            SendMessageBody::Comment {
                text: comment.clone(),
            },
            None,
        );
        let (cell, _) = build_internal_message(&message).expect("commented internal message");
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
        let amount = SendAmount::exact("1").expect("valid exact amount");
        let without_comment = send_message(amount.clone(), SendMessageBody::Empty, None);
        let with_empty_comment = send_message(
            amount,
            SendMessageBody::Comment {
                text: String::new(),
            },
            None,
        );
        let (without_comment, _) =
            build_internal_message(&without_comment).expect("internal message without comment");
        let (with_empty_comment, _) = build_internal_message(&with_empty_comment)
            .expect("internal message with empty comment");
        let without_comment =
            Msg::<TonCell>::from_cell(&without_comment).expect("decode internal message");
        let with_empty_comment =
            Msg::<TonCell>::from_cell(&with_empty_comment).expect("decode internal message");

        assert_eq!(without_comment.body.data_len_bits(), 0);
        assert_eq!(with_empty_comment.body.data_len_bits(), 32);
    }

    #[test]
    fn caller_built_payload_is_preserved_as_the_internal_message_body() {
        let amount = SendAmount::exact("1").expect("valid exact amount");
        let mut payload = TonCell::builder();
        payload
            .write_num(&0x1234_5678_u32, 32)
            .expect("payload opcode fits");
        let payload = payload.build().expect("payload cell builds");
        let payload_boc = Boc::try_from(payload.to_boc().expect("payload BOC encodes"))
            .expect("payload BOC validates");

        let send_message = send_message(
            amount,
            SendMessageBody::RawPayload { boc: payload_boc },
            None,
        );
        let (cell, _) = build_internal_message(&send_message).expect("payload internal message");
        let message = Msg::<TonCell>::from_cell(&cell).expect("decode internal message");

        assert_eq!(&*message.body, &payload);
    }

    #[test]
    fn caller_built_state_init_is_attached_to_the_internal_message() {
        let amount = SendAmount::exact("1").expect("valid exact amount");
        let state_init = StateInit::new(TonCell::empty().clone(), TonCell::empty().clone());
        let state_init_boc = Boc::try_from(state_init.to_boc().expect("StateInit BOC encodes"))
            .expect("StateInit BOC validates");

        let send_message = send_message(amount, SendMessageBody::Empty, Some(state_init_boc));
        let (cell, _) = build_internal_message(&send_message).expect("deploy internal message");
        let message = Msg::<TonCell>::from_cell(&cell).expect("decode internal message");
        let attached = message.init.expect("StateInit must be attached");

        assert_eq!(&*attached, &state_init);
    }

    #[test]
    fn undeployed_preview_rejects_a_public_key_for_another_source() {
        let (source, _) = derive_wallet_public_state(&[1_u8; 32], Network::Testnet)
            .expect("source public key must derive");
        let source = TonAddressString::from_address(&source, Network::Testnet);
        let request = SendPreviewRequest {
            intent: SendIntent {
                expiration: SendExpiration::EngineDefault,
                messages: vec![send_message(
                    SendAmount::exact("1").expect("valid exact amount"),
                    SendMessageBody::Empty,
                    None,
                )],
            },
        };
        let account = FreshSendAccount {
            status: crate::AccountStatus::Uninitialized,
            seqno: 0,
        };

        assert!(matches!(
            prepare_transfer_emulation(
                &source,
                &[2_u8; 32],
                Network::Testnet,
                &request,
                &account,
                1,
            ),
            Err(TransferError::PublicKeyMismatch)
        ));
    }

    #[test]
    fn preview_emulation_preserves_payload_and_state_init() {
        let public_key = [1_u8; 32];
        let (source, _) = derive_wallet_public_state(&public_key, Network::Testnet)
            .expect("source public key must derive");
        let source = TonAddressString::from_address(&source, Network::Testnet);
        let amount = SendAmount::exact("1").expect("valid exact amount");
        let account = FreshSendAccount {
            status: crate::AccountStatus::Active,
            seqno: 7,
        };

        let plain = SendPreviewRequest {
            intent: SendIntent {
                expiration: SendExpiration::EngineDefault,
                messages: vec![send_message(amount, SendMessageBody::Empty, None)],
            },
        };
        let mut payload = TonCell::builder();
        payload
            .write_num(&0x1234_5678_u32, 32)
            .expect("payload opcode fits");
        let payload = Boc::try_from(
            payload
                .build()
                .expect("payload cell builds")
                .to_boc()
                .expect("payload BOC encodes"),
        )
        .expect("payload BOC validates");
        let state_init = StateInit::new(TonCell::empty().clone(), TonCell::empty().clone());
        let state_init = Boc::try_from(state_init.to_boc().expect("StateInit BOC encodes"))
            .expect("StateInit BOC validates");
        let with_payload = SendPreviewRequest {
            intent: SendIntent {
                messages: vec![SendMessage {
                    body: SendMessageBody::RawPayload { boc: payload },
                    ..plain.intent.messages[0].clone()
                }],
                ..plain.intent.clone()
            },
        };
        let with_state_init = SendPreviewRequest {
            intent: SendIntent {
                messages: vec![SendMessage {
                    state_init: Some(state_init),
                    ..plain.intent.messages[0].clone()
                }],
                ..plain.intent.clone()
            },
        };

        let emulate = |request: &SendPreviewRequest| {
            prepare_transfer_emulation(
                &source,
                &public_key,
                Network::Testnet,
                request,
                &account,
                1_900_000_000,
            )
            .expect("preview message builds")
        };
        let plain_boc = emulate(&plain);

        assert_ne!(emulate(&with_payload), plain_boc);
        assert_ne!(emulate(&with_state_init), plain_boc);
    }

    #[test]
    fn internal_signing_preserves_the_ordered_batch_and_deployment_state() {
        const MNEMONIC: &str = "section garden tomato dinner season dice renew length useful spin trade intact use universe what post spike keen mandate behind concert egg doll rug";

        let source = derive_source(MNEMONIC.as_bytes(), Network::Testnet)
            .expect("fixture mnemonic derives a wallet");
        let source = TonAddressString::from_address(&source, Network::Testnet);
        let request = SendRequest {
            operation_id: NonEmptyString::try_from("sign-message-operation")
                .expect("operation id is valid"),
            force: false,
            intent: SendIntent {
                expiration: SendExpiration::Exact {
                    unix_timestamp: 1_900_000_000,
                },
                messages: vec![
                    send_message(
                        SendAmount::exact("1").expect("amount is valid"),
                        SendMessageBody::Empty,
                        None,
                    ),
                    send_message(
                        SendAmount::exact("2").expect("amount is valid"),
                        SendMessageBody::Empty,
                        None,
                    ),
                ],
            },
        };
        let account = FreshSendAccount {
            status: crate::AccountStatus::Uninitialized,
            seqno: 0,
        };

        let prepared = prepare_internal_signed_transfer(
            MNEMONIC.as_bytes(),
            &NonEmptyString::try_from("record").expect("record id is valid"),
            &source,
            Network::Testnet,
            &request,
            &account,
            1_900_000_000,
        )
        .expect("internal signed message builds");
        let outer = Msg::<TonCell>::from_boc(prepared.signed_boc.as_bytes().to_vec())
            .expect("signed BOC decodes as a message");
        let CommonMsgInfo::Int(info) = &outer.info else {
            panic!("signed message must use an internal envelope");
        };
        assert_eq!(info.src, MsgAddress::NONE);
        assert_eq!(info.dst, source.as_address().to_msg_address());
        assert_eq!(info.value.coins, TLBCoins::ZERO);
        assert!(!info.bounce);
        assert!(outer.init.is_some());

        let (body, signature) = ton::ton_wallet::WalletV5InternalSignedBody::read_signed(
            &mut outer.body.value.parser(),
        )
        .expect("signed body decodes");
        assert_eq!(body.valid_until, 1_900_000_000);
        assert_eq!(body.msg_seqno, 0);
        assert_eq!(body.msgs_modes, vec![EXACT_AMOUNT_SEND_MODE; 2]);
        assert_eq!(body.msgs.len(), 2);
        assert_eq!(signature.len(), 64);

        let values = body
            .msgs
            .iter()
            .map(|cell| {
                Msg::<TonCell>::from_cell(cell)
                    .expect("action message decodes")
                    .info
                    .as_int()
                    .expect("action is internal")
                    .value
                    .coins
                    .to_u128()
            })
            .collect::<Vec<_>>();
        assert_eq!(values, vec![1, 2]);
    }

    /// Builds one test message with a stable destination.
    fn send_message(
        amount: SendAmount,
        body: SendMessageBody,
        state_init: Option<Boc>,
    ) -> SendMessage {
        SendMessage {
            destination: TonAddressString::try_from(DESTINATION)
                .expect("valid preview destination"),
            amount,
            body,
            bounce: false,
            state_init,
        }
    }
}
