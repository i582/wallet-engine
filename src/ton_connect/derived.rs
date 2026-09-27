//! `MTProto`-relayed TON Connect sessions with server-nonce-derived keys.
//!
//! The server relays opaque `nonce || box` ciphertext between a dApp and every
//! device of the user. Each device derives the same session key pair from the
//! wallet's current signing key, the dApp client id, and a per-session server
//! nonce, so no session material is persisted anywhere. The server issues
//! event identifiers and orders requests; the session object here keeps no
//! replay state and no event counter.

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use ton_connect_core::{
    AccountAddress, AppRequest, Base64Value, CellBoc, ClientId, ConnectEvent, ConnectEventError,
    ConnectEventErrorCode, ConnectEventPayload, ConnectItemReply, DeviceInfo, Ed25519PublicKey,
    Ed25519Signature, EmptyObject, Feature, KnownAppRequest, NetworkId, RawAccountAddress,
    SendTransactionFeature, SessionCrypto, SignDataFeature, SignDataPayload, SignDataResult,
    SignDataType, TonProofItemReply, WalletResponse, WalletResponseError, WalletResponseSuccess,
    WalletResult,
};
use zeroize::Zeroizing;

use super::{
    RequestContext, TON_CONNECT_MAX_MESSAGES, TransactionRequestKind, account_reply,
    decode_transaction_request, device_platform, failed, proof_reply, rpc_error_code,
    session_error, unsupported_request,
};
use crate::{
    TonConnectAccountInfo, TonConnectDevice, TonConnectIncomingRequest, TonConnectProofReply,
    TonConnectRpcErrorCode, TonConnectSessionError, WalletDescriptor, bounded_diagnostic,
};

/// Inputs of one derived `MTProto` TON Connect session.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TonConnectDerivedSessionRequest {
    /// Wallet whose current signing key derives the session key.
    pub descriptor: WalletDescriptor,
    /// The dApp client id `A`: exactly 64 lowercase hex characters.
    pub dapp_client_id: String,
    /// The raw per-session nonce issued by the server. It must not be empty.
    pub nonce: Vec<u8>,
}

/// Connect-event error codes a wallet reports for a failed connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TonConnectConnectErrorCode {
    /// The dApp manifest could not be fetched (protocol code 2).
    ManifestNotFound,
    /// The dApp manifest is malformed (protocol code 3).
    ManifestContent,
    /// The user declined the connection (protocol code 300).
    UserDeclined,
}

/// One authenticated dApp request relayed by the server.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TonConnectDerivedRequest {
    /// The validated request. Only a derived session reports `SignData`; the
    /// classic bridge session reports `signData` as `Unsupported`.
    pub request: TonConnectIncomingRequest,
    /// The request `id` as a signed 64-bit value.
    ///
    /// `None` when `id` is not a canonical decimal that fits a signed 64-bit
    /// value; `request` is still decoded so the host can answer with the exact
    /// id string.
    pub request_id: Option<i64>,
}

/// The data of one `signData` request, exactly as the dApp sent it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum TonConnectSignDataPayload {
    /// UTF-8 text shown to the user and signed verbatim.
    Text {
        /// The text to sign.
        text: String,
    },
    /// Opaque bytes.
    Binary {
        /// The bytes in base64, exactly as the dApp sent them.
        bytes: String,
    },
    /// A TVM cell described by a TL-B schema; the engine does not decode it.
    Cell {
        /// The TL-B schema whose last declared type is the root, exactly as sent.
        schema: String,
        /// The one-root cell `BoC` in base64, exactly as the dApp sent it.
        cell: String,
    },
}

/// A `signData` request already validated against the session's wallet.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TonConnectSignDataRequest {
    /// The data to show and sign.
    pub payload: TonConnectSignDataPayload,
    /// The network global ID the dApp named, exactly as sent; it matches the wallet.
    pub network: Option<String>,
    /// The signer address the dApp named, exactly as sent; it is the wallet's address.
    pub from: Option<String>,
}

/// Requests a TON Connect `signData` signature with the wallet's current signing key.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TonConnectSignDataSignRequest {
    /// Wallet whose protected key must sign the data.
    pub descriptor: WalletDescriptor,
    /// The decoded request to sign.
    pub request: TonConnectSignDataRequest,
    /// Exact dApp manifest domain shown to and approved by the user.
    pub domain: String,
    /// Unix signing time in seconds.
    pub timestamp: u64,
}

/// A signed TON Connect `signData` request, ready for the success response.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TonConnectSignedData {
    /// The signed request.
    pub request: TonConnectSignDataRequest,
    /// The domain bound into the signature.
    pub domain: String,
    /// The Unix signing time in seconds bound into the signature.
    pub timestamp: u64,
    /// Exact 64-byte Ed25519 signature.
    pub signature: Vec<u8>,
    /// The current 32-byte Ed25519 signing public key that made the signature.
    pub public_key: Vec<u8>,
}

/// `ephemeral_pk(32) || nonce(24) || box(48)`: a 32-byte answer sealed to `W`.
const CHALLENGE_LENGTH: usize = 104;

/// Length of the plaintext a challenge box must open to.
const CHALLENGE_ANSWER_LENGTH: usize = 32;

/// Longest `signData` domain accepted: a DNS name is at most 253 bytes, plus a port.
const MAX_SIGN_DATA_DOMAIN_BYTES: usize = 259;

/// One `MTProto`-relayed TON Connect session whose key pair was derived from the wallet key.
///
/// The object keeps no replay state and no event counter: the server issues event
/// identifiers and enforces request ordering. Nothing here is persisted.
#[derive(uniffi::Object)]
pub struct TonConnectDerivedSession {
    crypto: SessionCrypto,
    peer: ClientId,
    signing_public_key: [u8; 32],
    address: RawAccountAddress,
    network: NetworkId,
}

impl TonConnectDerivedSession {
    /// Builds the session from a derived secret, which is wiped before this returns.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "taking the caller's only copy of the secret wipes it as soon as the session key exists"
    )]
    pub(crate) fn new(
        secret: Zeroizing<[u8; 32]>,
        peer: ClientId,
        signing_public_key: [u8; 32],
        address: RawAccountAddress,
        network: NetworkId,
    ) -> Arc<Self> {
        Arc::new(Self {
            crypto: SessionCrypto::from_secret_key(*secret),
            peer,
            signing_public_key,
            address,
            network,
        })
    }

    /// Serializes `payload` and seals it to the dApp as raw `nonce || box` bytes.
    fn encrypt<T: Serialize>(&self, payload: &T) -> Result<Vec<u8>, TonConnectSessionError> {
        let plaintext = serde_json::to_vec(payload).map_err(session_error)?;
        self.crypto
            .encrypt(self.peer, &plaintext)
            .map_err(session_error)
    }

    /// Validates one authenticated request against the session account.
    fn decode(
        &self,
        request: AppRequest,
        now: u64,
    ) -> Result<TonConnectIncomingRequest, TonConnectSessionError> {
        let id = request.id.clone();
        let method = request.method.clone();
        let context = RequestContext {
            network: &self.network,
            address: self.address,
            client_id: self.crypto.client_id(),
        };
        Ok(match request.decode() {
            Ok(KnownAppRequest::SendTransaction(request)) => decode_transaction_request(
                &context,
                &id,
                method,
                request.payload,
                now,
                TransactionRequestKind::Send,
            )?,
            Ok(KnownAppRequest::SignMessage(_)) => unsupported_request(
                id,
                method,
                TonConnectRpcErrorCode::MethodNotSupported,
                "Method is not supported",
            ),
            Ok(KnownAppRequest::SignData(request)) => {
                match request
                    .payload
                    .validate_context(&self.network, &self.address)
                {
                    Err(error) => unsupported_request(
                        id,
                        method,
                        TonConnectRpcErrorCode::BadRequest,
                        &error.to_string(),
                    ),
                    Ok(()) => TonConnectIncomingRequest::SignData {
                        id,
                        method,
                        request: sign_data_request(request.payload),
                    },
                }
            }
            Ok(KnownAppRequest::Disconnect(_)) => {
                TonConnectIncomingRequest::Disconnect { id, method }
            }
            Err(error) => unsupported_request(
                id,
                method,
                TonConnectRpcErrorCode::BadRequest,
                &error.to_string(),
            ),
        })
    }
}

#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "UniFFI exports owned records, byte buffers, and strings at the foreign-language boundary"
)]
impl TonConnectDerivedSession {
    /// Returns the session public key `W` as 64 lowercase hex characters.
    #[must_use]
    pub fn public_key_hex(&self) -> String {
        self.crypto.client_id().to_string()
    }

    /// Returns the 32-byte Ed25519 signing public key the session was derived from.
    #[must_use]
    pub fn signing_public_key(&self) -> Vec<u8> {
        self.signing_public_key.to_vec()
    }

    /// Opens a server challenge and returns its 32-byte answer.
    ///
    /// `challenge` is exactly `ephemeral_pk(32) || nonce(24) || box(48)`: a
    /// 32-byte answer sealed to `W` under an ephemeral key.
    pub fn open_challenge(&self, challenge: Vec<u8>) -> Result<Vec<u8>, TonConnectSessionError> {
        if challenge.len() != CHALLENGE_LENGTH {
            return Err(failed(
                "TON Connect challenge must contain exactly 104 bytes",
            ));
        }
        let ephemeral = challenge
            .get(..32)
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .ok_or_else(|| failed("TON Connect challenge is truncated"))?;
        let sealed = challenge
            .get(32..)
            .ok_or_else(|| failed("TON Connect challenge is truncated"))?;
        let answer = self
            .crypto
            .decrypt(ClientId::from_bytes(ephemeral), sealed)
            .map_err(session_error)?;
        if answer.len() != CHALLENGE_ANSWER_LENGTH {
            return Err(failed("TON Connect challenge answer must contain 32 bytes"));
        }
        Ok(answer)
    }

