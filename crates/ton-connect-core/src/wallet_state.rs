//! Address and public-key binding for TON Connect wallet state initialization.

use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;
use ton_core::cell::{CellType, TonCell};

use crate::{Ed25519PublicKey, RawAccountAddress, SigningError};

/// Canonical standard-base64 encoding of a valid, single-root TON `StateInit` `BoC`.
///
/// TON Connect requires standard base64 here, not base64url. Validation happens
/// at the wire boundary so later account checks cannot accidentally operate on
/// arbitrary bytes or a multi-root `BoC`.
#[derive(Clone, Eq, PartialEq)]
pub struct WalletStateInit {
    encoded: String,
    boc: Vec<u8>,
}

impl WalletStateInit {
    /// Creates a validated value from serialized `BoC` bytes.
    pub fn from_boc(boc: Vec<u8>) -> Result<Self, WalletStateError> {
        let root = parse_root(&boc)?;
        let _ = parse_state_init(&root)?;
        Ok(Self {
            encoded: STANDARD.encode(&boc),
            boc,
        })
    }

    /// Returns the canonical standard-base64 wire value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.encoded
    }

    /// Returns the validated serialized `BoC` bytes.
    #[must_use]
    pub fn as_boc(&self) -> &[u8] {
        &self.boc
    }

    /// Derives the raw account address for the supplied workchain.
    pub fn derive_address(&self, workchain: i32) -> Result<RawAccountAddress, WalletStateError> {
        let root = parse_root(&self.boc)?;
        let hash = root.hash().map_err(|_| WalletStateError::InvalidBoc)?;
        let hash =
            <[u8; 32]>::try_from(hash.as_slice()).map_err(|_| WalletStateError::InvalidBoc)?;
        Ok(RawAccountAddress::new(workchain, hash))
    }

    /// Extracts a public key from a recognized standard wallet contract.
    ///
    /// `Ok(None)` means that the state is valid but its code is not one of the
    /// standard wallet versions. A verifier can then use the protocol-defined
    /// on-chain `get_public_key` fallback without conflating it with malformed
    /// untrusted input.
    pub fn extract_standard_public_key(
        &self,
    ) -> Result<Option<StandardWalletState>, WalletStateError> {
        let root = parse_root(&self.boc)?;
        let state = parse_state_init(&root)?;
        let code = state.code.ok_or(WalletStateError::MissingCode)?;
        let data = state.data.ok_or(WalletStateError::MissingData)?;
        let version = version_from_code(&code)?;
        version
            .map(|version| {
                extract_public_key(&data, version).map(|public_key| StandardWalletState {
                    version,
                    public_key,
                })
            })
            .transpose()
    }

    /// Verifies address and advertised-key binding for a standard wallet.
    pub fn verify_standard_wallet(
        &self,
        address: &RawAccountAddress,
        advertised_public_key: &Ed25519PublicKey,
    ) -> Result<StandardWalletState, WalletStateError> {
        if self.derive_address(address.workchain())? != *address {
            return Err(WalletStateError::AddressMismatch);
        }
        let state = self
            .extract_standard_public_key()?
            .ok_or(WalletStateError::UnsupportedWalletCode)?;
        if state.public_key != *advertised_public_key {
            return Err(WalletStateError::PublicKeyMismatch);
        }
        Ok(state)
    }
}

impl fmt::Debug for WalletStateInit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WalletStateInit")
            .field("boc_bytes", &self.boc.len())
            .finish_non_exhaustive()
    }
}

impl TryFrom<String> for WalletStateInit {
    type Error = WalletStateError;

    fn try_from(encoded: String) -> Result<Self, Self::Error> {
        let boc = STANDARD
            .decode(&encoded)
            .map_err(|_| WalletStateError::InvalidBase64)?;
        if STANDARD.encode(&boc) != encoded {
            return Err(WalletStateError::InvalidBase64);
        }
        let root = parse_root(&boc)?;
        let _ = parse_state_init(&root)?;
        Ok(Self { encoded, boc })
    }
}

impl TryFrom<&str> for WalletStateInit {
    type Error = WalletStateError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl From<WalletStateInit> for String {
    fn from(value: WalletStateInit) -> Self {
        value.encoded
    }
}

impl Serialize for WalletStateInit {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WalletStateInit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        Self::try_from(encoded).map_err(de::Error::custom)
    }
}

