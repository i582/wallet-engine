//! Typed NFT transfer requests built on top of the shared wallet send pipeline.

use crate::{Boc, NonEmptyString, SendExpiration, TonAddressString, UnsignedDecimalString};

/// The TON amounts used to execute one NFT item transfer.
///
/// Both values are deliberately explicit. The engine does not choose a gas
/// deposit or recipient notification amount on behalf of the application.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NftTransferFunding {
    /// Attach an exact amount to the NFT item call and forward an exact amount
    /// with the ownership notification.
    Exact {
        /// Value attached to the internal message sent to the NFT item contract.
        #[serde(rename = "attachedNanograms", alias = "attached_nanograms")]
        attached_nanograms: UnsignedDecimalString,
        /// Value forwarded to the new owner in `ownership_assigned`.
        #[serde(rename = "forwardNanograms", alias = "forward_nanograms")]
        forward_nanograms: UnsignedDecimalString,
    },
}

/// Optional payload delivered to the new owner with `ownership_assigned`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum NftTransferPayload {
    /// Use an empty forward payload.
    Empty,
    /// Encode a zero opcode followed by UTF-8 TON snake data.
    Comment {
        /// Plaintext comment delivered to the new owner.
        text: String,
    },
    /// Preserve one caller-built cell as the forward payload.
    RawPayload {
        /// Complete payload cell encoded as a validated BOC.
        boc: Boc,
    },
}

/// Immutable choices for a TEP-62 NFT item transfer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NftTransferIntent {
    /// NFT item contract that currently owns the transferable item state.
    pub nft_address: TonAddressString,
    /// Account that becomes the new NFT owner.
    pub recipient: TonAddressString,
    /// Explicit attached and forwarded TON values.
    pub funding: NftTransferFunding,
    /// Payload forwarded to the new owner.
    pub payload: NftTransferPayload,
    /// Wallet message expiration policy.
    pub expiration: SendExpiration,
}

/// Requests fresh validation and emulation of an NFT transfer.
///
/// Reuse the same operation ID in [`NftTransferRequest`] after confirmation so
/// both operations use the same deterministic TEP-62 `query_id`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NftTransferPreviewRequest {
    /// Application-generated correlation identifier.
    pub operation_id: NonEmptyString,
    /// Immutable NFT transfer choices.
    pub intent: NftTransferIntent,
}

/// Requests one owner-signed TEP-62 NFT transfer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NftTransferRequest {
    /// Unique idempotency identifier chosen by the application.
    pub operation_id: NonEmptyString,
    /// Allows replacement of an unresolved durable send after explicit confirmation.
    #[serde(default)]
    pub force: bool,
    /// Immutable NFT transfer choices.
    pub intent: NftTransferIntent,
}

#[cfg(test)]
mod tests {
    use super::*;

    const NFT: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";
    const RECIPIENT: &str = "0:2222222222222222222222222222222222222222222222222222222222222222";

    #[test]
    fn json_requires_both_exact_funding_values() {
        let request = request();
        let value = serde_json::to_value(&request).expect("NFT transfer request serializes");
        assert_eq!(value["intent"]["funding"]["kind"], "exact");
        assert_eq!(value["intent"]["funding"]["attachedNanograms"], "50000000");
        assert_eq!(value["intent"]["funding"]["forwardNanograms"], "10000000");

        let mut missing = value;
        missing["intent"]["funding"]
            .as_object_mut()
            .expect("funding is an object")
            .remove("forwardNanograms");
        assert!(serde_json::from_value::<NftTransferRequest>(missing).is_err());
    }

    #[test]
    fn request_defaults_force_to_false() {
        let mut value = serde_json::to_value(request()).expect("request serializes");
        value
            .as_object_mut()
            .expect("request is an object")
            .remove("force");

        let decoded =
            serde_json::from_value::<NftTransferRequest>(value).expect("request deserializes");
        assert!(!decoded.force);
    }

    fn request() -> NftTransferRequest {
        NftTransferRequest {
            operation_id: NonEmptyString::try_from("nft-operation").expect("operation ID"),
            force: false,
            intent: NftTransferIntent {
                nft_address: TonAddressString::try_from(NFT).expect("NFT address"),
                recipient: TonAddressString::try_from(RECIPIENT).expect("recipient"),
                funding: NftTransferFunding::Exact {
                    attached_nanograms: UnsignedDecimalString::try_from("50000000")
                        .expect("attached value"),
                    forward_nanograms: UnsignedDecimalString::try_from("10000000")
                        .expect("forward value"),
                },
                payload: NftTransferPayload::Empty,
                expiration: SendExpiration::EngineDefault,
            },
        }
    }
}
