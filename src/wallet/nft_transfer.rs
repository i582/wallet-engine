//! Canonical TEP-62 NFT transfer construction.

use ton::tep::nft::NFTTransferMsg;
use ton::ton_core::cell::TonCell;
use ton::ton_core::errors::TonCoreError;
use ton::ton_core::traits::tlb::TLB as _;
use ton::ton_core::types::tlb_core::{TLBCoins, TLBEitherRef};

use crate::{
    Boc, BocError, NftTransferFunding, NftTransferIntent, NftTransferPayload, NonEmptyString,
    SendAmount, SendIntent, SendMessage, SendMessageBody, TonAddressString,
};

use super::transfer::{TransferError, build_comment_body};

/// The canonical generic send intent for one TEP-62 ownership transfer.
#[derive(Debug)]
pub(crate) struct CanonicalNftTransfer {
    pub(crate) intent: SendIntent,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum NftTransferBuildError {
    #[error("NFT attached amount exceeds the TON coin representation")]
    AttachedAmountOutOfRange,
    #[error("NFT forward amount exceeds the TON coin representation")]
    ForwardAmountOutOfRange,
    #[error("NFT attached amount must be greater than the forward amount")]
    AttachedAmountTooSmall,
    #[error("NFT forward payload construction failed")]
    ForwardPayload(#[source] TransferError),
    #[error("NFT raw forward payload is invalid")]
    InvalidRawPayload(#[source] TonCoreError),
    #[error("NFT transfer body serialization failed")]
    BodySerialization(#[source] TonCoreError),
    #[error("NFT transfer body BOC is invalid")]
    InvalidBodyBoc(#[source] BocError),
}

pub(crate) fn canonicalize_nft_transfer(
    operation_id: &NonEmptyString,
    source: &TonAddressString,
    nft: &NftTransferIntent,
) -> Result<CanonicalNftTransfer, NftTransferBuildError> {
    let NftTransferFunding::Exact {
        attached_nanograms,
        forward_nanograms,
    } = &nft.funding;
    let attached = attached_nanograms
        .try_to::<u128>()
        .map_err(|_| NftTransferBuildError::AttachedAmountOutOfRange)?;
    let forward = forward_nanograms
        .try_to::<u128>()
        .map_err(|_| NftTransferBuildError::ForwardAmountOutOfRange)?;
    if attached <= forward {
        return Err(NftTransferBuildError::AttachedAmountTooSmall);
    }

    let forward_payload = match &nft.payload {
        NftTransferPayload::Empty => TonCell::empty().to_owned(),
        NftTransferPayload::Comment { text } => {
            build_comment_body(text).map_err(NftTransferBuildError::ForwardPayload)?
        }
        NftTransferPayload::RawPayload { boc } => TonCell::from_boc(boc.as_bytes().to_vec())
            .map_err(NftTransferBuildError::InvalidRawPayload)?,
    };
    let query_id = operation_query_id(operation_id);
    let transfer = NFTTransferMsg {
        query_id,
        new_owner: nft.recipient.as_address().clone(),
        response_dst: source.as_address().clone(),
        custom_payload: None,
        forward_ton_amount: TLBCoins::new(forward),
        forward_payload: TLBEitherRef::new(forward_payload),
    };
    let body = Boc::try_from(
        transfer
            .to_boc()
            .map_err(NftTransferBuildError::BodySerialization)?,
    )
    .map_err(NftTransferBuildError::InvalidBodyBoc)?;

    Ok(CanonicalNftTransfer {
        intent: SendIntent {
            expiration: nft.expiration.clone(),
            messages: vec![SendMessage {
                destination: nft.nft_address.clone(),
                amount: SendAmount::Exact {
                    nanograms: attached_nanograms.clone(),
                },
                body: SendMessageBody::RawPayload { boc: body },
                bounce: true,
                state_init: None,
            }],
        },
    })
}

/// Derives a stable non-secret query ID from the application's idempotency key.
fn operation_query_id(operation_id: &NonEmptyString) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001b3;

    operation_id
        .as_str()
        .bytes()
        .fold(FNV_OFFSET, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NftTransferFunding, NftTransferPayload, SendExpiration, UnsignedDecimalString};

    const SOURCE: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";
    const NFT: &str = "0:2222222222222222222222222222222222222222222222222222222222222222";
    const RECIPIENT: &str = "0:3333333333333333333333333333333333333333333333333333333333333333";

    #[test]
    fn builds_canonical_bounceable_tep_62_message() {
        let operation_id = NonEmptyString::try_from("transfer-42").expect("operation ID");
        let source = TonAddressString::try_from(SOURCE).expect("source");
        let intent = intent("50000000", "10000000", NftTransferPayload::Empty);

        let canonical = canonicalize_nft_transfer(&operation_id, &source, &intent)
            .expect("NFT transfer builds");
        let message = canonical.intent.messages.first().expect("one message");
        assert!(message.bounce);
        assert_eq!(message.destination, intent.nft_address);
        assert_eq!(
            message.amount,
            SendAmount::Exact {
                nanograms: UnsignedDecimalString::try_from("50000000").expect("amount")
            }
        );

        let SendMessageBody::RawPayload { boc } = &message.body else {
            panic!("NFT transfer must use a raw TEP-62 body");
        };
        let decoded = NFTTransferMsg::from_boc(boc.as_bytes().to_vec()).expect("TEP-62 body");
        assert_eq!(decoded.query_id, operation_query_id(&operation_id));
        assert_eq!(decoded.new_owner, intent.recipient.as_address().clone());
        assert_eq!(decoded.response_dst, source.as_address().clone());
        assert_eq!(decoded.custom_payload, None);
        assert_eq!(decoded.forward_ton_amount, TLBCoins::new(10_000_000));
    }

    #[test]
    fn comment_is_encoded_as_zero_opcode_snake_payload() {
        let operation_id = NonEmptyString::try_from("transfer-comment").expect("operation ID");
        let source = TonAddressString::try_from(SOURCE).expect("source");
        let intent = intent(
            "50000000",
            "10000000",
            NftTransferPayload::Comment {
                text: "hello NFT".to_owned(),
            },
        );
        let canonical = canonicalize_nft_transfer(&operation_id, &source, &intent)
            .expect("NFT transfer builds");
        let SendMessageBody::RawPayload { boc } = &canonical.intent.messages[0].body else {
            panic!("raw body expected");
        };
        let decoded = NFTTransferMsg::from_boc(boc.as_bytes().to_vec()).expect("TEP-62 body");
        let mut parser = decoded.forward_payload.value.parser();
        assert_eq!(parser.read_num::<u32>(32).expect("comment opcode"), 0);
    }

    #[test]
    fn rejects_funding_that_leaves_nothing_for_execution() {
        let operation_id = NonEmptyString::try_from("transfer-small").expect("operation ID");
        let source = TonAddressString::try_from(SOURCE).expect("source");
        let error = canonicalize_nft_transfer(
            &operation_id,
            &source,
            &intent("10000000", "10000000", NftTransferPayload::Empty),
        )
        .expect_err("equal attached and forward amounts must fail");
        assert!(matches!(
            error,
            NftTransferBuildError::AttachedAmountTooSmall
        ));
    }

    fn intent(attached: &str, forward: &str, payload: NftTransferPayload) -> NftTransferIntent {
        NftTransferIntent {
            nft_address: TonAddressString::try_from(NFT).expect("NFT address"),
            recipient: TonAddressString::try_from(RECIPIENT).expect("recipient"),
            funding: NftTransferFunding::Exact {
                attached_nanograms: UnsignedDecimalString::try_from(attached).expect("attached"),
                forward_nanograms: UnsignedDecimalString::try_from(forward).expect("forward"),
            },
            payload,
            expiration: SendExpiration::EngineDefault,
        }
    }
}