/// Standard wallet contract recognized by local `StateInit` inspection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StandardWalletVersion {
    /// Wallet V1 revision 1.
    V1R1,
    /// Wallet V1 revision 2.
    V1R2,
    /// Wallet V1 revision 3.
    V1R3,
    /// Wallet V2 revision 1.
    V2R1,
    /// Wallet V2 revision 2.
    V2R2,
    /// Wallet V3 revision 1.
    V3R1,
    /// Wallet V3 revision 2.
    V3R2,
    /// Wallet V4 revision 1.
    V4R1,
    /// Wallet V4 revision 2.
    V4R2,
    /// Wallet V5 revision 1.
    V5R1,
    /// The wallet-engine contract with one-time public-key rotation.
    Wallet,
}

/// Public state extracted from a recognized standard wallet `StateInit`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardWalletState {
    version: StandardWalletVersion,
    public_key: Ed25519PublicKey,
}

impl StandardWalletState {
    /// Returns the recognized contract version.
    #[must_use]
    pub const fn version(&self) -> StandardWalletVersion {
        self.version
    }

    /// Returns the public key parsed from the address-bound data cell.
    #[must_use]
    pub const fn public_key(&self) -> &Ed25519PublicKey {
        &self.public_key
    }
}

/// Failure to parse or authenticate a TON Connect wallet `StateInit`.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum WalletStateError {
    /// The value is not canonical padded standard base64.
    #[error("walletStateInit must be canonical padded standard base64")]
    InvalidBase64,
    /// The decoded value is not a valid single-root `BoC`.
    #[error("walletStateInit must be a valid single-root BoC")]
    InvalidBoc,
    /// The root cell does not encode a complete `StateInit` value.
    #[error("walletStateInit root has an invalid StateInit layout")]
    InvalidStateInit,
    /// The `StateInit` has no contract code.
    #[error("walletStateInit has no code cell")]
    MissingCode,
    /// The `StateInit` has no contract data.
    #[error("walletStateInit has no data cell")]
    MissingData,
    /// The contract code is not a locally recognized standard wallet.
    #[error("walletStateInit uses an unsupported wallet contract")]
    UnsupportedWalletCode,
    /// The recognized wallet data cell does not match its version layout.
    #[error("walletStateInit data does not match the recognized wallet version")]
    InvalidWalletData,
    /// The state hash does not equal the reported raw account hash.
    #[error("walletStateInit does not derive the reported account address")]
    AddressMismatch,
    /// The wallet-advertised key differs from the key stored in `StateInit`.
    #[error("wallet public key does not match walletStateInit")]
    PublicKeyMismatch,
}

