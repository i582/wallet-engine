use std::{fmt, str::FromStr};

use base64::{Engine as _, engine::general_purpose};
use crc::{CRC_32_ISO_HDLC, Crc};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest, Sha256};
use thiserror::Error;
use ton_core::{
    cell::{BoC, TonCell, TonHash},
    traits::tlb::TLB,
    types::tlb_core::MsgAddressIntStd,
};

use crate::NetworkId;

const TON_PROOF_ITEM_PREFIX: &[u8] = b"ton-proof-item-v2/";
const TON_CONNECT_PREFIX: &[u8] = b"ton-connect";
const SIGN_DATA_PREFIX: &[u8] = b"ton-connect/sign-data/";
const SIGN_DATA_CELL_MAGIC: u32 = 0x7556_9022;
const SIGNATURE_DOMAIN_L2_TAG: i32 = 0x71b3_4ee1;
const DNS_SNAKE_CELL_BYTES: usize = 127;

/// Failure to construct or verify a TON Connect signing payload.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SigningError {
    /// A raw account address did not use the canonical `workchain:hash` form.
    #[error("account address must use raw workchain:64-hex form")]
    InvalidAddress,
    /// A public key was not exactly 32 bytes of hexadecimal text.
    #[error("Ed25519 public key must be exactly 64 hexadecimal characters")]
    InvalidPublicKey,
    /// A signature was not exactly 64 bytes of canonical standard base64.
    #[error("Ed25519 signature must be exactly 64 bytes of canonical standard base64")]
    InvalidSignature,
    /// A byte length cannot be represented by the protocol's 32-bit field.
    #[error("TON Connect signing field exceeds the 32-bit wire length")]
    LengthOverflow,
    /// A network ID cannot be represented by TON's signed 32-bit global ID.
    #[error("network global ID does not fit a signed 32-bit integer")]
    InvalidNetworkId,
    /// A cell-signing domain cannot be encoded as non-empty TEP-81 labels.
    #[error("cell-signing domain contains an empty DNS label")]
    InvalidDomain,
    /// A cell payload was not a valid single-root `BoC` or its signed cell could not be built.
    #[error("invalid TON Connect signData cell payload")]
    InvalidCell,
    /// A validated signData wire value could not be decoded as base64 bytes.
    #[error("invalid TON Connect signData base64 payload")]
    InvalidBase64Payload,
    /// The workchain cannot be represented by `MsgAddressIntStd` used by TON Connect signData.
    #[error("signData cell address workchain does not fit int8")]
    UnsupportedCellWorkchain,
    /// The Ed25519 public key bytes do not encode a valid verification key.
    #[error("invalid Ed25519 verification key")]
    InvalidVerificationKey,
}

/// Canonical raw TON account address used by TON Connect signed payloads.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RawAccountAddress {
    workchain: i32,
    hash: [u8; 32],
}

impl RawAccountAddress {
    /// Creates an address from its exact binary components.
    #[must_use]
    pub const fn new(workchain: i32, hash: [u8; 32]) -> Self {
        Self { workchain, hash }
    }

    /// Returns the signed workchain identifier.
    #[must_use]
    pub const fn workchain(&self) -> i32 {
        self.workchain
    }

    /// Returns the 256-bit account hash.
    #[must_use]
    pub const fn hash(&self) -> &[u8; 32] {
        &self.hash
    }
}

impl fmt::Debug for RawAccountAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for RawAccountAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.workchain, hex::encode(self.hash))
    }
}

impl FromStr for RawAccountAddress {
    type Err = SigningError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((workchain, hash)) = value.split_once(':') else {
            return Err(SigningError::InvalidAddress);
        };
        if hash.len() != 64 || !hash.as_bytes().iter().all(u8::is_ascii_hexdigit) {
            return Err(SigningError::InvalidAddress);
        }

        let parsed_workchain = workchain
            .parse::<i32>()
            .map_err(|_| SigningError::InvalidAddress)?;
        if parsed_workchain.to_string() != workchain {
            return Err(SigningError::InvalidAddress);
        }