    /// Authenticates and validates one dApp request relayed by the server.
    ///
    /// `body` is the raw `nonce(24) || box` bytes from the dApp; `now` is the
    /// current Unix time used for `valid_until` validation.
    pub fn decrypt_request(
        &self,
        body: Vec<u8>,
        now: u64,
    ) -> Result<TonConnectDerivedRequest, TonConnectSessionError> {
        let plaintext = self
            .crypto
            .decrypt(self.peer, &body)
            .map_err(session_error)?;
        let request = serde_json::from_slice::<AppRequest>(&plaintext).map_err(session_error)?;
        Ok(TonConnectDerivedRequest {
            request_id: checked_request_id(&request.id),
            request: self.decode(request, now)?,
        })
    }

    /// Encrypts a successful connect event for the dApp.
    ///
    /// `account` must belong to this session; `proof` is included when the
    /// dApp requested `ton_proof`. The device advertises `SendTransaction` and
    /// `SignData` (`text`, `binary`, `cell`).
    pub fn encrypt_connect_event(
        &self,
        event_id: u64,
        account: TonConnectAccountInfo,
        proof: Option<TonConnectProofReply>,
        device: TonConnectDevice,
    ) -> Result<Vec<u8>, TonConnectSessionError> {
        let (account_reply, network) = account_reply(&account)?;
        if network != self.network || account_reply.address != self.address {
            return Err(failed(
                "TON Connect account does not belong to this session",
            ));
        }
        // dApps verify `ton_proof` and `signData` with this key, and the session
        // signs with its current signing key, never with the anchor key.
        if account_reply.public_key.as_bytes() != &self.signing_public_key {
            return Err(failed(
                "TON Connect account must advertise the session's signing public key",
            ));
        }
        let mut items = vec![ConnectItemReply::TonAddress(account_reply)];
        if let Some(proof) = &proof {
            items.push(ConnectItemReply::TonProof(TonProofItemReply::new(
                proof_reply(proof)?,
            )));
        }
        self.encrypt(&ConnectEvent::Connect {
            id: event_id,
            payload: ConnectEventPayload {
                items,
                device: device_info(device)?,
            },
            response: None,
        })
    }

    /// Encrypts a failed-connection event for the dApp.
    pub fn encrypt_connect_error(
        &self,
        event_id: u64,
        code: TonConnectConnectErrorCode,
        message: String,
    ) -> Result<Vec<u8>, TonConnectSessionError> {
        self.encrypt(&ConnectEvent::ConnectError {
            id: event_id,
            payload: ConnectEventError {
                code: connect_error_code(code),
                message: bounded_diagnostic(message),
            },
        })
    }

    /// Encrypts a successful `sendTransaction` response carrying the signed `BoC`.
    pub fn encrypt_send_success(
        &self,
        request_id: String,
        signed_boc: String,
    ) -> Result<Vec<u8>, TonConnectSessionError> {
        let boc = CellBoc::try_from(signed_boc).map_err(session_error)?;
        self.encrypt(&WalletResponse::Success(WalletResponseSuccess {
            result: WalletResult::String(boc.as_str().to_owned()),
            id: request_id,
        }))
    }

    /// Encrypts a successful `signData` response for the signed data.
    ///
    /// The response names this session's wallet address. It is refused unless
    /// the request still matches this session's network and wallet and
    /// `signed_data.signature` verifies with this session's signing public key
    /// for exactly that address, request, domain, and timestamp.
    pub fn encrypt_sign_data_success(
        &self,
        request_id: String,
        signed_data: TonConnectSignedData,
    ) -> Result<Vec<u8>, TonConnectSessionError> {
        if signed_data.public_key.as_slice() != self.signing_public_key {
            return Err(failed(
                "TON Connect signData was not signed with the session's signing key",
            ));
        }
        let signature = <[u8; 64]>::try_from(signed_data.signature.as_slice())
            .map_err(|_| failed("TON Connect signData signature must contain 64 bytes"))?;
        let payload = sign_data_payload(
            &signed_data.request,
            &self.network,
            &self.address,
            &signed_data.domain,
        )?;
        let result = SignDataResult {
            signature: Ed25519Signature::from_bytes(signature),
            address: self.address,
            timestamp: signed_data.timestamp,
            domain: signed_data.domain,
            payload,
        };
        if !result
            .verify(&Ed25519PublicKey::from_bytes(self.signing_public_key))
            .map_err(session_error)?
        {
            return Err(failed(
                "TON Connect signData signature does not match its request",
            ));
        }
        let Value::Object(object) = serde_json::to_value(&result).map_err(session_error)? else {
            return Err(failed("TON Connect signData result must be an object"));
        };
        self.encrypt(&WalletResponse::Success(WalletResponseSuccess {
            result: WalletResult::Object(object),
            id: request_id,
        }))
    }

    /// Encrypts the empty-object result of a dApp-initiated disconnect request.
    pub fn encrypt_disconnect_success(
        &self,
        request_id: String,
    ) -> Result<Vec<u8>, TonConnectSessionError> {
        self.encrypt(&WalletResponse::Success(WalletResponseSuccess {
            result: WalletResult::Object(serde_json::Map::new()),
            id: request_id,
        }))
    }

    /// Encrypts a protocol RPC error response for the selected request.
    pub fn encrypt_error(
        &self,
        request_id: String,
        code: TonConnectRpcErrorCode,
        message: String,
    ) -> Result<Vec<u8>, TonConnectSessionError> {
        self.encrypt(&WalletResponse::Error {
            error: WalletResponseError {
                code: rpc_error_code(code),
                message: bounded_diagnostic(message),
                data: None,
            },
            id: request_id,
        })
    }

    /// Encrypts a wallet-initiated disconnect event.
    pub fn encrypt_disconnect_event(
        &self,
        event_id: u64,
    ) -> Result<Vec<u8>, TonConnectSessionError> {
        self.encrypt(&ConnectEvent::Disconnect {
            id: event_id,
            payload: EmptyObject,
        })
    }
}

/// Builds the advertised runtime descriptor: `SendTransaction` and `SignData`.
fn device_info(device: TonConnectDevice) -> Result<DeviceInfo, TonConnectSessionError> {
    let send = SendTransactionFeature::new(TON_CONNECT_MAX_MESSAGES, Some(false), None)
        .map_err(session_error)?;
    let data = SignDataFeature::new(vec![
        SignDataType::Text,
        SignDataType::Binary,
        SignDataType::Cell,
    ])
    .map_err(session_error)?;
    DeviceInfo::new(
        device_platform(device.platform),
        device.app_name,
        device.app_version,
        u32::from(ton_connect_core::PROTOCOL_VERSION),
        vec![Feature::SendTransaction(send), Feature::SignData(data)],
    )
    .map_err(session_error)
}

/// Converts a validated core payload into the FFI request, keeping every string as sent.
fn sign_data_request(payload: SignDataPayload) -> TonConnectSignDataRequest {
    let (payload, network, from) = match payload {
        SignDataPayload::Text {
            text,
            network,
            from,
        } => (TonConnectSignDataPayload::Text { text }, network, from),
        SignDataPayload::Binary {
            bytes,
            network,
            from,
        } => (
            TonConnectSignDataPayload::Binary {
                bytes: bytes.into_string(),
            },
            network,
            from,
        ),
        SignDataPayload::Cell {
            schema,
            cell,
            network,
            from,
        } => (
            TonConnectSignDataPayload::Cell {
                schema,
                cell: cell.as_str().to_owned(),
            },
            network,
            from,
        ),
    };
    TonConnectSignDataRequest {
        payload,
        network: network.map(NetworkId::into_string),
        from: from.map(|address| address.to_string()),
    }
}

/// Rebuilds the core payload of a request and checks it can be signed here.
///
/// Every string is validated again, since the host may build the record itself,
/// and the original strings are kept, so the result equals the payload the dApp
/// sent. The request must match `network` and `address`, and `domain` must be a
/// bare manifest host.
fn sign_data_payload(
    request: &TonConnectSignDataRequest,
    network: &NetworkId,
    address: &RawAccountAddress,
    domain: &str,
) -> Result<SignDataPayload, TonConnectSessionError> {
    validate_sign_data_domain(domain)?;
    let network_constraint = request
        .network
        .as_deref()
        .map(NetworkId::try_from)
        .transpose()
        .map_err(session_error)?;
    let from = request
        .from
        .as_deref()
        .map(AccountAddress::try_from)
        .transpose()
        .map_err(session_error)?;
    let payload = match &request.payload {
        TonConnectSignDataPayload::Text { text } => SignDataPayload::Text {
            text: text.clone(),
            network: network_constraint,
            from,
        },
        TonConnectSignDataPayload::Binary { bytes } => SignDataPayload::Binary {
            bytes: Base64Value::try_from(bytes.as_str()).map_err(session_error)?,
            network: network_constraint,
            from,
        },
        TonConnectSignDataPayload::Cell { schema, cell } => SignDataPayload::Cell {
            schema: schema.clone(),
            cell: CellBoc::try_from(cell.as_str()).map_err(session_error)?,
            network: network_constraint,
            from,
        },
    };
    payload
        .validate_context(network, address)
        .map_err(session_error)?;
    Ok(payload)
}

