//! Rotation mnemonic generation and wallet derivation.
//!
//! The engine supports exactly one recovery scheme: the rotation mnemonic of
//! [TEP-0003 section 3.3](https://github.com/ton-blockchain/TEPs/blob/master/text/0003-wallets.md#33-rotation-mnemonic).
//! Its anchor half determines the wallet account address; its signing half
//! signs outgoing messages and is replaced on rotation.
//!
//! The engine embeds Wallet rev00, which is not declared final. Its initial
//! state and address use the anchor public key, while ordinary outgoing
//! requests are signed by the signing half of the mnemonic.
//!
//! Lifecycle and signing code share this private module so both paths derive
//! the same key pair, contract wallet ID, and address for a selected network.

use std::ops::Deref;

use ed25519_dalek::{Signer as _, SigningKey};
use ton::block_tlb::StateInit;
use ton::errors::TonError;
use ton::ton_core::cell::{TonCell, TonHash};
use ton::ton_core::traits::tlb::TLB;
use ton::ton_core::types::TonAddress;
use ton::ton_wallet::{
    KeyPair, TonWallet, WALLET_SUBWALLET_ID_DEFAULT, WALLET_SUBWALLET_ID_DEFAULT_TESTNET,
    WalletVersion,
};
use zeroize::Zeroizing;

use super::mnemonic::{Bip39Half, ENTROPY_LEN, RotationMnemonic};
use super::slip_0010::{TON_ACCOUNT_PATH, derive_path, signing_key};
use crate::Network;

#[derive(Debug, thiserror::Error)]
pub(crate) enum WalletCryptoError {
    #[error("secure random generation failed")]
    RandomGeneration,
    #[error("invalid recovery phrase")]
    InvalidMnemonic,
    #[error("wallet construction failed")]
    WalletConstruction,
}

/// Mnemonic bytes owned by one Rust operation.
///
/// Platform and FFI boundaries can still create transient copies. This type
/// wipes the buffer retained by the wallet engine when it is dropped.
pub(crate) struct SensitiveMnemonic {
    bytes: Zeroizing<Vec<u8>>,
}

/// A derived wallet with separate address and ordinary-signing keys.
///
/// The inner [`TonWallet`] retains the anchor key so any attached `StateInit`
/// remains address-correct. The separate signing key signs Wallet requests.
/// Both secret keys wipe themselves on drop.
pub(crate) struct SensitiveWallet {
    wallet: TonWallet,
    signing: SigningKey,
}

impl Deref for SensitiveWallet {
    type Target = TonWallet;

    fn deref(&self) -> &Self::Target {
        &self.wallet
    }
}

impl SensitiveWallet {
    /// Reports whether the wallet is still in its initial, unrotated state.
    pub(crate) fn is_pre_rotation(&self) -> bool {
        self.wallet.key_pair.public_key == self.signing_public_key()
    }

