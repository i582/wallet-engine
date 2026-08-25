//! Rotation mnemonic generation and wallet derivation.
//!
//! The engine supports exactly one recovery scheme: the rotation mnemonic of
//! [TEP-0003 section 3.3](https://github.com/ton-blockchain/TEPs/blob/master/text/0003-wallets.md#33-rotation-mnemonic).
//! Its anchor half determines the wallet account address; its signing half
//! signs outgoing messages and is replaced on rotation.
//!
//! The engine embeds Wallet rev00, which is not declared final. The current
//! lifecycle initializes it with the anchor public key and uses the anchor key
//! for outgoing signatures. [`derive_rotation_keys`] already derives the
//! separate signing key, but key rotation is not wired into the engine yet.
//!
//! Lifecycle and signing code share this private module so both paths derive
//! the same key pair, contract wallet ID, and address for a selected network.

use std::ops::Deref;

use ed25519_dalek::SigningKey;
use ton::block_tlb::StateInit;
use ton::ton_core::cell::TonHash;
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

/// A derived wallet whose vendored key pair wipes itself on drop.
pub(crate) struct SensitiveWallet(TonWallet);

impl Deref for SensitiveWallet {
    type Target = TonWallet;

    fn deref(&self) -> &Self::Target {
        &self.0
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
    #[allow(
        dead_code,
        reason = "unused until Wallet key rotation is wired into the engine"
    )]
    pub(crate) signing: SigningKey,
}

/// Derives both key pairs of a rotation mnemonic.
///
/// Each half becomes a passphraseless BIP-39 seed and then an Ed25519 key on
/// [`TON_ACCOUNT_PATH`], independently of the other half. The derivation is
/// infallible: a parsed [`RotationMnemonic`] always yields two keys.
pub(crate) fn derive_rotation_keys(mnemonic: &RotationMnemonic) -> RotationKeys {
    let derive = |half: &Bip39Half| {
        signing_key(&derive_path(half.to_seed("").as_slice(), &TON_ACCOUNT_PATH))
    };

    RotationKeys {
        anchor: derive(mnemonic.anchor()),
        signing: derive(mnemonic.signing()),
    }
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
/// The current Wallet rev00 lifecycle initializes the stored key from the
/// anchor, so the anchor key determines the address and signs messages. Key
/// rotation will switch ordinary signing to the separately derived signing key.
pub(crate) fn derive_wallet(
    mnemonic: &str,
    network: Network,
) -> Result<SensitiveWallet, WalletCryptoError> {
    let mnemonic =
        RotationMnemonic::parse(mnemonic).map_err(|_| WalletCryptoError::InvalidMnemonic)?;
    let keys = derive_rotation_keys(&mnemonic);

    let contract_wallet_id = wallet_contract_id(network);

    TonWallet::new_with_params(
        WalletVersion::Wallet,
        ton_key_pair(&keys.anchor),
        0,
        contract_wallet_id,
    )
    .map(SensitiveWallet)
    .map_err(|_| WalletCryptoError::WalletConstruction)
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

    /// Pins the embedded Wallet rev00 code-cell hash.
    #[test]
    fn wallet_code_matches_upstream_hash() -> Result<(), Box<dyn std::error::Error>> {
        const EXPECTED_HASH: [u8; 32] = [
            0x37, 0x91, 0xf4, 0xbf, 0xbb, 0x8a, 0x2f, 0x69, 0x7a, 0x5c, 0xe3, 0x59, 0x8f, 0xdc,
            0xee, 0xaa, 0xa0, 0xea, 0xd0, 0xba, 0xdd, 0xed, 0x84, 0x73, 0xa3, 0x5f, 0xb6, 0x9f,
            0x76, 0xb0, 0x21, 0xe5,
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

    /// Wallet rev00 currently takes the anchor key, and the public-state path
    /// must agree with the full derivation.
    #[test]
    fn wallet_derives_from_the_anchor_key() -> Result<(), Box<dyn std::error::Error>> {
        let wallet = derive_wallet(ROTATION_PHRASE, Network::Testnet)?;

        assert_eq!(
            hex(&wallet.key_pair.public_key),
            "7952e94118f34607c75e23258dd9220d66ccac5a3ee074125c25068e8107bfbf"
        );

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