/// Computes the digest the wallet signs for a `signData` request.
///
/// It is the digest a dApp rebuilds from the response: `address`, `domain` and
/// `timestamp` are bound into it next to the payload.
pub(crate) fn sign_data_digest(
    request: &TonConnectSignDataRequest,
    network: &NetworkId,
    address: &RawAccountAddress,
    domain: &str,
    timestamp: u64,
) -> Result<[u8; 32], TonConnectSessionError> {
    sign_data_payload(request, network, address, domain)?
        .signing_hash(address, domain, timestamp)
        .map_err(session_error)
}

/// Accepts the manifest host (`app.example`, `localhost:3000`) a `signData`
/// signature binds, and refuses what is clearly not one, such as a URL.
///
/// Text and binary signatures bind the domain bytes as given, so a wrong value
/// still signs but no dApp verifies it; cell signatures additionally require a
/// TEP-81 encodable name, which `signing_hash` enforces.
fn validate_sign_data_domain(domain: &str) -> Result<(), TonConnectSessionError> {
    let is_host = !domain.is_empty()
        && domain.len() <= MAX_SIGN_DATA_DOMAIN_BYTES
        && !domain
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        && !domain.contains(['/', '?', '#', '@']);
    if is_host {
        Ok(())
    } else {
        Err(failed(
            "TON Connect signData domain must be the dApp manifest host",
        ))
    }
}

const fn connect_error_code(code: TonConnectConnectErrorCode) -> ConnectEventErrorCode {
    match code {
        TonConnectConnectErrorCode::ManifestNotFound => ConnectEventErrorCode::ManifestNotFound,
        TonConnectConnectErrorCode::ManifestContent => ConnectEventErrorCode::ManifestContent,
        TonConnectConnectErrorCode::UserDeclined => ConnectEventErrorCode::UserDeclined,
    }
}