/// Failure while verifying a signed response against its connected account.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum AccountVerificationError {
    /// The account's `StateInit` did not establish a trusted address and key.
    #[error(transparent)]
    WalletState(#[from] WalletStateError),
    /// The signed payload or Ed25519 key was invalid.
    #[error(transparent)]
    Signing(#[from] SigningError),
    /// A `signData` result was produced for a different connected address.
    #[error("signed response address differs from the connected account")]
    ResponseAddressMismatch,
}

struct ParsedStateInit {
    code: Option<TonCell>,
    data: Option<TonCell>,
}

fn parse_root(boc: &[u8]) -> Result<TonCell, WalletStateError> {
    crate::cell_boc::parse_single_root(boc).map_err(|_| WalletStateError::InvalidBoc)
}

fn parse_state_init(root: &TonCell) -> Result<ParsedStateInit, WalletStateError> {
    if root.cell_type() != CellType::Ordinary {
        return Err(WalletStateError::InvalidStateInit);
    }

    let mut parser = root.parser();
    if parser
        .read_bit()
        .map_err(|_| WalletStateError::InvalidStateInit)?
    {
        let _ = parser
            .read_bits(5)
            .map_err(|_| WalletStateError::InvalidStateInit)?;
    }
    if parser
        .read_bit()
        .map_err(|_| WalletStateError::InvalidStateInit)?
    {
        let _ = parser
            .read_bits(2)
            .map_err(|_| WalletStateError::InvalidStateInit)?;
    }

    let code = read_optional_ref(&mut parser)?;
    let data = read_optional_ref(&mut parser)?;

    // `library` is HashmapE: one presence bit and, when non-empty, one root
    // reference. Its contents do not affect where standard wallet data stores
    // the public key, but consuming it proves that the root is a full StateInit.
    if parser
        .read_bit()
        .map_err(|_| WalletStateError::InvalidStateInit)?
    {
        let _ = parser
            .read_next_ref()
            .map_err(|_| WalletStateError::InvalidStateInit)?;
    }
    parser
        .ensure_empty()
        .map_err(|_| WalletStateError::InvalidStateInit)?;
    Ok(ParsedStateInit { code, data })
}

fn read_optional_ref(
    parser: &mut ton_core::cell::CellParser<'_>,
) -> Result<Option<TonCell>, WalletStateError> {
    if parser
        .read_bit()
        .map_err(|_| WalletStateError::InvalidStateInit)?
    {
        parser
            .read_next_ref()
            .cloned()
            .map(Some)
            .map_err(|_| WalletStateError::InvalidStateInit)
    } else {
        Ok(None)
    }
}

fn version_from_code(code: &TonCell) -> Result<Option<StandardWalletVersion>, WalletStateError> {
    let hash = code
        .hash()
        .map_err(|_| WalletStateError::InvalidStateInit)?;
    let hash = hex::encode(hash.as_slice());
    Ok(match hash.as_str() {
        "a0cfc2c48aee16a271f2cfc0b7382d81756cecb1017d077faaab3bb602f6868c" => {
            Some(StandardWalletVersion::V1R1)
        }
        "d4902fcc9fad74698fa8e353220a68da0dcf72e32bcb2eb9ee04217c17d3062c" => {
            Some(StandardWalletVersion::V1R2)
        }
        "587cc789eff1c84f46ec3797e45fc809a14ff5ae24f1e0c7a6a99cc9dc9061ff" => {
            Some(StandardWalletVersion::V1R3)
        }
        "5c9a5e68c108e18721a07c42f9956bfb39ad77ec6d624b60c576ec88eee65329" => {
            Some(StandardWalletVersion::V2R1)
        }
        "fe9530d3243853083ef2ef0b4c2908c0abf6fa1c31ea243aacaa5bf8c7d753f1" => {
            Some(StandardWalletVersion::V2R2)
        }
        "b61041a58a7980b946e8fb9e198e3c904d24799ffa36574ea4251c41a566f581" => {
            Some(StandardWalletVersion::V3R1)
        }
        "84dafa449f98a6987789ba232358072bc0f76dc4524002a5d0918b9a75d2d599" => {
            Some(StandardWalletVersion::V3R2)
        }
        "64dd54805522c5be8a9db59cea0105ccf0d08786ca79beb8cb79e880a8d7322d" => {
            Some(StandardWalletVersion::V4R1)
        }
        "feb5ff6820e2ff0d9483e7e0d62c817d846789fb4ae580c878866d959dabd5c0" => {
            Some(StandardWalletVersion::V4R2)
        }
        "20834b7b72b112147e1b2fb457b84e74d1a30f04f737d4f62a668e9552d2b72f" => {
            Some(StandardWalletVersion::V5R1)
        }
        "3791f4bfbb8a2f697a5ce3598fdceeaaa0ead0badded8473a35fb69f76b021e5" => {
            Some(StandardWalletVersion::Wallet)
        }
        _ => None,
    })
}

fn extract_public_key(
    data: &TonCell,
    version: StandardWalletVersion,
) -> Result<Ed25519PublicKey, WalletStateError> {
    if data.cell_type() != CellType::Ordinary {
        return Err(WalletStateError::InvalidWalletData);
    }

    let mut parser = data.parser();
    let prefix_bits = match version {
        StandardWalletVersion::V1R1
        | StandardWalletVersion::V1R2
        | StandardWalletVersion::V1R3
        | StandardWalletVersion::V2R1
        | StandardWalletVersion::V2R2 => 32,
        StandardWalletVersion::V3R1
        | StandardWalletVersion::V3R2
        | StandardWalletVersion::V4R1
        | StandardWalletVersion::V4R2 => 64,
        StandardWalletVersion::V5R1 => 65,
        StandardWalletVersion::Wallet => 72,
    };
    let _ = parser
        .read_bits(prefix_bits)
        .map_err(|_| WalletStateError::InvalidWalletData)?;

    let mut public_key = [0_u8; 32];
    parser
        .read_bits_to(256, &mut public_key)
        .map_err(|_| WalletStateError::InvalidWalletData)?;

    if matches!(
        version,
        StandardWalletVersion::V4R1 | StandardWalletVersion::V4R2 | StandardWalletVersion::V5R1
    ) && parser
        .read_bit()
        .map_err(|_| WalletStateError::InvalidWalletData)?
    {
        let _ = parser
            .read_next_ref()
            .map_err(|_| WalletStateError::InvalidWalletData)?;
    }
    if matches!(version, StandardWalletVersion::Wallet)
        && parser
            .read_bit()
            .map_err(|_| WalletStateError::InvalidWalletData)?
    {
        return Err(WalletStateError::InvalidWalletData);
    }
    parser
        .ensure_empty()
        .map_err(|_| WalletStateError::InvalidWalletData)?;
    Ok(Ed25519PublicKey::from_bytes(public_key))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use base64::engine::general_purpose::URL_SAFE;
    use ed25519_dalek::{Signer as _, SigningKey};
    use ton_core::traits::tlb::TLB as _;

    use super::*;
    use crate::{
        Ed25519Signature, NetworkId, SignDataResult, SignDataSigningPayload, TonAddressItemReply,
        TonProof, TonProofDomain, Uint64String, sign_data_signing_hash, ton_proof_signing_hash,
    };

    const PUBLIC_KEY: [u8; 32] = [0x5a; 32];
    const V1R1_CODE: &str = include_str!("testdata/wallet_v1r1.code");
    const V1R2_CODE: &str = include_str!("testdata/wallet_v1r2.code");
    const V1R3_CODE: &str = include_str!("testdata/wallet_v1r3.code");
    const V2R1_CODE: &str = include_str!("testdata/wallet_v2r1.code");
    const V2R2_CODE: &str = include_str!("testdata/wallet_v2r2.code");
    const V3R1_CODE: &str = include_str!("testdata/wallet_v3r1.code");
    const V3R2_CODE: &str = include_str!("testdata/wallet_v3r2.code");
    const V4R1_CODE: &str = include_str!("testdata/wallet_v4r1.code");
    const V4R2_CODE: &str = include_str!("testdata/wallet_v4r2.code");
    const V5R1_CODE: &str = include_str!("testdata/wallet_v5.code");
    const WALLET_CODE: &str = include_str!("testdata/wallet_v5_experimental.code");

    type TestResult = Result<(), Box<dyn Error>>;

    fn standard_wallet(
        code_boc: &str,
        version: StandardWalletVersion,
    ) -> Result<(WalletStateInit, RawAccountAddress), Box<dyn Error>> {
        standard_wallet_with_key(code_boc, version, PUBLIC_KEY)
    }

    fn standard_wallet_with_key(
        code_boc: &str,
        version: StandardWalletVersion,
        public_key: [u8; 32],
    ) -> Result<(WalletStateInit, RawAccountAddress), Box<dyn Error>> {
        let code = TonCell::from_boc_base64(code_boc.trim())?;
        let data = wallet_data(version, public_key)?;
        let root = state_init(code, data)?;
        let state = WalletStateInit::from_boc(root.to_boc()?)?;
        let address = state.derive_address(0)?;
        Ok((state, address))
    }

    fn wallet_data(
        version: StandardWalletVersion,
        public_key: [u8; 32],
    ) -> Result<TonCell, Box<dyn Error>> {
        let mut builder = TonCell::builder();
        match version {
            StandardWalletVersion::V1R1
            | StandardWalletVersion::V1R2
            | StandardWalletVersion::V1R3
            | StandardWalletVersion::V2R1
            | StandardWalletVersion::V2R2 => {
                builder.write_num(&0_u32, 32)?;
            }
            StandardWalletVersion::V3R1
            | StandardWalletVersion::V3R2
            | StandardWalletVersion::V4R1
            | StandardWalletVersion::V4R2 => {
                builder.write_num(&0_u32, 32)?;
                builder.write_num(&0x29a9_a317_u32, 32)?;
            }
            StandardWalletVersion::V5R1 => {
                builder.write_bit(true)?;
                builder.write_num(&0_u32, 32)?;
                builder.write_num(&0x7fff_ff11_u32, 32)?;
            }
            StandardWalletVersion::Wallet => {
                builder.write_num(&0_u8, 8)?;
                builder.write_num(&0_u32, 32)?;
                builder.write_num(&0x7fff_7f11_u32, 32)?;
            }
        }
        builder.write_bits(public_key, 256)?;
        if matches!(
            version,
            StandardWalletVersion::V4R1 | StandardWalletVersion::V4R2 | StandardWalletVersion::V5R1
        ) {
            builder.write_bit(false)?;
        }
        if matches!(version, StandardWalletVersion::Wallet) {
            builder.write_bit(false)?;
        }
        Ok(builder.build()?)
    }

    fn state_init(code: TonCell, data: TonCell) -> Result<TonCell, Box<dyn Error>> {
        let mut builder = TonCell::builder();
        builder.write_bit(false)?;
        builder.write_bit(false)?;
        builder.write_bit(true)?;
        builder.write_ref(code)?;
        builder.write_bit(true)?;
        builder.write_ref(data)?;
        builder.write_bit(false)?;
        Ok(builder.build()?)
    }

    #[test]
    fn extracts_keys_from_every_standard_data_layout() -> TestResult {
        let fixtures = [
            (V1R1_CODE, StandardWalletVersion::V1R1),
            (V1R2_CODE, StandardWalletVersion::V1R2),
            (V1R3_CODE, StandardWalletVersion::V1R3),
            (V2R1_CODE, StandardWalletVersion::V2R1),
            (V2R2_CODE, StandardWalletVersion::V2R2),
            (V3R1_CODE, StandardWalletVersion::V3R1),
            (V3R2_CODE, StandardWalletVersion::V3R2),
            (V4R1_CODE, StandardWalletVersion::V4R1),
            (V4R2_CODE, StandardWalletVersion::V4R2),
            (V5R1_CODE, StandardWalletVersion::V5R1),
            (WALLET_CODE, StandardWalletVersion::Wallet),
        ];

        for (code, version) in fixtures {
            let (state_init, address) = standard_wallet(code, version)?;
            let advertised = Ed25519PublicKey::from_bytes(PUBLIC_KEY);
            let verified = state_init.verify_standard_wallet(&address, &advertised)?;
            assert_eq!(verified.version(), version);
            assert_eq!(verified.public_key(), &advertised);

            let json = serde_json::to_string(&state_init)?;
            assert_eq!(serde_json::from_str::<WalletStateInit>(&json)?, state_init);
        }
        Ok(())
    }

    #[test]
    fn address_and_advertised_key_are_independent_required_checks() -> TestResult {
        let (state_init, address) = standard_wallet(V5R1_CODE, StandardWalletVersion::V5R1)?;
        let wrong_address = RawAccountAddress::new(address.workchain(), [0x22; 32]);
        assert_eq!(
            state_init
                .verify_standard_wallet(&wrong_address, &Ed25519PublicKey::from_bytes(PUBLIC_KEY)),
            Err(WalletStateError::AddressMismatch)
        );
        assert_eq!(
            state_init.verify_standard_wallet(&address, &Ed25519PublicKey::from_bytes([0x33; 32])),
            Err(WalletStateError::PublicKeyMismatch)
        );
        Ok(())
    }

    #[test]
    fn unknown_wallet_is_distinct_from_malformed_state() -> TestResult {
        let mut code_builder = TonCell::builder();
        code_builder.write_num(&7_u8, 8)?;
        let code = code_builder.build()?;
        let data = TonCell::builder().build()?;
        let root = state_init(code, data)?;
        let state_init = WalletStateInit::from_boc(root.to_boc()?)?;
        let address = state_init.derive_address(0)?;

        assert_eq!(state_init.extract_standard_public_key()?, None);
        assert_eq!(
            state_init.verify_standard_wallet(&address, &Ed25519PublicKey::from_bytes(PUBLIC_KEY)),
            Err(WalletStateError::UnsupportedWalletCode)
        );

        let empty_cell = TonCell::builder().build()?;
        assert_eq!(
            WalletStateInit::from_boc(empty_cell.to_boc()?),
            Err(WalletStateError::InvalidStateInit)
        );
        Ok(())
    }

    #[test]
    fn wire_value_requires_canonical_standard_base64() -> TestResult {
        let (state_init, _) = standard_wallet(V4R2_CODE, StandardWalletVersion::V4R2)?;
        let encoded = state_init.as_str().to_owned();
        assert_eq!(WalletStateInit::try_from(encoded.as_str())?, state_init);
        assert_eq!(String::from(state_init.clone()), encoded);
        assert!(format!("{state_init:?}").contains("boc_bytes"));
        let url_safe = URL_SAFE.encode(state_init.as_boc());
        assert_ne!(url_safe, state_init.as_str());
        assert_eq!(
            WalletStateInit::try_from(url_safe),
            Err(WalletStateError::InvalidBase64)
        );

        let excessive_padding = format!("{}=", state_init.as_str());
        assert_eq!(
            WalletStateInit::try_from(excessive_padding),
            Err(WalletStateError::InvalidBase64)
        );
        Ok(())
    }

    #[test]
    fn ton_proof_verification_uses_the_state_bound_key() -> TestResult {
        let signing_key = SigningKey::from_bytes(&[0x44; 32]);
        let public_key = signing_key.verifying_key().to_bytes();
        let (state_init, address) =
            standard_wallet_with_key(V5R1_CODE, StandardWalletVersion::V5R1, public_key)?;
        let account = TonAddressItemReply::new(
            address,
            NetworkId::try_from("-239")?,
            state_init,
            Ed25519PublicKey::from_bytes(public_key),
        );
        let timestamp = 1_800_000_000_u64;
        let domain = "wallet.example";
        let payload = "single-use challenge";
        let hash = ton_proof_signing_hash(&address, domain, timestamp, payload)?;
        let proof = TonProof {
            timestamp: Uint64String::from(timestamp),
            domain: TonProofDomain::new(domain.to_owned())?,
            payload: payload.to_owned(),
            signature: Ed25519Signature::from_bytes(signing_key.sign(&hash).to_bytes()),
        };

        assert!(proof.verify_with_account(&account)?);

        let account_with_untrusted_key = TonAddressItemReply::new(
            address,
            NetworkId::try_from("-239")?,
            account.wallet_state_init.clone(),
            Ed25519PublicKey::from_bytes([0x77; 32]),
        );
        assert_eq!(
            proof.verify_with_account(&account_with_untrusted_key),
            Err(AccountVerificationError::WalletState(
                WalletStateError::PublicKeyMismatch
            ))
        );
        Ok(())
    }

    /// Ported from `tonkeeper/tongo/tonconnect/proof_test.go` at
    /// `835e443188a680a08100c3a324f68369c1c4f400`.
    #[test]
    fn tongo_v4_proofs_verify_against_the_state_bound_key() -> TestResult {
        for (code, version, secret) in [
            (V4R1_CODE, StandardWalletVersion::V4R1, [0x41_u8; 32]),
            (V4R2_CODE, StandardWalletVersion::V4R2, [0x42_u8; 32]),
        ] {
            let signing_key = SigningKey::from_bytes(&secret);
            let public_key = signing_key.verifying_key().to_bytes();
            let (state_init, address) = standard_wallet_with_key(code, version, public_key)?;
            let account = TonAddressItemReply::new(
                address,
                NetworkId::try_from("-3")?,
                state_init,
                Ed25519PublicKey::from_bytes(public_key),
            );
            let timestamp = 1_800_000_000_u64;
            let payload = "some-random-secret";
            let hash = ton_proof_signing_hash(&address, "web", timestamp, payload)?;
            let mut proof = TonProof {
                timestamp: Uint64String::from(timestamp),
                domain: TonProofDomain::new("web".to_owned())?,
                payload: payload.to_owned(),
                signature: Ed25519Signature::from_bytes(signing_key.sign(&hash).to_bytes()),
            };
            assert!(proof.verify_with_account(&account)?);
            proof.payload.push('x');
            assert!(!proof.verify_with_account(&account)?);
        }
        Ok(())
    }

    #[test]
    fn sign_data_verification_is_bound_to_the_connected_address() -> TestResult {
        let signing_key = SigningKey::from_bytes(&[0x55; 32]);
        let public_key = signing_key.verifying_key().to_bytes();
        let (state_init, address) =
            standard_wallet_with_key(V4R2_CODE, StandardWalletVersion::V4R2, public_key)?;
        let account = TonAddressItemReply::new(
            address,
            NetworkId::try_from("-239")?,
            state_init,
            Ed25519PublicKey::from_bytes(public_key),
        );
        let timestamp = 1_800_000_001_u64;
        let domain = "wallet.example";
        let text = "Authorize this operation";
        let hash = sign_data_signing_hash(
            &address,
            domain,
            timestamp,
            SignDataSigningPayload::Text(text),
        )?;
        let signature = signing_key.sign(&hash).to_bytes();
        let result = serde_json::from_value::<SignDataResult>(serde_json::json!({
            "signature": STANDARD.encode(signature),
            "address": address.to_string(),
            "timestamp": timestamp,
            "domain": domain,
            "payload": {
                "type": "text",
                "text": text,
                "network": "-239",
                "from": address.to_string()
            }
        }))?;

        assert!(result.verify_with_account(&account)?);

        let mut wrong_result = result;
        wrong_result.address = RawAccountAddress::new(0, [0x66; 32]);
        assert_eq!(
            wrong_result.verify_with_account(&account),
            Err(AccountVerificationError::ResponseAddressMismatch)
        );
        Ok(())
    }
}
