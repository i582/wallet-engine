use ton::ton_wallet::{Mnemonic, TonWallet, WORDLIST_EN_SET, WalletVersion};

use crate::Network;

const MAINNET_GLOBAL_ID: i32 = -239;
const TESTNET_GLOBAL_ID: i32 = -3;
const V5R1_CLIENT_CONTEXT_ID: i32 = i32::MIN;
const MNEMONIC_ENTROPY_BYTES: usize = 48;
const MNEMONIC_WORD_COUNT: usize = 24;

#[derive(Debug, thiserror::Error)]
pub(crate) enum WalletCryptoError {
    #[error("secure random generation failed")]
    RandomGeneration,
    #[error("invalid recovery phrase")]
    InvalidMnemonic,
    #[error("V5R1 wallet construction failed")]
    WalletConstruction,
}

/// Generates a passwordless 24-word TON mnemonic using system randomness.
///
/// TON mnemonics are not BIP-39 checksummed phrases. Candidate words are
/// sampled from the TON English word list until `Mnemonic::new` accepts the
/// passwordless seed-version constraint.
pub(crate) fn generate_mnemonic() -> Result<String, WalletCryptoError> {
    let mut wordlist = WORDLIST_EN_SET.iter().copied().collect::<Vec<_>>();
    wordlist.sort_unstable();
    if wordlist.len() != 2048 {
        return Err(WalletCryptoError::InvalidMnemonic);
    }

    loop {
        let mut entropy = [0_u8; MNEMONIC_ENTROPY_BYTES];
        getrandom::fill(&mut entropy).map_err(|_| WalletCryptoError::RandomGeneration)?;

        let words = entropy
            .chunks_exact(2)
            .map(|bytes| {
                let value = u16::from_be_bytes([bytes[0], bytes[1]]);
                wordlist[usize::from(value & 0x07ff)]
            })
            .collect::<Vec<_>>();

        debug_assert_eq!(words.len(), MNEMONIC_WORD_COUNT);
        if Mnemonic::new(words.clone(), None).is_ok() {
            return Ok(words.join(" "));
        }
    }
}

/// Derives the V5R1 wallet contract used by lifecycle and transaction signing.
pub(crate) fn derive_v5r1_wallet(
    mnemonic: &str,
    network: Network,
) -> Result<TonWallet, WalletCryptoError> {
    let mnemonic =
        Mnemonic::from_str(mnemonic, None).map_err(|_| WalletCryptoError::InvalidMnemonic)?;
    let key_pair = mnemonic
        .to_key_pair()
        .map_err(|_| WalletCryptoError::InvalidMnemonic)?;
    let contract_wallet_id = v5r1_contract_wallet_id(network);

    TonWallet::new_with_params(WalletVersion::V5R1, key_pair, 0, contract_wallet_id)
        .map_err(|_| WalletCryptoError::WalletConstruction)
}

const fn v5r1_contract_wallet_id(network: Network) -> i32 {
    let network_global_id = match network {
        Network::Mainnet => MAINNET_GLOBAL_ID,
        Network::Testnet => TESTNET_GLOBAL_ID,
    };
    network_global_id ^ V5R1_CLIENT_CONTEXT_ID
}

pub(crate) fn derive_v5r1_address(
    mnemonic: &str,
    network: Network,
    bounceable: bool,
) -> Result<String, WalletCryptoError> {
    let wallet = derive_v5r1_wallet(mnemonic, network)?;
    Ok(wallet
        .address
        .to_base64(network == Network::Mainnet, bounceable, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MNEMONIC: &str = "cupboard match uphold miracle fog balance unknown region share hand trophy million toy narrow ability exchange first toast fresh maid report cram strong later";

    #[test]
    fn derives_the_known_mainnet_v5r1_address() {
        let address = derive_v5r1_address(TEST_MNEMONIC, Network::Mainnet, true).unwrap();

        assert_eq!(address, "EQAz8sBz-Twy965gFWNHlwa2ArkRLaoVzAowtRaW542bDO5p");
    }

    #[test]
    fn network_and_bounce_flags_change_the_friendly_address() {
        let mainnet = derive_v5r1_address(TEST_MNEMONIC, Network::Mainnet, true).unwrap();
        let testnet = derive_v5r1_address(TEST_MNEMONIC, Network::Testnet, true).unwrap();
        let non_bounceable = derive_v5r1_address(TEST_MNEMONIC, Network::Mainnet, false).unwrap();

        assert_ne!(mainnet, testnet);
        assert_ne!(mainnet, non_bounceable);
        assert!(testnet.starts_with("kQ"));
        assert!(non_bounceable.starts_with("UQ"));
    }

    #[test]
    fn v5r1_wallet_id_depends_on_the_network_global_id() {
        let mainnet = derive_v5r1_wallet(TEST_MNEMONIC, Network::Mainnet).unwrap();
        let testnet = derive_v5r1_wallet(TEST_MNEMONIC, Network::Testnet).unwrap();

        assert_eq!(mainnet.wallet_id, 0x7FFF_FF11);
        assert_eq!(testnet.wallet_id, 0x7FFF_FFFD);
        assert_ne!(mainnet.address, testnet.address);
    }

    #[test]
    fn generated_mnemonic_is_valid_for_a_v5r1_wallet() {
        let mnemonic = generate_mnemonic().unwrap();

        assert_eq!(mnemonic.split_whitespace().count(), MNEMONIC_WORD_COUNT);
        assert!(Mnemonic::from_str(&mnemonic, None).is_ok());
        assert!(derive_v5r1_wallet(&mnemonic, Network::Testnet).is_ok());
    }
}