        let mut parsed_hash = [0_u8; 32];
        hex::decode_to_slice(hash, &mut parsed_hash).map_err(|_| SigningError::InvalidAddress)?;
        Ok(Self::new(parsed_workchain, parsed_hash))
    }
}

impl Serialize for RawAccountAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RawAccountAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(de::Error::custom)
    }
}

/// A 32-byte Ed25519 public key encoded as lowercase hex on the wire.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct Ed25519PublicKey([u8; 32]);

impl Ed25519PublicKey {
    /// Creates a public key from its exact bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the public-key bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for Ed25519PublicKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for Ed25519PublicKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl FromStr for Ed25519PublicKey {
    type Err = SigningError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64 || !value.as_bytes().iter().all(u8::is_ascii_hexdigit) {
            return Err(SigningError::InvalidPublicKey);
        }
        let mut bytes = [0_u8; 32];
        hex::decode_to_slice(value, &mut bytes).map_err(|_| SigningError::InvalidPublicKey)?;
        Ok(Self(bytes))
    }
}

impl Serialize for Ed25519PublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Ed25519PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(de::Error::custom)
    }
}

/// A 64-byte Ed25519 signature encoded as canonical standard base64.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Ed25519Signature([u8; 64]);

impl Ed25519Signature {
    /// Creates a signature from its exact bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 64]) -> Self {
        Self(bytes)
    }

    /// Returns the signature bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 64] {
        &self.0
    }
}

impl fmt::Debug for Ed25519Signature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Ed25519Signature(<redacted>)")
    }
}

impl fmt::Display for Ed25519Signature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&general_purpose::STANDARD.encode(self.0))
    }
}

impl FromStr for Ed25519Signature {
    type Err = SigningError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let decoded = general_purpose::STANDARD
            .decode(value)
            .map_err(|_| SigningError::InvalidSignature)?;
        let bytes =
            <[u8; 64]>::try_from(decoded.as_slice()).map_err(|_| SigningError::InvalidSignature)?;
        if general_purpose::STANDARD.encode(bytes) != value {
            return Err(SigningError::InvalidSignature);
        }
        Ok(Self(bytes))
    }
}

impl Serialize for Ed25519Signature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Ed25519Signature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(de::Error::custom)
    }
}

/// TON signature domain applied by networks outside mainnet and testnet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignatureDomain {
    /// Legacy empty signature domain used by mainnet and testnet.
    Empty,
    /// L2 domain that binds the signature to a custom network global ID.
    L2 {
        /// Signed 32-bit network global ID.
        global_id: i32,
    },
}

impl SignatureDomain {
    /// Selects the protocol signature domain for a network global ID.
    pub fn for_network(network: &NetworkId) -> Result<Self, SigningError> {
        let global_id = network
            .as_str()
            .parse::<i32>()
            .map_err(|_| SigningError::InvalidNetworkId)?;
        if matches!(global_id, -239 | -3) {
            Ok(Self::Empty)
        } else {
            Ok(Self::L2 { global_id })
        }
    }

    fn apply(self, payload: &[u8]) -> Vec<u8> {
        match self {
            Self::Empty => payload.to_vec(),
            Self::L2 { global_id } => {
                let mut encoded_domain = Vec::with_capacity(8);
                encoded_domain.extend_from_slice(&SIGNATURE_DOMAIN_L2_TAG.to_le_bytes());
                encoded_domain.extend_from_slice(&global_id.to_le_bytes());
                let domain_hash = Sha256::digest(encoded_domain);

                let mut signed =
                    Vec::with_capacity(domain_hash.len().saturating_add(payload.len()));
                signed.extend_from_slice(&domain_hash);
                signed.extend_from_slice(payload);
                signed
            }
        }
    }
}