/// Accepts only `0` or digits without a leading zero that fit `i64`.
fn checked_request_id(id: &str) -> Option<i64> {
    let value = id.parse::<u64>().ok()?;
    if value.to_string() != id {
        return None;
    }
    i64::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ed25519_dalek::{Signer as _, SigningKey};
    use serde_json::json;
    use ton::block_tlb::{CommonMsgInfo, Msg};
    use ton::ton_core::cell::TonCell;
    use ton::ton_core::traits::tlb::TLB as _;
    use ton::ton_core::types::{TonAddress, tlb_core::TLBCoins};
    use ton::ton_wallet::WalletExtMsgBody;
    use ton_connect_core::{KnownWalletResponse, test_boc};

    use super::*;
    use crate::wallet::send::FreshSendAccount;
    use crate::wallet::transfer::{derive_source, prepare_transfer};
    use crate::{
        Network, NonEmptyString, SendAmount, SendExpiration, SendMessageBody, SendRequest,
        TonAddressString, TonConnectDevicePlatform,
    };

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    /// The dApp client id of the fixed vector (the tweetnacl "alice" public key).
    const PEER: &str = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";

    /// `W` of the fixed vector, computed independently in Python.
    const EXPECTED_W: &str = "a8ed199729d5efd0c212c2542fd7d353e692e42c95e7e97a9e47e2569cbe4f61";

    const OTHER_ADDRESS: &str =
        "0:1111111111111111111111111111111111111111111111111111111111111111";

    const DESTINATION: &str = "Ef8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADAU";

    /// The demo dApp's bounceable destination and its non-bounceable twin.
    const BOUNCEABLE: &str = "EQCKWpx7cNMpvmcN5ObM5lLUZHZRFKqYA4xmw9jOry0ZsF9M";
    const NON_BOUNCEABLE: &str = "UQCKWpx7cNMpvmcN5ObM5lLUZHZRFKqYA4xmw9jOry0ZsAKJ";
    const DESTINATION_RAW: &str =
        "0:8a5a9c7b70d329be670de4e6cce652d464765114aa98038c66c3d8ceaf2d19b0";

    /// The demo dApp's "Hello!" comment payload and contract `stateInit`.
    const DEMO_PAYLOAD: &str = "te6cckEBAQEADAAAFAAAAABIZWxsbyGVgYQo";
    const DEMO_STATE_INIT: &str = "te6cckEBBAEAOgACATQCAQAAART/APSkE/S88sgLAwBI0wHQ0wMBcbCRW+D6QDBwgBDIywVYzxYh+gLLagHPFsmAQPsAlxCarA==";

    /// The public pre-rotation fixture of the `wallet::transfer` tests.
    const PRE_ROTATION_MNEMONIC: &str =
        "notice tortoise soup strong gun divide offer process salon siren general carry";

    /// The throwaway fixed-vector session secret; `crypto.rs` pins its derivation.
    fn fixed_secret() -> Zeroizing<[u8; 32]> {
        Zeroizing::new([
            0xa0, 0xca, 0x52, 0xb8, 0xe8, 0xde, 0x99, 0xd0, 0xf8, 0x34, 0xcb, 0x24, 0x3f, 0x61,
            0xed, 0xd4, 0x07, 0x98, 0x86, 0xb6, 0x4c, 0x19, 0x6c, 0xa0, 0x82, 0xb7, 0xf6, 0x4e,
            0x15, 0x11, 0xdb, 0x5d,
        ])
    }

    /// The account material of a rotated wallet: address and `StateInit` of an
    /// all-zero anchor key, advertised with the current signing key.
    fn account() -> (TonConnectAccountInfo, RawAccountAddress, NetworkId) {
        let (address, state_init) =
            crate::wallet::crypto::derive_wallet_public_state(&[0_u8; 32], Network::Testnet)
                .expect("32-byte public key derives a wallet");
        let info = TonConnectAccountInfo {
            address: address.to_hex(),
            network: "-3".to_owned(),
            wallet_state_init: state_init.to_boc_base64().expect("state init encodes"),
            public_key: signing_public_key().to_vec(),
        };
        let raw = RawAccountAddress::from_str(&info.address).expect("raw address parses");
        let network = NetworkId::try_from("-3").expect("testnet id");
        (info, raw, network)
    }

    fn session(dapp: &SessionCrypto) -> Arc<TonConnectDerivedSession> {
        let (_, address, network) = account();
        TonConnectDerivedSession::new(
            fixed_secret(),
            dapp.client_id(),
            signing_public_key(),
            address,
            network,
        )
    }

    fn w(session: &TonConnectDerivedSession) -> ClientId {
        session.public_key_hex().parse().expect("W is a client id")
    }

    fn device() -> TonConnectDevice {
        TonConnectDevice {
            platform: TonConnectDevicePlatform::Mac,
            app_name: "telegram".to_owned(),
            app_version: "6.3.1".to_owned(),
        }
    }

    fn send_request(
        id: &str,
        network: &str,
        from: &str,
        valid_until: u64,
        messages: usize,
    ) -> AppRequest {
        let message = json!({
            "address": DESTINATION,
            "amount": "1000000",
            "payload": "te6ccgEBAQEAAgAAAA=="
        });
        AppRequest {
            method: "sendTransaction".to_owned(),
            params: vec![
                json!({
                    "valid_until": valid_until,
                    "network": network,
                    "from": from,
                    "messages": vec![message; messages],
                })
                .to_string(),
            ],
            id: id.to_owned(),
        }
    }

    fn request_with_messages(id: &str, from: &str, messages: Vec<Value>) -> AppRequest {
        AppRequest {
            method: "sendTransaction".to_owned(),
            params: vec![
                json!({
                    "valid_until": 1_900_000_000_u64,
                    "network": "-3",
                    "from": from,
                    "messages": messages,
                })
                .to_string(),
            ],
            id: id.to_owned(),
        }
    }

    /// Decrypts one relayed request and returns its decoded `sendTransaction`.
    fn decode_send(
        dapp: &SessionCrypto,
        session: &TonConnectDerivedSession,
        request: &AppRequest,
    ) -> Result<SendRequest, Box<dyn std::error::Error>> {
        let decoded =
            session.decrypt_request(encrypt_for(dapp, session, request)?, 1_800_000_000)?;
        let TonConnectIncomingRequest::SendTransaction { request, .. } = decoded.request else {
            return Err("request was not decoded as sendTransaction".into());
        };
        Ok(request)
    }

    /// Signs a decoded request as the engine send path does and returns the
    /// internal messages of the signed external message, in order.
    fn signed_internal_messages(
        request: &SendRequest,
    ) -> Result<Vec<Msg<TonCell>>, Box<dyn std::error::Error>> {
        let source = derive_source(PRE_ROTATION_MNEMONIC.as_bytes(), Network::Testnet)?;
        let source = TonAddressString::from_address(&source, Network::Testnet);
        let prepared = prepare_transfer(
            PRE_ROTATION_MNEMONIC.as_bytes(),
            &NonEmptyString::try_from("record")?,
            &source,
            Network::Testnet,
            request,
            &FreshSendAccount {
                status: crate::AccountStatus::Active,
                seqno: 7,
            },
            1_900_000_000,
        )?;
        let external = Msg::<TonCell>::from_boc(prepared.signed_boc.as_bytes().to_vec())?;
        if !matches!(external.info, CommonMsgInfo::ExtIn(_)) {
            return Err("signed transfer must be an external message".into());
        }
        let (body, _) = WalletExtMsgBody::read_signed(&mut external.body.value.parser())?;
        Ok(body
            .msgs
            .iter()
            .map(Msg::<TonCell>::from_cell)
            .collect::<Result<Vec<_>, _>>()?)
    }

    fn disconnect_request(id: &str) -> AppRequest {
        AppRequest {
            method: "disconnect".to_owned(),
            params: Vec::new(),
            id: id.to_owned(),
        }
    }

    fn encrypt_for(
        dapp: &SessionCrypto,
        session: &TonConnectDerivedSession,
        request: &AppRequest,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        Ok(dapp.encrypt(w(session), &serde_json::to_vec(request)?)?)
    }

    fn decrypt_json(
        dapp: &SessionCrypto,
        session: &TonConnectDerivedSession,
        bytes: &[u8],
    ) -> Result<Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_slice(&dapp.decrypt(w(session), bytes)?)?)
    }

    /// The sorted keys of a JSON object.
    fn keys(value: &Value) -> Result<Vec<&str>, Box<dyn std::error::Error>> {
        let object = value.as_object().ok_or("value must be an object")?;
        let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
        keys.sort_unstable();
        Ok(keys)
    }

    const SIGN_DATA_TEXT: &str = "Confirm new 2fa number:\n+1 *** *** ** 89";
    const SIGN_DATA_BYTES: &str = "I0hlbGxvLCBXb3JsZCE=";
    const SIGN_DATA_SCHEMA: &str =
        "message#_ len:uint7 {len <= 127} text:(bits len * 8) = Message;";
    const SIGN_DATA_DOMAIN: &str = "tonconnect-sdk-demo-dapp.vercel.app";
    const SIGN_DATA_TIMESTAMP: u64 = 1_800_000_000;

    fn sign_data_app_request(id: &str, payload: &Value) -> AppRequest {
        AppRequest {
            method: "signData".to_owned(),
            params: vec![payload.to_string()],
            id: id.to_owned(),
        }
    }

    /// The three payload types, each with the FFI payload it must decode to.
    fn sign_data_payloads() -> [(Value, TonConnectSignDataPayload); 3] {
        [
            (
                json!({"type": "text", "text": SIGN_DATA_TEXT}),
                TonConnectSignDataPayload::Text {
                    text: SIGN_DATA_TEXT.to_owned(),
                },
            ),
            (
                json!({"type": "binary", "bytes": SIGN_DATA_BYTES}),
                TonConnectSignDataPayload::Binary {
                    bytes: SIGN_DATA_BYTES.to_owned(),
                },
            ),
            (
                json!({"type": "cell", "schema": SIGN_DATA_SCHEMA, "cell": DEMO_PAYLOAD}),
                TonConnectSignDataPayload::Cell {
                    schema: SIGN_DATA_SCHEMA.to_owned(),
                    cell: DEMO_PAYLOAD.to_owned(),
                },
            ),
        ]
    }

    /// The session wallet in user-friendly form.
    fn friendly_session_address() -> Result<String, Box<dyn std::error::Error>> {
        let (info, _, _) = account();
        let address = TonAddress::from_str(&info.address)?;
        Ok(TonAddressString::from_address(&address, Network::Testnet).into_string())
    }

    /// A throwaway signer standing in for the wallet's current signing key.
    fn signer() -> SigningKey {
        SigningKey::from_bytes(&[0x55; 32])
    }

    fn signing_public_key() -> [u8; 32] {
        signer().verifying_key().to_bytes()
    }

    fn other_public_key() -> Ed25519PublicKey {
        Ed25519PublicKey::from_bytes(
            SigningKey::from_bytes(&[0x66; 32])
                .verifying_key()
                .to_bytes(),
        )
    }

    /// Signs `request` for `address` the way `WalletLifecycle::sign_ton_connect_data` does.
    fn signed_data(
        key: &SigningKey,
        address: RawAccountAddress,
        request: TonConnectSignDataRequest,
    ) -> Result<TonConnectSignedData, Box<dyn std::error::Error>> {
        let (_, _, network) = account();
        let hash = sign_data_digest(
            &request,
            &network,
            &address,
            SIGN_DATA_DOMAIN,
            SIGN_DATA_TIMESTAMP,
        )?;
        Ok(TonConnectSignedData {
            request,
            domain: SIGN_DATA_DOMAIN.to_owned(),
            timestamp: SIGN_DATA_TIMESTAMP,
            signature: key.sign(&hash).to_bytes().to_vec(),
            public_key: key.verifying_key().to_bytes().to_vec(),
        })
    }

    /// Decrypts one relayed request and returns its decoded `signData`.
    fn decode_sign_data(
        dapp: &SessionCrypto,
        session: &TonConnectDerivedSession,
        request: &AppRequest,
    ) -> Result<TonConnectSignDataRequest, Box<dyn std::error::Error>> {
        let decoded =
            session.decrypt_request(encrypt_for(dapp, session, request)?, 1_800_000_000)?;
        let TonConnectIncomingRequest::SignData { request, .. } = decoded.request else {
            return Err(format!("request was not decoded as signData: {decoded:?}").into());
        };
        Ok(request)
    }

    #[test]
    fn derived_public_key_matches_the_fixed_vector() -> TestResult {
        let peer = PEER.parse::<ClientId>()?;
        let (_, address, network) = account();
        let session =
            TonConnectDerivedSession::new(fixed_secret(), peer, [7_u8; 32], address, network);
        assert_eq!(session.public_key_hex(), EXPECTED_W);
        Ok(())
    }

    #[test]
    fn dapp_decrypts_the_connect_event_with_its_features() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();
        let bytes = session.encrypt_connect_event(5, info, None, device())?;
        let plaintext = dapp.decrypt(w(&session), &bytes)?;
        assert_eq!(bytes.len(), plaintext.len() + 40);

        let value: Value = serde_json::from_slice(&plaintext)?;
        assert_eq!(value["event"], "connect");
        assert_eq!(value["id"], 5);
        let items = value["payload"]["items"]
            .as_array()
            .ok_or("items must be an array")?;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["name"], "ton_addr");
        let device = &value["payload"]["device"];
        assert_eq!(device["appName"], "telegram");
        assert_eq!(device["appVersion"], "6.3.1");
        assert_eq!(device["platform"], "mac");
        assert_eq!(device["maxProtocolVersion"], 2);
        assert_eq!(
            device["features"],
            json!([
                {
                    "name": "SendTransaction",
                    "maxMessages": 255,
                    "extraCurrencySupported": false
                },
                {
                    "name": "SignData",
                    "types": ["text", "binary", "cell"]
                }
            ])
        );
        let features = device["features"]
            .as_array()
            .ok_or("features must be an array")?;
        assert_eq!(features.len(), 2);
        for feature in features {
            assert_ne!(feature["name"], "SignMessage");
            assert_ne!(feature["name"], "EmbeddedRequest");
        }
        assert!(value.get("response").is_none());
        let _ = serde_json::from_slice::<ConnectEvent>(&plaintext)?;
        Ok(())
    }

    #[test]
    fn connect_event_without_proof_keeps_the_core_bytes() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();
        let bytes = session.encrypt_connect_event(5, info.clone(), None, device())?;
        let plaintext = String::from_utf8(dapp.decrypt(w(&session), &bytes)?)?;

        let expected = format!(
            concat!(
                r#"{{"event":"connect","id":5,"payload":{{"items":[{{"name":"ton_addr","#,
                r#""address":"{}","network":"-3","walletStateInit":"{}","publicKey":"{}"}}],"#,
                r#""device":{{"platform":"mac","appName":"telegram","appVersion":"6.3.1","#,
                r#""maxProtocolVersion":2,"features":[{{"name":"SendTransaction","#,
                r#""maxMessages":255,"extraCurrencySupported":false}},"#,
                r#"{{"name":"SignData","types":["text","binary","cell"]}}]}}}}}}"#
            ),
            info.address,
            info.wallet_state_init,
            signing_public_key()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        assert_eq!(plaintext, expected);

        let (account_item, _) = account_reply(&info)?;
        let core = serde_json::to_string(&ConnectEvent::Connect {
            id: 5,
            payload: ConnectEventPayload {
                items: vec![ConnectItemReply::TonAddress(account_item)],
                device: device_info(device())?,
            },
            response: None,
        })?;
        assert_eq!(plaintext, core);
        Ok(())
    }

    #[test]
    fn dapp_decrypts_the_connect_event_with_proof() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();
        let proof = TonConnectProofReply {
            timestamp: 1_800_000_000,
            domain: "app.example".to_owned(),
            payload: "challenge".to_owned(),
            signature: vec![9_u8; 64],
        };
        let bytes =
            session.encrypt_connect_event(6, info.clone(), Some(proof.clone()), device())?;
        let plaintext = dapp.decrypt(w(&session), &bytes)?;
        let value: Value = serde_json::from_slice(&plaintext)?;
        let items = value["payload"]["items"]
            .as_array()
            .ok_or("items must be an array")?;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["name"], "ton_addr");
        assert_eq!(items[1]["name"], "ton_proof");

        // The checks `@tonconnect/sdk` 4.0.2 runs before it keeps a `ton_proof`
        // item; on any failure it replaces the item by an error. The signed
        // bytes are unaffected by the JSON form of `timestamp`: the signature
        // covers the little-endian 64-bit value.
        assert_eq!(keys(&items[1])?, ["name", "proof"]);
        let item = &items[1]["proof"];
        assert_eq!(keys(item)?, ["domain", "payload", "signature", "timestamp"]);
        assert!(item["timestamp"].is_u64());
        assert_eq!(item["timestamp"], 1_800_000_000_u64);
        assert!(item["domain"]["lengthBytes"].is_u64());
        assert_eq!(item["domain"]["lengthBytes"], "app.example".len());
        assert_eq!(item["domain"]["value"], "app.example");
        assert_eq!(item["payload"], "challenge");
        let signature = item["signature"]
            .as_str()
            .ok_or("signature must be a string")?;
        assert_eq!(signature.len(), 88);
        assert!(signature.ends_with("=="));
        assert_eq!(signature, STANDARD.encode([9_u8; 64]));
        assert_eq!(STANDARD.decode(signature)?, vec![9_u8; 64]);

        // The reply is exactly the core serialization, which writes the number.
        let (account_item, _) = account_reply(&info)?;
        let core = serde_json::to_string(&ConnectEvent::Connect {
            id: 6,
            payload: ConnectEventPayload {
                items: vec![
                    ConnectItemReply::TonAddress(account_item),
                    ConnectItemReply::TonProof(TonProofItemReply::new(proof_reply(&proof)?)),
                ],
                device: device_info(device())?,
            },
            response: None,
        })?;
        assert_eq!(String::from_utf8(plaintext)?, core);

        // Every `u64` is written as its exact decimal digits, never as a float.
        let mut latest = proof.clone();
        latest.timestamp = u64::MAX;
        let bytes = session.encrypt_connect_event(6, info.clone(), Some(latest), device())?;
        let plaintext = String::from_utf8(dapp.decrypt(w(&session), &bytes)?)?;
        assert!(plaintext.contains(r#""timestamp":18446744073709551615,"#));

        let mut short = proof;
        short.signature = vec![9_u8; 63];
        assert!(matches!(
            session.encrypt_connect_event(6, info, Some(short), device()),
            Err(TonConnectSessionError::Failed { .. })
        ));
        Ok(())
    }

    #[test]
    fn connect_error_codes_are_numeric() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let cases = [
            (TonConnectConnectErrorCode::ManifestNotFound, 2_u64, 21_u64),
            (TonConnectConnectErrorCode::ManifestContent, 3, 22),
            (TonConnectConnectErrorCode::UserDeclined, 300, 23),
        ];
        for (code, wire, event_id) in cases {
            let bytes = session.encrypt_connect_error(event_id, code, "declined".to_owned())?;
            let value = decrypt_json(&dapp, &session, &bytes)?;
            assert_eq!(value["event"], "connect_error");
            assert_eq!(value["id"], event_id);
            assert_eq!(value["payload"]["code"], wire);
            assert_eq!(value["payload"]["message"], "declined");
        }
        Ok(())
    }

    #[test]
    fn we_decrypt_the_dapp_send_transaction_request() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();
        let request = send_request("7", "-3", &info.address, 1_900_000_000, 1);
        let body = encrypt_for(&dapp, &session, &request)?;
        let decoded = session.decrypt_request(body, 1_800_000_000)?;
        assert_eq!(decoded.request_id, Some(7));
        let TonConnectIncomingRequest::SendTransaction {
            id,
            method,
            request,
        } = decoded.request
        else {
            return Err("request was not decoded as sendTransaction".into());
        };
        assert_eq!(id, "7");
        assert_eq!(method, "sendTransaction");
        assert_eq!(
            request.operation_id.as_str(),
            format!("ton-connect:{}:7", session.public_key_hex())
        );
        assert_eq!(
            request.intent.expiration,
            SendExpiration::Exact {
                unix_timestamp: 1_900_000_000,
            }
        );
        assert_eq!(request.intent.messages.len(), 1);
        assert!(!request.force);
        Ok(())
    }

    #[test]
    fn send_transaction_messages_carry_their_address_bounce_flag() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();
        let request = request_with_messages(
            "8",
            &info.address,
            vec![
                json!({ "address": BOUNCEABLE, "amount": "1" }),
                json!({ "address": NON_BOUNCEABLE, "amount": "2" }),
            ],
        );
        let request = decode_send(&dapp, &session, &request)?;
        assert!(!request.force);
        let messages = &request.intent.messages;
        assert_eq!(messages.len(), 2);
        assert!(messages[0].bounce, "EQ destination must bounce");
        assert!(!messages[1].bounce, "UQ destination must not bounce");
        assert_eq!(messages[0].amount, SendAmount::exact("1")?);
        assert_eq!(messages[1].amount, SendAmount::exact("2")?);

        let destination = TonAddress::from_str(DESTINATION_RAW)?.to_msg_address();
        let internal = signed_internal_messages(&request)?;
        assert_eq!(internal.len(), 2);
        let mut flags = Vec::new();
        for (message, amount) in internal.iter().zip([1_u128, 2]) {
            let CommonMsgInfo::Int(info) = &message.info else {
                return Err("wallet message must be internal".into());
            };
            assert!(!info.bounced);
            assert_eq!(info.value.coins, TLBCoins::new(amount));
            assert_eq!(info.dst, destination);
            flags.push(info.bounce);
        }
        assert_eq!(flags, [true, false]);
        Ok(())
    }

    #[test]
    fn demo_dapp_request_bounces_and_keeps_payload_and_state_init() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();
        let request = request_with_messages(
            "9",
            &info.address,
            vec![json!({
                "address": BOUNCEABLE,
                "amount": "5000000",
                "payload": DEMO_PAYLOAD,
                "stateInit": DEMO_STATE_INIT,
            })],
        );
        let request = decode_send(&dapp, &session, &request)?;
        let payload = STANDARD.decode(DEMO_PAYLOAD)?;
        let state_init = STANDARD.decode(DEMO_STATE_INIT)?;
        let [message] = request.intent.messages.as_slice() else {
            return Err("demo request must decode to one message".into());
        };
        assert!(message.bounce, "EQ destination must bounce");
        assert_eq!(message.amount, SendAmount::exact("5000000")?);
        let SendMessageBody::RawPayload { boc } = &message.body else {
            return Err("demo payload must stay a raw payload".into());
        };
        assert_eq!(boc.as_bytes(), payload.as_slice());
        assert_eq!(
            message.state_init.as_ref().map(|boc| boc.as_bytes()),
            Some(state_init.as_slice())
        );

        let internal = signed_internal_messages(&request)?;
        let [signed] = internal.as_slice() else {
            return Err("demo transfer must carry one internal message".into());
        };
        let CommonMsgInfo::Int(info) = &signed.info else {
            return Err("wallet message must be internal".into());
        };
        assert!(info.bounce);
        assert!(!info.bounced);
        assert_eq!(
            signed.body.value.hash()?,
            TonCell::from_boc(payload)?.hash()?
        );
        let init = signed.init.as_ref().ok_or("stateInit must be attached")?;
        assert_eq!(
            init.value.cell_hash()?,
            TonCell::from_boc(state_init)?.cell_hash()?
        );
        Ok(())
    }

    #[test]
    fn request_validation_matches_the_classic_path() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();
        let now = 1_800_000_000;

        let bad_requests = [
            send_request("1", "-239", &info.address, 1_900_000_000, 1),
            send_request("2", "-3", OTHER_ADDRESS, 1_900_000_000, 1),
            send_request("3", "-3", &info.address, 1_700_000_000, 1),
        ];
        for request in bad_requests {
            let decoded = session.decrypt_request(encrypt_for(&dapp, &session, &request)?, now)?;
            assert!(
                matches!(
                    decoded.request,
                    TonConnectIncomingRequest::Unsupported {
                        error_code: TonConnectRpcErrorCode::BadRequest,
                        ..
                    }
                ),
                "{:?}",
                decoded.request
            );
        }

        let too_many = send_request("4", "-3", &info.address, 1_900_000_000, 256);
        let decoded = session.decrypt_request(encrypt_for(&dapp, &session, &too_many)?, now)?;
        assert!(matches!(
            decoded.request,
            TonConnectIncomingRequest::Unsupported {
                error_code: TonConnectRpcErrorCode::MethodNotSupported,
                ..
            }
        ));

        let mut sign = send_request("5", "-3", &info.address, 1_900_000_000, 1);
        sign.method = "signMessage".to_owned();
        let decoded = session.decrypt_request(encrypt_for(&dapp, &session, &sign)?, now)?;
        assert!(matches!(
            decoded.request,
            TonConnectIncomingRequest::Unsupported {
                error_code: TonConnectRpcErrorCode::MethodNotSupported,
                ..
            }
        ));

        let decoded = session
            .decrypt_request(encrypt_for(&dapp, &session, &disconnect_request("6"))?, now)?;
        assert_eq!(
            decoded.request,
            TonConnectIncomingRequest::Disconnect {
                id: "6".to_owned(),
                method: "disconnect".to_owned(),
            }
        );
        assert_eq!(decoded.request_id, Some(6));
        Ok(())
    }

    #[test]
    fn dapp_decrypts_our_responses() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);

        let bytes =
            session.encrypt_send_success("7".to_owned(), "te6ccgEBAQEAAgAAAA==".to_owned())?;
        assert_eq!(
            decrypt_json(&dapp, &session, &bytes)?,
            json!({"result": "te6ccgEBAQEAAgAAAA==", "id": "7"})
        );
        assert!(matches!(
            session.encrypt_send_success("7".to_owned(), "not a boc".to_owned()),
            Err(TonConnectSessionError::Failed { .. })
        ));

        let bytes = session.encrypt_disconnect_success("8".to_owned())?;
        assert_eq!(
            decrypt_json(&dapp, &session, &bytes)?,
            json!({"result": {}, "id": "8"})
        );

        let bytes = session.encrypt_error(
            "9".to_owned(),
            TonConnectRpcErrorCode::UserDeclined,
            "no".to_owned(),
        )?;
        let value = decrypt_json(&dapp, &session, &bytes)?;
        assert_eq!(value["error"]["code"], 300);
        assert_eq!(value["error"]["message"], "no");
        assert_eq!(value["id"], "9");
        assert!(value["error"].get("data").is_none());

        let bytes = session.encrypt_disconnect_event(11)?;
        assert_eq!(
            decrypt_json(&dapp, &session, &bytes)?,
            json!({"event": "disconnect", "id": 11, "payload": {}})
        );
        Ok(())
    }

    #[test]
    fn challenge_round_trips_against_a_box_sealed_to_w() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let ephemeral = SessionCrypto::generate()?;
        let answer = [0x42_u8; 32];
        let sealed = ephemeral.encrypt(w(&session), &answer)?;
        assert_eq!(sealed.len(), 72);
        let mut challenge = ephemeral.client_id().to_bytes().to_vec();
        challenge.extend_from_slice(&sealed);
        assert_eq!(challenge.len(), CHALLENGE_LENGTH);
        assert_eq!(session.open_challenge(challenge.clone())?, answer.to_vec());

        let mut tampered = challenge;
        tampered[CHALLENGE_LENGTH - 1] ^= 1;
        assert!(matches!(
            session.open_challenge(tampered),
            Err(TonConnectSessionError::Failed { .. })
        ));
        Ok(())
    }

    #[test]
    fn garbage_and_truncated_inputs_return_typed_errors() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let now = 1_800_000_000;

        let mut flipped = encrypt_for(&dapp, &session, &disconnect_request("1"))?;
        if let Some(last) = flipped.last_mut() {
            *last ^= 1;
        }
        let bodies = [
            Vec::new(),
            vec![0_u8; 39],
            vec![0_u8; 40],
            (0..200).map(|i| i as u8).collect(),
            flipped,
        ];
        for body in bodies {
            assert!(
                matches!(
                    session.decrypt_request(body.clone(), now),
                    Err(TonConnectSessionError::Failed { .. })
                ),
                "body of {} bytes",
                body.len()
            );
        }

        for length in [0, 103, 105, 104] {
            assert!(
                matches!(
                    session.open_challenge(vec![0_u8; length]),
                    Err(TonConnectSessionError::Failed { .. })
                ),
                "challenge of {length} bytes"
            );
        }

        for plaintext in [b"{}".as_slice(), b"not json".as_slice()] {
            let body = dapp.encrypt(w(&session), plaintext)?;
            assert!(matches!(
                session.decrypt_request(body, now),
                Err(TonConnectSessionError::Failed { .. })
            ));
        }
        Ok(())
    }

    #[test]
    fn request_ids_outside_canonical_i64_are_reported_distinctly() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let now = 1_800_000_000;

        let accepted = [("0", 0_i64), ("7", 7), ("9223372036854775807", i64::MAX)];
        for (id, expected) in accepted {
            let decoded = session
                .decrypt_request(encrypt_for(&dapp, &session, &disconnect_request(id))?, now)?;
            assert_eq!(decoded.request_id, Some(expected), "{id:?}");
            assert_eq!(
                decoded.request,
                TonConnectIncomingRequest::Disconnect {
                    id: id.to_owned(),
                    method: "disconnect".to_owned(),
                }
            );
        }

        let rejected = [
            "",
            "007",
            "+7",
            "-1",
            "7a",
            " 7",
            "9223372036854775808",
            "18446744073709551616",
        ];
        for id in rejected {
            let decoded = session
                .decrypt_request(encrypt_for(&dapp, &session, &disconnect_request(id))?, now)?;
            assert_eq!(decoded.request_id, None, "{id:?}");
            assert_eq!(
                decoded.request,
                TonConnectIncomingRequest::Disconnect {
                    id: id.to_owned(),
                    method: "disconnect".to_owned(),
                }
            );
        }
        Ok(())
    }

    #[test]
    fn checked_request_id_accepts_only_canonical_i64_decimals() {
        assert_eq!(checked_request_id("0"), Some(0));
        assert_eq!(checked_request_id("7"), Some(7));
        assert_eq!(checked_request_id("9223372036854775807"), Some(i64::MAX));
        for id in [
            "",
            "007",
            "+7",
            "-1",
            "7a",
            " 7",
            "9223372036854775808",
            "18446744073709551616",
        ] {
            assert_eq!(checked_request_id(id), None, "{id:?}");
        }
    }

    #[test]
    fn sign_data_requests_decode_with_their_payload() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();
        let friendly = friendly_session_address()?;
        assert_ne!(friendly, info.address);
        let mut next_id = 20_i64;
        for (payload, expected) in sign_data_payloads() {
            let constraints = [
                (None, None),
                (Some("-3"), Some(info.address.as_str())),
                (None, Some(friendly.as_str())),
            ];
            for (network, from) in constraints {
                let mut payload = payload.clone();
                let object = payload.as_object_mut().ok_or("payload is an object")?;
                if let Some(network) = network {
                    let _ = object.insert("network".to_owned(), json!(network));
                }
                if let Some(from) = from {
                    let _ = object.insert("from".to_owned(), json!(from));
                }
                next_id += 1;
                let id = next_id.to_string();
                let request = sign_data_app_request(&id, &payload);
                let decoded = session
                    .decrypt_request(encrypt_for(&dapp, &session, &request)?, 1_800_000_000)?;
                assert_eq!(decoded.request_id, Some(next_id));
                assert_eq!(
                    decoded.request,
                    TonConnectIncomingRequest::SignData {
                        id,
                        method: "signData".to_owned(),
                        request: TonConnectSignDataRequest {
                            payload: expected.clone(),
                            network: network.map(str::to_owned),
                            from: from.map(str::to_owned),
                        },
                    }
                );
            }
        }
        Ok(())
    }

    #[test]
    fn sign_data_requests_that_cannot_be_signed_are_bad_requests() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let now = 1_800_000_000;
        let bad_payloads = [
            json!({"type": "text", "text": "hi", "network": "-239"}),
            json!({"type": "text", "text": "hi", "from": OTHER_ADDRESS}),
            json!({"type": "binary", "bytes": "not base64!"}),
            json!({"type": "binary", "bytes": "+/8_"}),
            json!({"type": "cell", "schema": SIGN_DATA_SCHEMA, "cell": "not a boc"}),
            json!({"type": "cell", "schema": SIGN_DATA_SCHEMA, "cell": "AAAA"}),
            json!({"type": "cell", "schema": SIGN_DATA_SCHEMA, "cell": null}),
            json!({"type": "unknown", "text": "hi"}),
            json!({"type": "text", "text": "hi", "extra": 1}),
        ];
        for (index, payload) in bad_payloads.iter().enumerate() {
            let id = format!("{}", index + 30);
            let request = sign_data_app_request(&id, payload);
            let decoded = session.decrypt_request(encrypt_for(&dapp, &session, &request)?, now)?;
            let TonConnectIncomingRequest::Unsupported {
                id: decoded_id,
                method,
                error_code,
                error_message,
            } = decoded.request
            else {
                return Err(format!("{payload} must be a bad request: {decoded:?}").into());
            };
            assert_eq!(decoded_id, id);
            assert_eq!(method, "signData");
            assert_eq!(error_code, TonConnectRpcErrorCode::BadRequest, "{payload}");
            if index == 0 {
                // The caller answers with exactly this error: code 1 on the wire.
                let bytes = session.encrypt_error(id.clone(), error_code, error_message)?;
                let value = decrypt_json(&dapp, &session, &bytes)?;
                assert_eq!(value["error"]["code"], 1);
                assert_eq!(
                    value["error"]["message"],
                    "TON Connect request network differs from the active network"
                );
                assert_eq!(value["id"], id);
            }
        }
        Ok(())
    }

    #[test]
    fn sign_message_and_unknown_methods_decode_as_before() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();
        let now = 1_800_000_000;

        let mut sign = send_request("40", "-3", &info.address, 1_900_000_000, 1);
        sign.method = "signMessage".to_owned();
        let decoded = session.decrypt_request(encrypt_for(&dapp, &session, &sign)?, now)?;
        assert_eq!(
            decoded.request,
            TonConnectIncomingRequest::Unsupported {
                id: "40".to_owned(),
                method: "signMessage".to_owned(),
                error_code: TonConnectRpcErrorCode::MethodNotSupported,
                error_message: "Method is not supported".to_owned(),
            }
        );

        let (text, _) = sign_data_payloads()[0].clone();
        for method in ["signDataV2", "foo"] {
            let mut request = sign_data_app_request("41", &text);
            request.method = method.to_owned();
            let decoded = session.decrypt_request(encrypt_for(&dapp, &session, &request)?, now)?;
            let TonConnectIncomingRequest::Unsupported {
                method: decoded_method,
                error_code,
                error_message,
                ..
            } = decoded.request
            else {
                return Err(format!("{method} must be unsupported: {decoded:?}").into());
            };
            assert_eq!(decoded_method, method);
            assert_eq!(error_code, TonConnectRpcErrorCode::BadRequest);
            assert!(
                error_message.starts_with("unsupported TON Connect RPC method"),
                "{error_message}"
            );
        }
        Ok(())
    }

    #[test]
    fn dapp_decrypts_the_sign_data_response() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, address, _) = account();
        let key = signer();
        let public_key = Ed25519PublicKey::from_bytes(key.verifying_key().to_bytes());
        for (payload, _) in sign_data_payloads() {
            let app_request = sign_data_app_request("11", &payload);
            let request = decode_sign_data(&dapp, &session, &app_request)?;
            let signed = signed_data(&key, address, request)?;
            let bytes = session.encrypt_sign_data_success("11".to_owned(), signed)?;
            let plaintext = dapp.decrypt(w(&session), &bytes)?;

            let value: Value = serde_json::from_slice(&plaintext)?;
            assert_eq!(keys(&value)?, ["id", "result"]);
            assert_eq!(value["id"], "11");
            let result = &value["result"];
            assert_eq!(
                keys(result)?,
                ["address", "domain", "payload", "signature", "timestamp"]
            );
            assert!(result["timestamp"].is_u64());
            assert_eq!(result["timestamp"], SIGN_DATA_TIMESTAMP);
            assert_eq!(result["address"], info.address.as_str());
            assert_eq!(result["domain"], SIGN_DATA_DOMAIN);
            assert_eq!(result["payload"], payload);

            let response = serde_json::from_slice::<WalletResponse>(&plaintext)?;
            let KnownWalletResponse::SignData(result) =
                response.validate_for(&app_request.decode()?)?
            else {
                return Err("response must be a signData result".into());
            };
            assert_eq!(result.address, address);
            assert_eq!(result.domain, SIGN_DATA_DOMAIN);
            assert_eq!(result.timestamp, SIGN_DATA_TIMESTAMP);
            assert!(result.verify(&public_key)?);
            assert!(!result.verify(&other_public_key())?);
        }
        Ok(())
    }

    #[test]
    fn sign_data_response_refuses_a_signature_that_does_not_match() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (_, address, _) = account();
        let key = signer();
        let request = TonConnectSignDataRequest {
            payload: TonConnectSignDataPayload::Text {
                text: SIGN_DATA_TEXT.to_owned(),
            },
            network: None,
            from: None,
        };
        let signed = signed_data(&key, address, request.clone())?;
        let _ = session.encrypt_sign_data_success("12".to_owned(), signed.clone())?;

        let other_address = RawAccountAddress::from_str(OTHER_ADDRESS)?;
        let mut cases = vec![signed_data(&key, other_address, request)?];
        let mut other_domain = signed.clone();
        other_domain.domain = "other.example".to_owned();
        cases.push(other_domain);
        let mut other_timestamp = signed.clone();
        other_timestamp.timestamp += 1;
        cases.push(other_timestamp);
        let mut altered_text = signed.clone();
        altered_text.request.payload = TonConnectSignDataPayload::Text {
            text: "Confirm new 2fa number:\n+1 *** *** ** 88".to_owned(),
        };
        cases.push(altered_text);
        let mut wrong_key = signed.clone();
        wrong_key.public_key = other_public_key().as_bytes().to_vec();
        cases.push(wrong_key);
        // Consistent in itself, but made by a key that is not the session's.
        let other_signer = SigningKey::from_bytes(&[0x66; 32]);
        let other_request = signed.request.clone();
        cases.push(signed_data(&other_signer, address, other_request.clone())?);
        let mut claimed_key = signed_data(&other_signer, address, other_request)?;
        claimed_key.public_key = signing_public_key().to_vec();
        cases.push(claimed_key);
        let mut short_signature = signed.clone();
        short_signature.signature.truncate(63);
        cases.push(short_signature);
        let mut short_key = signed.clone();
        short_key.public_key.truncate(31);
        cases.push(short_key);
        let mut garbage = signed.clone();
        garbage.request.payload = TonConnectSignDataPayload::Binary {
            bytes: "not base64!".to_owned(),
        };
        cases.push(garbage);
        let mut garbage_network = signed.clone();
        garbage_network.request.network = Some("mainnet".to_owned());
        cases.push(garbage_network);
        let mut garbage_from = signed;
        garbage_from.request.from = Some("nobody".to_owned());
        cases.push(garbage_from);

        for signed in cases {
            assert!(
                matches!(
                    session.encrypt_sign_data_success("12".to_owned(), signed.clone()),
                    Err(TonConnectSessionError::Failed { .. })
                ),
                "{signed:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn sign_data_domains_must_be_manifest_hosts() -> TestResult {
        let (_, address, network) = account();
        let request = TonConnectSignDataRequest {
            payload: TonConnectSignDataPayload::Text {
                text: SIGN_DATA_TEXT.to_owned(),
            },
            network: None,
            from: None,
        };
        let digest = |domain: &str| {
            sign_data_digest(&request, &network, &address, domain, SIGN_DATA_TIMESTAMP)
        };
        for domain in [SIGN_DATA_DOMAIN, "localhost:3000", "пример.рф"] {
            assert!(digest(domain).is_ok(), "{domain}");
        }
        let too_long = "a".repeat(MAX_SIGN_DATA_DOMAIN_BYTES + 1);
        for domain in [
            "",
            "https://app.example",
            "app.example/connect",
            "app.example?x=1",
            "user@app.example",
            "app example",
            "app.example\n",
            too_long.as_str(),
        ] {
            assert!(digest(domain).is_err(), "{domain:?}");
        }
        Ok(())
    }

    /// A standard base64 `BoC` of `levels` data-less cells, each referencing the next one.
    fn deep_chain_cell(levels: usize) -> String {
        STANDARD.encode(test_boc::chain(levels))
    }

    /// Runs `job` on a 512 KiB stack: Telegram decrypts and decodes requests on a
    /// default `std::thread`, which gets 512 KiB on macOS.
    fn on_worker_stack(job: impl FnOnce() -> TestResult + Send + 'static) -> TestResult {
        std::thread::Builder::new()
            .name("wallet-worker-512k".into())
            .stack_size(512 * 1024)
            .spawn(move || job().map_err(|error| error.to_string()))
            .expect("spawn")
            .join()
            .expect("the job must not panic")?;
        Ok(())
    }

    #[test]
    fn deep_sign_data_cells_are_refused_on_a_512_kib_stack() -> TestResult {
        on_worker_stack(|| {
            let dapp = SessionCrypto::generate()?;
            let session = session(&dapp);
            let cell = deep_chain_cell(5000);
            let payload = json!({"type": "cell", "schema": SIGN_DATA_SCHEMA, "cell": cell});
            let request = sign_data_app_request("50", &payload);
            let decoded =
                session.decrypt_request(encrypt_for(&dapp, &session, &request)?, 1_800_000_000)?;
            let TonConnectIncomingRequest::Unsupported {
                id,
                method,
                error_code,
                ..
            } = decoded.request
            else {
                return Err(format!("a deep cell must be a bad request: {decoded:?}").into());
            };
            assert_eq!(id, "50");
            assert_eq!(method, "signData");
            assert_eq!(error_code, TonConnectRpcErrorCode::BadRequest);
            Ok(())
        })
    }

    #[test]
    fn deep_send_transaction_payloads_are_refused_on_a_512_kib_stack() -> TestResult {
        on_worker_stack(|| {
            let dapp = SessionCrypto::generate()?;
            let session = session(&dapp);
            let (info, _, _) = account();
            let deep = deep_chain_cell(5000);
            let deep_payload = json!({
                "address": DESTINATION,
                "amount": "1000000",
                "payload": deep,
            });
            let deep_state_init = json!({
                "address": DESTINATION,
                "amount": "1000000",
                "payload": DEMO_PAYLOAD,
                "stateInit": deep,
            });
            let mut sign = request_with_messages("53", &info.address, vec![deep_payload.clone()]);
            sign.method = "signMessage".to_owned();
            let requests = [
                request_with_messages("51", &info.address, vec![deep_payload]),
                request_with_messages("52", &info.address, vec![deep_state_init]),
                sign,
            ];
            for request in requests {
                let decoded = session
                    .decrypt_request(encrypt_for(&dapp, &session, &request)?, 1_800_000_000)?;
                let TonConnectIncomingRequest::Unsupported {
                    id,
                    method,
                    error_code,
                    ..
                } = decoded.request
                else {
                    return Err(format!("a deep payload must be a bad request: {decoded:?}").into());
                };
                assert_eq!(id, request.id);
                assert_eq!(method, request.method);
                assert_eq!(
                    error_code,
                    TonConnectRpcErrorCode::BadRequest,
                    "{}",
                    request.id
                );
            }
            Ok(())
        })
    }

    #[test]
    fn cells_below_the_depth_bound_still_pass_on_a_512_kib_stack() -> TestResult {
        on_worker_stack(|| {
            let dapp = SessionCrypto::generate()?;
            let session = session(&dapp);
            let (info, _, _) = account();
            // 1 025 cells are 1 024 levels deep: the deepest cell the TVM accepts.
            for levels in [1000, 1025] {
                let cell = deep_chain_cell(levels);
                let payload = json!({"type": "cell", "schema": SIGN_DATA_SCHEMA, "cell": cell});
                let request =
                    decode_sign_data(&dapp, &session, &sign_data_app_request("60", &payload))?;
                assert_eq!(
                    request.payload,
                    TonConnectSignDataPayload::Cell {
                        schema: SIGN_DATA_SCHEMA.to_owned(),
                        cell,
                    }
                );
            }

            let chain = deep_chain_cell(1000);
            let message = json!({"address": DESTINATION, "amount": "1000000", "payload": chain});
            let request = request_with_messages("61", &info.address, vec![message]);
            let request = decode_send(&dapp, &session, &request)?;
            let [message] = request.intent.messages.as_slice() else {
                return Err("the request must decode to one message".into());
            };
            let SendMessageBody::RawPayload { boc } = &message.body else {
                return Err("a deep payload must stay a raw payload".into());
            };
            assert_eq!(boc.to_base64(), chain);
            Ok(())
        })
    }

    /// Decrypts `request`, which carries a malformed exotic cell, and checks it
    /// is a bad request. Should it decode instead, runs what the engine does
    /// next with it (`next_step` hashes the cell) and fails.
    fn assert_exotic_bad_request(
        dapp: &SessionCrypto,
        session: &TonConnectDerivedSession,
        request: &AppRequest,
        next_step: impl FnOnce(TonConnectIncomingRequest) -> TestResult,
    ) -> TestResult {
        let decoded =
            session.decrypt_request(encrypt_for(dapp, session, request)?, 1_800_000_000)?;
        match decoded.request {
            TonConnectIncomingRequest::Unsupported {
                id,
                method,
                error_code,
                ..
            } => {
                assert_eq!(id, request.id);
                assert_eq!(method, request.method);
                assert_eq!(error_code, TonConnectRpcErrorCode::BadRequest);
                Ok(())
            }
            accepted @ (TonConnectIncomingRequest::SendTransaction { .. }
            | TonConnectIncomingRequest::SignData { .. }) => {
                next_step(accepted)?;
                Err("a malformed exotic cell must be a bad request".into())
            }
            unexpected @ (TonConnectIncomingRequest::SignMessage { .. }
            | TonConnectIncomingRequest::Disconnect { .. }) => {
                Err(format!("unexpected request: {unexpected:?}").into())
            }
        }
    }

    /// Signs an accepted `sendTransaction` as the engine's send and preview do.
    fn prepare_accepted_send(accepted: TonConnectIncomingRequest) -> TestResult {
        if let TonConnectIncomingRequest::SendTransaction { request, .. } = accepted {
            let _ = signed_internal_messages(&request)?;
        }
        Ok(())
    }

    #[test]
    fn malformed_exotic_send_transaction_payloads_are_bad_requests() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();
        let payload = STANDARD.encode(test_boc::TRUNCATED_PRUNED_BRANCH);
        let message = json!({"address": DESTINATION, "amount": "1000000", "payload": payload});
        let request = request_with_messages("70", &info.address, vec![message]);
        assert_exotic_bad_request(&dapp, &session, &request, prepare_accepted_send)
    }

    #[test]
    fn malformed_exotic_state_inits_are_bad_requests() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();
        // The cell under the `StateInit` root is what the send hashes; a bare
        // root is re-serialized from its fields, so it is refused but never hashed.
        for (id, state_init) in [
            ("71", &test_boc::STATE_INIT_WITH_TRUNCATED_PRUNED_CODE[..]),
            ("72", &test_boc::TRUNCATED_PRUNED_BRANCH[..]),
        ] {
            let message = json!({
                "address": DESTINATION,
                "amount": "1000000",
                "payload": DEMO_PAYLOAD,
                "stateInit": STANDARD.encode(state_init),
            });
            let request = request_with_messages(id, &info.address, vec![message]);
            assert_exotic_bad_request(&dapp, &session, &request, prepare_accepted_send)?;
        }
        Ok(())
    }

    #[test]
    fn malformed_exotic_sign_data_cells_are_bad_requests() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let cell = STANDARD.encode(test_boc::TRUNCATED_PRUNED_BRANCH);
        let payload = json!({"type": "cell", "schema": SIGN_DATA_SCHEMA, "cell": cell});
        let request = sign_data_app_request("73", &payload);
        assert_exotic_bad_request(&dapp, &session, &request, |accepted| {
            if let TonConnectIncomingRequest::SignData { request, .. } = accepted {
                // The digest `sign_ton_connect_data` signs at approval.
                let (_, address, network) = account();
                let _ = sign_data_digest(
                    &request,
                    &network,
                    &address,
                    SIGN_DATA_DOMAIN,
                    SIGN_DATA_TIMESTAMP,
                )?;
            }
            Ok(())
        })
    }

    #[test]
    fn deep_payloads_that_fit_a_message_still_prepare() -> TestResult {
        // The claim is that such payloads still work, not a stack bound.
        let job = || -> TestResult {
            let dapp = SessionCrypto::generate()?;
            let session = session(&dapp);
            let (info, _, _) = account();
            let send = |levels: usize| -> Result<_, Box<dyn std::error::Error>> {
                let payload = deep_chain_cell(levels);
                let message =
                    json!({"address": DESTINATION, "amount": "1000000", "payload": payload});
                let request = request_with_messages("62", &info.address, vec![message]);
                Ok((payload, decode_send(&dapp, &session, &request)?))
            };

            let (payload, request) = send(1022)?;
            let internal = signed_internal_messages(&request)?;
            let [signed] = internal.as_slice() else {
                return Err("the transfer must carry one internal message".into());
            };
            assert_eq!(
                signed.body.value.hash()?,
                TonCell::from_boc(STANDARD.decode(payload)?)?.hash()?
            );

            // Three more levels wrap the payload in the signed external,
            // which would be 1,025 levels deep: TON refuses cells deeper
            // than 1024 (`CellTraits::max_depth`), so preparation fails.
            let (_, request) = send(1023)?;
            let error = signed_internal_messages(&request)
                .expect_err("an external deeper than 1024 levels must not prepare");
            assert!(format!("{error:?}").contains("InvalidBoc"), "{error:?}");
            Ok(())
        };
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(move || job().map_err(|error| error.to_string()))
            .expect("spawn")
            .join()
            .expect("the job must not panic")?;
        Ok(())
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_sign_data_params_never_panic(
            param in proptest::prelude::any::<String>(),
            shaped in "[{}\":, a-z0-9+/=_-]{0,80}",
            kind in "(text|binary|cell|unknown)",
            value in proptest::prelude::any::<String>(),
            network in proptest::option::of(proptest::prelude::any::<String>()),
            from in proptest::option::of(proptest::prelude::any::<String>()),
            variant in 0_u8..3,
            signature in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..70),
            public_key in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..40),
        ) {
            let dapp = SessionCrypto::generate().expect("entropy");
            let session = session(&dapp);
            let field = match kind.as_str() {
                "binary" => "bytes",
                "cell" => "cell",
                _ => "text",
            };
            let structured = json!({"type": kind, field: value, "schema": value}).to_string();
            for param in [param, shaped, structured] {
                let request = AppRequest {
                    method: "signData".to_owned(),
                    params: vec![param],
                    id: "1".to_owned(),
                };
                let body = encrypt_for(&dapp, &session, &request).expect("request encrypts");
                let _ = session.decrypt_request(body, 1_800_000_000);
            }

            let payload = match variant {
                0 => TonConnectSignDataPayload::Text { text: value.clone() },
                1 => TonConnectSignDataPayload::Binary { bytes: value.clone() },
                _ => TonConnectSignDataPayload::Cell { schema: value.clone(), cell: value.clone() },
            };
            let _ = session.encrypt_sign_data_success(
                value.clone(),
                TonConnectSignedData {
                    request: TonConnectSignDataRequest { payload, network, from },
                    domain: value,
                    timestamp: u64::from(variant),
                    signature,
                    public_key,
                },
            );
        }
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_bodies_and_challenges_never_panic(
            bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300)
        ) {
            let dapp = SessionCrypto::generate().expect("entropy");
            let session = session(&dapp);
            let _ = session.decrypt_request(bytes.clone(), 1_800_000_000);
            let _ = session.open_challenge(bytes);
        }
    }

    #[test]
    fn account_from_another_wallet_is_refused() -> TestResult {
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        let (info, _, _) = account();

        let mut other_address = info.clone();
        other_address.address = OTHER_ADDRESS.to_owned();
        assert!(matches!(
            session.encrypt_connect_event(1, other_address, None, device()),
            Err(TonConnectSessionError::Failed { .. })
        ));

        let mut other_network = info.clone();
        other_network.network = "-239".to_owned();
        assert!(matches!(
            session.encrypt_connect_event(1, other_network, None, device()),
            Err(TonConnectSessionError::Failed { .. })
        ));

        // The anchor key of a rotated wallet cannot verify what the session signs.
        let mut anchor_key = info;
        anchor_key.public_key = vec![0_u8; 32];
        assert!(matches!(
            session.encrypt_connect_event(1, anchor_key, None, device()),
            Err(TonConnectSessionError::Failed { .. })
        ));
        Ok(())
    }

    #[test]
    fn session_exposes_no_secret_material() -> TestResult {
        // The object derives no `Debug`; `SessionCrypto` redacts its secret key.
        let dapp = SessionCrypto::generate()?;
        let session = session(&dapp);
        assert_eq!(session.signing_public_key(), signing_public_key().to_vec());
        assert_eq!(session.public_key_hex().len(), 64);
        // `W` depends on the derived secret only, never on the peer.
        assert_eq!(session.public_key_hex(), EXPECTED_W);
        Ok(())
    }
}