    /// Returns the public key that authorizes ordinary outgoing requests.
    pub(crate) fn signing_public_key(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    /// Builds and signs an external Wallet request while preserving anchor-based `StateInit`.
    pub(crate) fn create_ext_in_msg_with_modes(
        &self,
        int_msgs: Vec<TonCell>,
        int_msg_modes: Vec<u8>,
        seqno: u32,
        expire_at: u32,
        add_state_init: bool,
    ) -> Result<TonCell, TonError> {
        let body =
            self.wallet
                .create_ext_in_body_with_modes(expire_at, seqno, int_msgs, int_msg_modes)?;
        let signed = self.sign_wallet_body(&body)?;
        self.wallet
            .create_ext_in_msg_from_body(signed, add_state_init)
    }

    /// Builds and signs an internal Wallet request while preserving anchor-based `StateInit`.
    pub(crate) fn create_internal_signed_msg_with_modes(
        &self,
        int_msgs: Vec<TonCell>,
        int_msg_modes: Vec<u8>,
        seqno: u32,
        expire_at: u32,
        add_state_init: bool,
    ) -> Result<TonCell, TonError> {
        let body = WalletVersion::build_internal_signed_body_with_modes(
            self.wallet.version,
            expire_at,
            seqno,
            self.wallet.wallet_id,
            int_msgs,
            int_msg_modes,
        )?;
        let signed = self.sign_wallet_body(&body)?;
        self.wallet
            .create_internal_signed_msg_from_body(signed, add_state_init)
    }

    /// Signs the request-cell hash and prepends the Wallet rev00 signature.
    fn sign_wallet_body(&self, body: &TonCell) -> Result<TonCell, TonError> {
        let hash = body.cell_hash()?;
        let signature = self.signing.sign(hash.as_slice()).to_bytes();
        let mut builder = TonCell::builder();
        builder.write_bits(signature, 512)?;
        builder.write_cell(body)?;
        Ok(builder.build()?)
    }
}

impl SensitiveMnemonic {
    pub(crate) fn from_words(words: Vec<String>) -> Result<Self, WalletCryptoError> {
        let words = Zeroizing::new(words);

        let mut bytes = Vec::new();
        for (index, word) in words.iter().enumerate() {
            if index != 0 {
                bytes.push(b' ');
            }

            bytes.extend_from_slice(word.as_bytes());
        }

        Self::from_bytes(bytes)
    }

    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Result<Self, WalletCryptoError> {
        let candidate = Self {
            bytes: Zeroizing::new(bytes),
        };
        candidate.validate()?;

        Ok(candidate)
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn as_str(&self) -> Result<&str, WalletCryptoError> {
        std::str::from_utf8(&self.bytes).map_err(|_| WalletCryptoError::InvalidMnemonic)
    }

    /// Parses the buffer as a rotation mnemonic: two checksummed BIP-39 halves.
    fn validate(&self) -> Result<(), WalletCryptoError> {
        let phrase = self.as_str()?;
        RotationMnemonic::parse(phrase)
            .map(|_| ())
            .map_err(|_| WalletCryptoError::InvalidMnemonic)
    }
}

/// The anchor and signing key pairs of one rotation mnemonic.
///
/// [`SigningKey`] wipes its secret on drop.
pub(crate) struct RotationKeys {
    /// Determines the wallet account address. Never changes.
    pub(crate) anchor: SigningKey,
    /// Signs ordinary outgoing messages. Replaced on rotation.
    pub(crate) signing: SigningKey,
}

/// Derives both key pairs of a rotation mnemonic.
///
/// Each half becomes a passphraseless BIP-39 seed and then an Ed25519 key on
/// [`TON_ACCOUNT_PATH`], independently of the other half. The derivation is
/// infallible: a parsed [`RotationMnemonic`] always yields two keys.
pub(crate) fn derive_rotation_keys(mnemonic: &RotationMnemonic) -> RotationKeys {
    RotationKeys {
        anchor: derive_half_key(mnemonic.anchor()),
        signing: derive_half_key(mnemonic.signing()),
    }
}

/// Derives the Ed25519 account key represented by one mnemonic half.
pub(crate) fn derive_half_key(half: &Bip39Half) -> SigningKey {
    signing_key(&derive_path(half.to_seed("").as_slice(), &TON_ACCOUNT_PATH))
}

/// Generates the initial 12-word recovery phrase from one 128-bit draw.
///
/// A new wallet starts before its one-time key rotation: the signing key
/// equals the anchor key, so the user records a single 12-word half. The
/// engine expands it into both halves wherever a full rotation mnemonic is
/// needed; the phrase gains its second, independent half at rotation. Any 16
/// bytes encode a valid half, so unlike a TON mnemonic no rejection sampling
/// is involved.
pub(crate) fn generate_mnemonic() -> Result<SensitiveMnemonic, WalletCryptoError> {
    let mut entropy = Zeroizing::new([0_u8; ENTROPY_LEN]);
    getrandom::fill(entropy.as_mut()).map_err(|_| WalletCryptoError::RandomGeneration)?;

    let half = Bip39Half::from_entropy(&entropy).map_err(|_| WalletCryptoError::InvalidMnemonic)?;

    SensitiveMnemonic::from_bytes(half.to_phrase().as_bytes().to_vec())
}

/// Converts a derived Ed25519 key into the vendored key-pair layout.
fn ton_key_pair(key: &SigningKey) -> KeyPair {
    KeyPair {
        public_key: key.verifying_key().to_bytes(),
        secret_key: key.to_keypair_bytes(),
    }
}

/// Derives the wallet contract used by lifecycle and transaction signing.
///
/// Its V5-compatible `wallet_id` combines the TON `network_global_id` with the
/// client context. Mainnet and testnet therefore derive different contracts.
///
/// Wallet rev00 initializes its stored key from the anchor, so the anchor key
/// determines the address and deployment state. Ordinary requests use the
/// separately derived signing key; before rotation both keys are identical.
pub(crate) fn derive_wallet(
    mnemonic: &str,
    network: Network,
) -> Result<SensitiveWallet, WalletCryptoError> {
    let mnemonic =
        RotationMnemonic::parse(mnemonic).map_err(|_| WalletCryptoError::InvalidMnemonic)?;
    let keys = derive_rotation_keys(&mnemonic);

    let contract_wallet_id = wallet_contract_id(network);

    let wallet = TonWallet::new_with_params(
        WalletVersion::Wallet,
        ton_key_pair(&keys.anchor),
        0,
        contract_wallet_id,
    )
    .map_err(|_| WalletCryptoError::WalletConstruction)?;

    Ok(SensitiveWallet {
        wallet,
        signing: keys.signing,
    })
}

/// Returns the network-specific contract identifier used by the wallet.
const fn wallet_contract_id(network: Network) -> i32 {
    match network {
        Network::Mainnet => WALLET_SUBWALLET_ID_DEFAULT,
        Network::Testnet => WALLET_SUBWALLET_ID_DEFAULT_TESTNET,
    }
}

/// Derives the wallet address and `StateInit` from public metadata only.
///
/// The anchor public key is sufficient because wallet code and initial data
/// are deterministic. No signing key or mnemonic is involved.
pub(crate) fn derive_wallet_public_state(
    public_key: &[u8],
    network: Network,
) -> Result<(TonAddress, StateInit), WalletCryptoError> {
    let public_key =
        TonHash::from_slice(public_key).map_err(|_| WalletCryptoError::WalletConstruction)?;
    let wallet_id = wallet_contract_id(network);

    let code = WalletVersion::get_code(WalletVersion::Wallet)
        .map_err(|_| WalletCryptoError::WalletConstruction)?
        .clone();
    let data = ton::ton_wallet::WalletData::new(wallet_id, public_key)
        .to_cell()
        .map_err(|_| WalletCryptoError::WalletConstruction)?;
    let state_init = StateInit::new(code, data);

    let address = state_init
        .derive_address(0)
        .map_err(|_| WalletCryptoError::WalletConstruction)?;

    Ok((address, state_init))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::mnemonic::HALF_WORD_COUNT;

    /// The two official BIP-39 half vectors joined into one rotation phrase.
    const ROTATION_PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about \
                                   zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong";

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Pins the embedded wallet trampoline code-cell hash.
    #[test]
    fn wallet_code_matches_upstream_hash() -> Result<(), Box<dyn std::error::Error>> {
        const EXPECTED_HASH: [u8; 32] = [
            0x91, 0x49, 0xae, 0x51, 0xc1, 0xe4, 0x68, 0x97, 0x10, 0xce, 0xbf, 0x78, 0x30, 0x29,
            0x7b, 0x16, 0xac, 0xfb, 0xad, 0xb3, 0x63, 0xa9, 0x20, 0xa5, 0x37, 0x89, 0x3e, 0x7f,
            0xfe, 0xec, 0xa7, 0x68,
        ];
        let code = WalletVersion::get_code(WalletVersion::Wallet)?;
        let hash = code.cell_hash()?;
        assert_eq!(hash.as_slice(), EXPECTED_HASH.as_slice());
        Ok(())
    }

    /// Both rotation keys, pinned against an independent implementation of
    /// BIP-39 seeding plus SLIP-0010 on `m/44'/607'/0'`.
    #[test]
    fn rotation_keys_match_the_reference_derivation() -> Result<(), Box<dyn std::error::Error>> {
        let mnemonic = RotationMnemonic::parse(ROTATION_PHRASE)?;
        let keys = derive_rotation_keys(&mnemonic);

        assert_eq!(
            hex(keys.anchor.as_bytes()),
            "b477ef5ed17fb8a2b8faddd7a9835a227243a82c70b190c7af4896155aa7df9f"
        );
        assert_eq!(
            hex(&keys.anchor.verifying_key().to_bytes()),
            "7952e94118f34607c75e23258dd9220d66ccac5a3ee074125c25068e8107bfbf"
        );
        assert_eq!(
            hex(keys.signing.as_bytes()),
            "a7e4e571135b501905f0be50d4bbd7a407e194cc23b1573c0be8a769aef43333"
        );
        assert_eq!(
            hex(&keys.signing.verifying_key().to_bytes()),
            "5d6320a0546c2df0908f0477e1ade79226faf854d041548f846b58872de5213e"
        );
        Ok(())
    }

    /// Wallet rev00 derives its address from the anchor and keeps the distinct
    /// signing half for ordinary requests.
    #[test]
    fn wallet_separates_anchor_and_signing_keys() -> Result<(), Box<dyn std::error::Error>> {
        let wallet = derive_wallet(ROTATION_PHRASE, Network::Testnet)?;

        assert_eq!(
            hex(&wallet.key_pair.public_key),
            "7952e94118f34607c75e23258dd9220d66ccac5a3ee074125c25068e8107bfbf"
        );
        assert_eq!(
            hex(&wallet.signing_public_key()),
            "5d6320a0546c2df0908f0477e1ade79226faf854d041548f846b58872de5213e"
        );
        assert!(!wallet.is_pre_rotation());

        let (address, _) =
            derive_wallet_public_state(&wallet.key_pair.public_key, Network::Testnet)?;
        assert_eq!(address, wallet.address);

        // Mainnet uses another wallet ID, so the address must differ.
        let mainnet = derive_wallet(ROTATION_PHRASE, Network::Mainnet)?;
        assert_ne!(mainnet.address, wallet.address);
        Ok(())
    }

    /// A generated phrase is the 12-word pre-rotation form and derives a wallet.
    #[test]
    fn generated_mnemonics_are_valid_and_unique() -> Result<(), Box<dyn std::error::Error>> {
        let first = generate_mnemonic()?;
        let second = generate_mnemonic()?;

        assert_ne!(first.as_bytes(), second.as_bytes());

        let phrase = first.as_str()?;
        assert_eq!(phrase.split_whitespace().count(), HALF_WORD_COUNT);

        let mnemonic = RotationMnemonic::parse(phrase)?;
        assert!(mnemonic.is_pre_rotation());

        let _ = derive_wallet(phrase, Network::Testnet)?;
        Ok(())
    }

    /// The engine owns the pre-rotation expansion: a 12-word phrase and its
    /// hand-duplicated 24-word form must derive the same wallet, and both
    /// keys of the pre-rotation wallet must be the anchor key.
    #[test]
    fn twelve_word_phrase_derives_the_duplicated_wallet() -> Result<(), Box<dyn std::error::Error>>
    {
        const HALF: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

        let duplicated = format!("{HALF} {HALF}");
        let from_half = derive_wallet(HALF, Network::Testnet)?;
        let from_duplicated = derive_wallet(&duplicated, Network::Testnet)?;

        assert_eq!(from_half.address, from_duplicated.address);
        assert_eq!(
            from_half.key_pair.public_key,
            from_duplicated.key_pair.public_key
        );
        assert_eq!(
            from_half.signing_public_key(),
            from_half.key_pair.public_key
        );
        assert!(from_half.is_pre_rotation());
        assert!(from_duplicated.is_pre_rotation());

        let keys = derive_rotation_keys(&RotationMnemonic::parse(HALF)?);
        assert_eq!(keys.anchor.as_bytes(), keys.signing.as_bytes());
        assert_eq!(
            hex(&keys.anchor.verifying_key().to_bytes()),
            "7952e94118f34607c75e23258dd9220d66ccac5a3ee074125c25068e8107bfbf"
        );
        Ok(())
    }

    /// TON-style mnemonics belong to the scheme this engine does not support.
    #[test]
    fn rejects_non_rotation_phrases() {
        // A valid 24-word TON mnemonic: no per-half BIP-39 checksums.
        const TON_MNEMONIC: &str = "dose ice enrich trigger test dove century still betray gas diet dune use other base gym mad law immense village world example praise game";

        assert!(matches!(
            SensitiveMnemonic::from_bytes(TON_MNEMONIC.as_bytes().to_vec()),
            Err(WalletCryptoError::InvalidMnemonic)
        ));
        assert!(matches!(
            SensitiveMnemonic::from_bytes(vec![0xff]),
            Err(WalletCryptoError::InvalidMnemonic)
        ));
        assert!(matches!(
            SensitiveMnemonic::from_bytes(b"not a rotation mnemonic".to_vec()),
            Err(WalletCryptoError::InvalidMnemonic)
        ));
        assert!(matches!(
            derive_wallet(TON_MNEMONIC, Network::Testnet),
            Err(WalletCryptoError::InvalidMnemonic)
        ));
    }
}