/// Borrowed payload used to build a TON Connect `signData` signing digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignDataSigningPayload<'a> {
    /// UTF-8 text with the protocol `txt` discriminator.
    Text(&'a str),
    /// Opaque bytes with the protocol `bin` discriminator.
    Binary(&'a [u8]),
    /// A one-root cell `BoC` interpreted under the exact TL-B schema string.
    Cell {
        /// Exact UTF-8 TL-B schema used for CRC-32.
        schema: &'a str,
        /// Serialized one-root payload cell.
        boc: &'a [u8],
    },
}

/// Builds the exact preimage inside the two SHA-256 rounds of `ton_proof`.
pub fn ton_proof_message(
    address: &RawAccountAddress,
    domain: &str,
    timestamp: u64,
    payload: &str,
) -> Result<Vec<u8>, SigningError> {
    let domain_length = u32::try_from(domain.len()).map_err(|_| SigningError::LengthOverflow)?;
    let mut message = Vec::new();
    message.extend_from_slice(TON_PROOF_ITEM_PREFIX);
    message.extend_from_slice(&address.workchain.to_be_bytes());
    message.extend_from_slice(&address.hash);
    message.extend_from_slice(&domain_length.to_le_bytes());
    message.extend_from_slice(domain.as_bytes());
    message.extend_from_slice(&timestamp.to_le_bytes());
    message.extend_from_slice(payload.as_bytes());
    Ok(message)
}

/// Computes the 32-byte digest that a wallet signs for `ton_proof`.
pub fn ton_proof_signing_hash(
    address: &RawAccountAddress,
    domain: &str,
    timestamp: u64,
    payload: &str,
) -> Result<[u8; 32], SigningError> {
    let message_hash = Sha256::digest(ton_proof_message(address, domain, timestamp, payload)?);
    let mut wrapped = Vec::with_capacity(
        2_usize
            .saturating_add(TON_CONNECT_PREFIX.len())
            .saturating_add(32),
    );
    wrapped.extend_from_slice(&[0xff, 0xff]);
    wrapped.extend_from_slice(TON_CONNECT_PREFIX);
    wrapped.extend_from_slice(&message_hash);
    Ok(Sha256::digest(wrapped).into())
}

/// Computes the 32-byte digest that a wallet signs for `signData`.
pub fn sign_data_signing_hash(
    address: &RawAccountAddress,
    domain: &str,
    timestamp: u64,
    payload: SignDataSigningPayload<'_>,
) -> Result<[u8; 32], SigningError> {
    match payload {
        SignDataSigningPayload::Text(text) => {
            text_or_binary_signing_hash(address, domain, timestamp, *b"txt", text.as_bytes())
        }
        SignDataSigningPayload::Binary(bytes) => {
            text_or_binary_signing_hash(address, domain, timestamp, *b"bin", bytes)
        }
        SignDataSigningPayload::Cell { schema, boc } => {
            cell_signing_hash(address, domain, timestamp, schema, boc)
        }
    }
}

/// Verifies an Ed25519 signature over an already constructed TON Connect digest.
///
/// `ton_proof` on custom networks must pass the network's L2 domain. The
/// `signData` protocol signs its digest directly and therefore uses `Empty`.
pub fn verify_signature(
    hash: &[u8; 32],
    signature: &Ed25519Signature,
    public_key: &Ed25519PublicKey,
    domain: SignatureDomain,
) -> Result<bool, SigningError> {
    let verifying_key = VerifyingKey::from_bytes(public_key.as_bytes())
        .map_err(|_| SigningError::InvalidVerificationKey)?;
    let signature = Signature::from_bytes(signature.as_bytes());
    Ok(verifying_key
        .verify_strict(&domain.apply(hash), &signature)
        .is_ok())
}

fn text_or_binary_signing_hash(
    address: &RawAccountAddress,
    domain: &str,
    timestamp: u64,
    discriminator: [u8; 3],
    payload: &[u8],
) -> Result<[u8; 32], SigningError> {
    let domain_length = u32::try_from(domain.len()).map_err(|_| SigningError::LengthOverflow)?;
    let payload_length = u32::try_from(payload.len()).map_err(|_| SigningError::LengthOverflow)?;

    let mut message = Vec::new();
    message.extend_from_slice(&[0xff, 0xff]);
    message.extend_from_slice(SIGN_DATA_PREFIX);
    message.extend_from_slice(&address.workchain.to_be_bytes());
    message.extend_from_slice(&address.hash);
    message.extend_from_slice(&domain_length.to_be_bytes());
    message.extend_from_slice(domain.as_bytes());
    message.extend_from_slice(&timestamp.to_be_bytes());
    message.extend_from_slice(&discriminator);
    message.extend_from_slice(&payload_length.to_be_bytes());
    message.extend_from_slice(payload);
    Ok(Sha256::digest(message).into())
}

fn cell_signing_hash(
    address: &RawAccountAddress,
    domain: &str,
    timestamp: u64,
    schema: &str,
    boc: &[u8],
) -> Result<[u8; 32], SigningError> {
    let payload_cell = BoC::from_bytes(boc.to_vec())
        .and_then(BoC::single_root)
        .map_err(|_| SigningError::InvalidCell)?;
    let domain_cell = snake_cell(&tep81_domain(domain)?)?;
    let workchain =
        i8::try_from(address.workchain).map_err(|_| SigningError::UnsupportedCellWorkchain)?;
    let address_hash = TonHash::from_slice(&address.hash).map_err(|_| SigningError::InvalidCell)?;
    let address = MsgAddressIntStd {
        anycast: None,
        workchain,
        address: address_hash,
    };
    let schema_hash = Crc::<u32>::new(&CRC_32_ISO_HDLC).checksum(schema.as_bytes());

    let mut builder = TonCell::builder();
    builder
        .write_num(&SIGN_DATA_CELL_MAGIC, 32)
        .and_then(|()| builder.write_num(&schema_hash, 32))
        .and_then(|()| builder.write_num(&timestamp, 64))
        .and_then(|()| address.write(&mut builder))
        .and_then(|()| builder.write_ref(domain_cell))
        .and_then(|()| builder.write_ref(payload_cell))
        .map_err(|_| SigningError::InvalidCell)?;
    let message = builder.build().map_err(|_| SigningError::InvalidCell)?;
    let hash = message.hash().map_err(|_| SigningError::InvalidCell)?;
    <[u8; 32]>::try_from(hash.as_slice()).map_err(|_| SigningError::InvalidCell)
}

fn tep81_domain(domain: &str) -> Result<Vec<u8>, SigningError> {
    if domain.is_empty() {
        return Err(SigningError::InvalidDomain);
    }

    let mut encoded = Vec::new();
    for label in domain.split('.').rev() {
        if label.is_empty() {
            return Err(SigningError::InvalidDomain);
        }
        encoded.extend_from_slice(label.as_bytes());
        encoded.push(0);
    }
    Ok(encoded)
}

fn snake_cell(bytes: &[u8]) -> Result<TonCell, SigningError> {
    let mut next = None;
    for chunk in bytes.rchunks(DNS_SNAKE_CELL_BYTES) {
        let bit_length = chunk
            .len()
            .checked_mul(8)
            .ok_or(SigningError::LengthOverflow)?;
        let mut builder = TonCell::builder();
        builder
            .write_bits(chunk, bit_length)
            .map_err(|_| SigningError::InvalidCell)?;
        if let Some(child) = next.take() {
            builder
                .write_ref(child)
                .map_err(|_| SigningError::InvalidCell)?;
        }
        next = Some(builder.build().map_err(|_| SigningError::InvalidCell)?);
    }
    next.ok_or(SigningError::InvalidDomain)
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer as _, SigningKey};

    use super::*;

    fn address() -> RawAccountAddress {
        RawAccountAddress::new(0, [0x11; 32])
    }

    fn decode_hash(value: &str) -> Result<[u8; 32], SigningError> {
        let decoded = hex::decode(value).map_err(|_| SigningError::InvalidCell)?;
        <[u8; 32]>::try_from(decoded.as_slice()).map_err(|_| SigningError::InvalidCell)
    }

    #[test]
    fn raw_address_requires_canonical_wire_form() {
        let canonical = format!("-1:{}", "ab".repeat(32));
        assert_eq!(
            RawAccountAddress::from_str(&canonical).map(|value| value.to_string()),
            Ok(canonical)
        );
        assert!(RawAccountAddress::from_str(&format!("00:{}", "ab".repeat(32))).is_err());
        assert_eq!(
            RawAccountAddress::from_str(&format!("0:{}", "AB".repeat(32)))
                .map(|value| value.to_string()),
            Ok(format!("0:{}", "ab".repeat(32)))
        );
    }

    #[test]
    fn signature_wire_type_rejects_wrong_alphabet_length_and_padding() {
        let signature = Ed25519Signature::from_bytes([0xfb; 64]);
        let encoded = signature.to_string();
        assert_eq!(Ed25519Signature::from_str(&encoded), Ok(signature));
        assert!(Ed25519Signature::from_str(encoded.trim_end_matches('=')).is_err());
        assert!(Ed25519Signature::from_str(&encoded.replace('+', "-")).is_err());
        assert!(Ed25519Signature::from_str("AA==").is_err());
    }

    #[test]
    fn proof_signature_rejects_any_bound_field_change() -> Result<(), SigningError> {
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let public_key = Ed25519PublicKey::from_bytes(signing_key.verifying_key().to_bytes());
        let hash = ton_proof_signing_hash(&address(), "example.com", 1_700_000_000, "nonce")?;
        let signature = Ed25519Signature::from_bytes(signing_key.sign(&hash).to_bytes());

        assert!(verify_signature(
            &hash,
            &signature,
            &public_key,
            SignatureDomain::Empty
        )?);
        let tampered = ton_proof_signing_hash(&address(), "example.com", 1_700_000_001, "nonce")?;
        assert!(!verify_signature(
            &tampered,
            &signature,
            &public_key,
            SignatureDomain::Empty
        )?);
        Ok(())
    }

    #[test]
    fn custom_network_signature_domain_changes_signed_bytes() -> Result<(), SigningError> {
        let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let public_key = Ed25519PublicKey::from_bytes(signing_key.verifying_key().to_bytes());
        let hash = [0x44_u8; 32];
        let domain = SignatureDomain::L2 { global_id: 42 };
        let signature =
            Ed25519Signature::from_bytes(signing_key.sign(&domain.apply(&hash)).to_bytes());

        assert!(verify_signature(&hash, &signature, &public_key, domain)?);
        assert!(!verify_signature(
            &hash,
            &signature,
            &public_key,
            SignatureDomain::Empty
        )?);
        Ok(())
    }

    #[test]
    fn hashes_match_typescript_and_ton_core_vectors() -> Result<(), SigningError> {
        // Generated independently with Node.js crypto, @ton/core 0.63.1,
        // and crc-32 1.2.2 from the reference WalletKit workspace.
        let proof_message = ton_proof_message(&address(), "example.com", 1_700_000_000, "nonce")?;
        assert_eq!(
            hex::encode(proof_message),
            "746f6e2d70726f6f662d6974656d2d76322f0000000011111111111111111111111111111111111111111111111111111111111111110b0000006578616d706c652e636f6d00f15365000000006e6f6e6365"
        );
        assert_eq!(
            ton_proof_signing_hash(&address(), "example.com", 1_700_000_000, "nonce")?,
            decode_hash("c65bdd5675baff214a9c3fac25c6e48007bcefce280c94827d4e2562a5638441")?
        );
        assert_eq!(
            sign_data_signing_hash(
                &address(),
                "example.com",
                1_700_000_000,
                SignDataSigningPayload::Text("Hello 🌍")
            )?,
            decode_hash("5ec8d5210d541e5e6995ef4a003557a04f3ed070fc1b6e78631895bb23002e32")?
        );
        assert_eq!(
            sign_data_signing_hash(
                &address(),
                "example.com",
                1_700_000_000,
                SignDataSigningPayload::Binary(&[0, 1, 2, 253, 254, 255])
            )?,
            decode_hash("e009a87c047c8dedb7e193819dc2b7b21898f9b56968bcf3a569f2a05370b509")?
        );

        let boc = general_purpose::STANDARD
            .decode("te6cckEBAQEABgAACN6tvu+qPBS2")
            .map_err(|_| SigningError::InvalidCell)?;
        assert_eq!(
            sign_data_signing_hash(
                &address(),
                "example.com",
                1_700_000_000,
                SignDataSigningPayload::Cell {
                    schema: "value#12345678 amount:uint64 = Value;",
                    boc: &boc,
                }
            )?,
            decode_hash("88d4dd981afd3253c89c119c3399d097d83894fdf2558f04846cb25533df18c3")?
        );
        Ok(())
    }
}
