//! Requests for TON encrypted transfer comments.

use crate::{Boc, TonAddressString};

/// Requests a ready-to-send TON encrypted-comment body.
///
/// The engine loads the recipient's public key from chain state and asks the
/// platform host to authorize access to this wallet's protected mnemonic.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CreateEncryptedCommentRequest {
    /// Wallet contract that must be able to expose `get_public_key`.
    pub recipient: TonAddressString,
    /// UTF-8 comment to encrypt. Its encoded form must not exceed 960 bytes.
    pub comment: String,
}

/// Requests explicit decryption of one encrypted-comment message body.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DecryptCommentRequest {
    /// Address that sent the encrypted comment.
    ///
    /// TON binds this bounceable, URL-safe, non-test-only address to the
    /// authentication tag. For an incoming activity item this is its
    /// `counterparty`.
    pub sender: TonAddressString,
    /// Complete message-body cell encoded as a Base64 BOC.
    pub body: Boc,
}
