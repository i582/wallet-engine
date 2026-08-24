//! Protected-secret references, requests, and host errors.

/// An opaque reference to recovery words in host protected storage.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProtectedSecretRef {
    /// The host storage key. The value does not contain secret bytes.
    pub value: String,
}

/// Explains why the engine requests access to a protected secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum SecretAccessReason {
    /// Reserved for wallet creation storage policy.
    CreateWallet,
    /// The engine needs the mnemonic to sign a transfer.
    SignTransfer,
    /// The engine needs the mnemonic for an off-chain TON Connect ownership proof.
    SignTonConnectProof,
    /// The engine needs the mnemonic to encrypt a transfer comment.
    EncryptComment,
    /// The engine needs the mnemonic to decrypt a transfer comment.
    DecryptComment,
    /// The user requested the recovery phrase.
    RevealRecoveryPhrase,
}

/// A request to read and authorize access to protected secret bytes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProtectedSecretRead {
    /// The host storage key.
    pub secret_ref: ProtectedSecretRef,
    /// The operation that needs secret access.
    pub reason: SecretAccessReason,
    /// User-facing authentication text supplied by Rust.
    pub prompt: String,
}

/// A request to store secret bytes in platform protected storage.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ProtectedSecretStore {
    /// The host storage key.
    pub secret_ref: ProtectedSecretRef,
    /// The mnemonic bytes. The host must not log or persist them outside protected storage.
    pub bytes: Vec<u8>,
    /// Whether later reads must require user presence or device authentication.
    pub require_user_presence: bool,
}

/// Classifies a protected-storage failure reported by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ProtectedSecretHostErrorKind {
    /// No secret exists for the reference.
    NotFound,
    /// User or device authentication failed.
    AuthenticationFailed,
    /// The user or host cancelled the operation.
    Cancelled,
    /// Protected storage is temporarily unavailable.
    Unavailable,
    /// The request violates a platform security policy.
    PolicyViolation,
    /// The failure does not match another kind.
    Other,
}

/// A protected-storage failure returned by [`crate::WalletPlatformHost`].
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    thiserror::Error,
    serde::Serialize,
    serde::Deserialize,
    uniffi::Error,
)]
#[serde(rename_all = "camelCase")]
pub enum ProtectedSecretHostError {
    /// Reports a classified host failure with a safe diagnostic message.
    #[error("protected-secret host failure ({kind:?}): {diagnostic}")]
    Failed {
        /// The stable failure classification.
        kind: ProtectedSecretHostErrorKind,
        /// A developer-facing message that contains no secret bytes.
        diagnostic: String,
    },
}
