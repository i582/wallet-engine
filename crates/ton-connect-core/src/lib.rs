//! Runtime-neutral building blocks for a TON Connect wallet implementation.
//!
//! This crate models the protocol wire format and the NaCl-compatible HTTP
//! bridge session encryption. It deliberately contains no HTTP client,
//! executor, wallet implementation, or persistence policy.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod bridge;
mod connect;
mod deep_link;
mod embedded;
mod http_bridge;
mod manifest;
mod rpc;
mod session;
mod session_state;
mod signing;
mod value;
mod wallet_state;

pub use bridge::BridgeMessage;
pub use connect::{
    ConnectEvent, ConnectEventError, ConnectEventErrorCode, ConnectEventPayload, ConnectItem,
    ConnectItemError, ConnectItemErrorCode, ConnectItemReply, ConnectRequest, DeviceInfo,
    DevicePlatform, Feature, SendTransactionFeature, SignDataFeature, SignDataType,
    SignMessageFeature, StructuredItemType, TonAddressItem, TonAddressItemReply, TonProof,
    TonProofDomain, TonProofItem, TonProofItemReply,
};
pub use deep_link::{ConnectLink, ConnectLinkError, ReturnStrategy};
pub use embedded::{
    EmbeddedRequest, EmbeddedRequestError, decode_embedded_request_param,
    encode_embedded_request_param,
};
pub use http_bridge::{BridgeSseDecoder, BridgeSseMessage, HttpBridgeError, HttpBridgeUrl};
pub use manifest::{AppManifest, ManifestError};
pub use rpc::{
    AppRequest, DisconnectRequest, ExtraCurrencies, KnownAppRequest, KnownWalletResponse,
    RawMessage, RawTransactionPayload, ResponseValidationError, RpcError, RpcErrorCode,
    SendTransactionRequest, SignDataPayload, SignDataRequest, SignDataResult, SignMessageRequest,
    SignMessageResult, StructuredItem, StructuredTransactionPayload, TransactionPayload,
    WalletResponse, WalletResponseError, WalletResponseSuccess, WalletResult,
};
pub use session::{
    PersistedSessionKeyPair, SessionCrypto, SessionCryptoError, SessionKeyPairError,
};
pub use session_state::{
    PreparedAppRequest, PreparedWalletEvent, RpcRequestId, SessionStateError, WalletEventKind,
    WalletSessionPhase, WalletSessionState,
};
pub use signing::{
    Ed25519PublicKey, Ed25519Signature, RawAccountAddress, SignDataSigningPayload, SignatureDomain,
    SigningError, sign_data_signing_hash, ton_proof_message, ton_proof_signing_hash,
    verify_signature,
};
pub use value::{
    Base64Value, ClientId, DecimalString, HttpsUrl, NetworkId, NonEmptyVec, TraceId, Uint64String,
    ValueError,
};
pub use wallet_state::{
    AccountVerificationError, StandardWalletState, StandardWalletVersion, WalletStateError,
    WalletStateInit,
};

/// TON Connect transport protocol version implemented by this crate.
pub const PROTOCOL_VERSION: u8 = 2;

/// Normative specification revision used for this implementation.
///
/// Pinning the revision makes later protocol changes explicit instead of
/// silently changing validation behavior under existing wallet binaries.
pub const SPEC_REVISION: &str = "5656a962eee30819a31a9e918e3de0b9614713b6";
