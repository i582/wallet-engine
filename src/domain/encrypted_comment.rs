//! Requests for TON encrypted transfer comments.

use crate::{Boc, TonAddressString};

/// Requests a ready-to-send TON encrypted-comment body.
///
/// The engine uses the supplied recipient public key or loads it from chain
/// state, then asks the platform host to authorize access to this wallet's
/// protected mnemonic.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CreateEncryptedCommentRequest {
    /// Recipient wallet address. Must expose `get_public_key` when no key is supplied.
    pub recipient: TonAddressString,
    /// UTF-8 comment to encrypt. Its encoded form must not exceed 960 bytes.
    pub comment: String,
    /// Optional 32-byte Ed25519 public key used instead of an on-chain lookup.
    ///
    /// The engine verifies this key against `recipient` by deriving supported
    /// wallet addresses with default parameters. A mismatch or unsupported
    /// wallet configuration is rejected before authorizing the sender's secret.
    #[serde(default)]
    #[uniffi(default = None)]
    pub recipient_public_key: Option<Vec<u8>>,
}

/// Asks which public key a TON encrypted comment for a recipient would use.
///
/// The fields mean the same as in [`CreateEncryptedCommentRequest`], so the
/// answer predicts whether that request can encrypt, without a comment and
/// without any protected-secret access.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedCommentRecipientRequest {
    /// Recipient wallet address.
    pub recipient: TonAddressString,
    /// Optional 32-byte Ed25519 public key used instead of an on-chain lookup.
    ///
    /// It is verified against `recipient` exactly as
    /// [`CreateEncryptedCommentRequest::recipient_public_key`] is.
    #[serde(default)]
    #[uniffi(default = None)]
    pub recipient_public_key: Option<Vec<u8>>,
}

/// Requests explicit decryption of one encrypted-comment message body.
///
/// [`crate::WalletClient::decrypt_comment`] reads the protected mnemonic once
/// and tries the current signing key, then the anchor key, without any HTTP
/// request. When neither matches and the wallet has rotated its key, it reads
/// the wallet's `change_wallet_key` history from Toncenter v3, recovers each
/// earlier signing key from the encrypted old key its rotation published, and
/// tries those keys. Recovered keys never leave the call.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DecryptCommentRequest {
    /// Address that sent the encrypted comment.
    ///
    /// TON binds this bounceable, URL-safe, non-test-only address to the
    /// authentication tag. For an incoming activity item this is its
    /// `counterparty`; for an outgoing item it is this wallet's address.
    pub sender: TonAddressString,
    /// Complete message-body cell encoded as a Base64 BOC.
    pub body: Boc,
}
