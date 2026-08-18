//! TON mnemonic generation and wallet derivation.
//!
//! Lifecycle and signing code share this private module so both paths derive
//! the same key pair, contract wallet ID, and address for a selected network.

use std::ops::Deref;

use ton::block_tlb::StateInit;
use ton::ton_core::cell::TonHash;
use ton::ton_core::traits::tlb::TLB;
use ton::ton_core::types::TonAddress;
use ton::ton_wallet::{
    Mnemonic, TonWallet, WALLET_V5R1_ID_DEFAULT, WALLET_V5R1_ID_DEFAULT_TESTNET, WORDLIST_EN_SET,
    WalletVersion,
};
use zeroize::Zeroizing;

use crate::Network;

const MNEMONIC_ENTROPY_BYTES: usize = 48;
const MNEMONIC_WORD_COUNT: usize = 24;

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
    fn from_generated_words(words: &[&str]) -> Self {
        let mut bytes = Vec::new();

        for (index, word) in words.iter().enumerate() {
            if index != 0 {
                bytes.push(b' ');
            }

            bytes.extend_from_slice(word.as_bytes());
        }

        Self {
            bytes: Zeroizing::new(bytes),
        }
    }

    pub(crate) fn from_words(words: Vec<String>) -> Result<Self, WalletCryptoError> {
        let words = Zeroizing::new(words);

        if words.len() != MNEMONIC_WORD_COUNT {
            return Err(WalletCryptoError::InvalidMnemonic);
        }

        let word_refs = words.iter().map(String::as_str).collect::<Vec<_>>();
        let candidate = Self::from_generated_words(&word_refs);
        candidate.validate()?;

        Ok(candidate)
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

    fn validate(&self) -> Result<(), WalletCryptoError> {
        let phrase = self.as_str()?;
        Mnemonic::from_str(phrase, None)
            .map(|_| ())
            .map_err(|_| WalletCryptoError::InvalidMnemonic)
    }
}

/// Generates a passwordless 24-word TON mnemonic using system randomness.
///
/// TON mnemonics are not BIP-39 checksummed phrases. Candidate words are
/// sampled from the TON English word list until `Mnemonic::new` accepts the
/// passwordless seed-version constraint.
pub(crate) fn generate_mnemonic() -> Result<SensitiveMnemonic, WalletCryptoError> {
    let mut wordlist = WORDLIST_EN_SET.iter().copied().collect::<Vec<_>>();
    wordlist.sort_unstable();

    if wordlist.len() != 2048 {
        return Err(WalletCryptoError::InvalidMnemonic);
    }

    loop {
        let mut entropy = Zeroizing::new([0_u8; MNEMONIC_ENTROPY_BYTES]);
        getrandom::fill(entropy.as_mut()).map_err(|_| WalletCryptoError::RandomGeneration)?;

        let words = entropy
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| {
                let index = usize::from(u16::from_be_bytes(*bytes) & 0x07ff);
                #[allow(
                    clippy::indexing_slicing,
                    reason = "the 11-bit index is always within the validated 2048-word list"
                )]
                let word = wordlist[index];
                word
            })
            .collect::<Vec<_>>();

        debug_assert_eq!(
            words.len(),
            MNEMONIC_WORD_COUNT,
            "entropy must produce the configured mnemonic word count"
        );

        let candidate = SensitiveMnemonic::from_generated_words(&words);
        if Mnemonic::new(words, None).is_ok() {
            return Ok(candidate);
        }
    }
}

/// Derives the wallet contract used by lifecycle and transaction signing.
///
/// Its V5-compatible `wallet_id` combines the TON `network_global_id` with the
/// client context. Mainnet and testnet therefore derive different contracts.
pub(crate) fn derive_wallet(
    mnemonic: &str,
    network: Network,
) -> Result<SensitiveWallet, WalletCryptoError> {
    let mnemonic =
        Mnemonic::from_str(mnemonic, None).map_err(|_| WalletCryptoError::InvalidMnemonic)?;
    let key_pair = mnemonic
        .to_key_pair()
        .map_err(|_| WalletCryptoError::InvalidMnemonic)?;

    let contract_wallet_id = wallet_contract_id(network);

    TonWallet::new_with_params(WalletVersion::Wallet, key_pair, 0, contract_wallet_id)
        .map(SensitiveWallet)
        .map_err(|_| WalletCryptoError::WalletConstruction)
}

/// Returns the network-specific contract identifier used by the wallet.
const fn wallet_contract_id(network: Network) -> i32 {
    match network {
        Network::Mainnet => WALLET_V5R1_ID_DEFAULT,
        Network::Testnet => WALLET_V5R1_ID_DEFAULT_TESTNET,
    }
}

/// Derives the wallet address and `StateInit` from public metadata only.
///
/// The public key is sufficient because wallet code and initial data are
/// deterministic. No signing key or mnemonic is involved.
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

    /// Pins the embedded wallet contract to the reviewed upstream build.
    #[test]
    fn wallet_code_matches_upstream_hash() -> Result<(), Box<dyn std::error::Error>> {
        const EXPECTED_HASH: [u8; 32] = [
            0x99, 0xcc, 0xa0, 0x9e, 0xd5, 0xdf, 0xc6, 0x04, 0xfb, 0xfe, 0x67, 0xe1, 0xd2, 0xd6,
            0x9a, 0x00, 0xba, 0x74, 0x85, 0x2b, 0x23, 0x65, 0xa2, 0x3b, 0x49, 0x62, 0x8b, 0x56,
            0x33, 0x79, 0x78, 0x98,
        ];
        let code = WalletVersion::get_code(WalletVersion::Wallet)?;
        let hash = code.cell_hash()?;
        assert_eq!(hash.as_slice(), EXPECTED_HASH.as_slice());
        Ok(())
    }

    #[test]
    fn sensitive_mnemonic_rejects_invalid_utf8_and_invalid_ton_words() {
        assert!(matches!(
            SensitiveMnemonic::from_bytes(vec![0xff]),
            Err(WalletCryptoError::InvalidMnemonic)
        ));
        assert!(matches!(
            SensitiveMnemonic::from_bytes(b"not a TON mnemonic".to_vec()),
            Err(WalletCryptoError::InvalidMnemonic)
        ));
    }
}
