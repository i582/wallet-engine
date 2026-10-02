//! Local address binding for caller-supplied encrypted-comment public keys.

use ton::block_tlb::StateInit;
use ton::ton_core::cell::{TonCell, TonHash};
use ton::ton_core::traits::tlb::TLB as _;
use ton::ton_wallet::{
    WALLET_ID_DEFAULT, WALLET_SUBWALLET_ID_DEFAULT, WALLET_SUBWALLET_ID_DEFAULT_TESTNET,
    WALLET_V5R1_ID_DEFAULT, WALLET_V5R1_ID_DEFAULT_TESTNET, WalletData, WalletV1V2Data,
    WalletV3Data, WalletV4Data, WalletV5Data, WalletVersion,
};

use super::encrypted_comment::EncryptedCommentError;
use crate::{Network, TonAddressString};

/// Authenticates a supplied key by reconstructing standard initial wallet states.
///
/// This works before deployment and never trusts a provider response. V1-V5
/// use their standard wallet IDs, including the workchain-aware SDK defaults;
/// Wallet rev00 uses the engine's network-specific ID in workchain zero. Custom
/// wallet IDs and unknown wallet contracts cannot be proven with only a key and
/// are rejected. This binds only the initial key, so the engine uses it only for
/// an undeployed recipient, whose initial key is its current key.
pub(crate) fn verify_recipient_public_key(
    recipient: &TonAddressString,
    public_key: &[u8; 32],
    network: Network,
) -> Result<(), EncryptedCommentError> {
    let public_key =
        TonHash::from_slice(public_key).map_err(|_| EncryptedCommentError::CellEncoding)?;
    let workchain = recipient.as_address().workchain;

    let data = WalletV1V2Data::new(public_key.clone())
        .to_cell()
        .map_err(|_| EncryptedCommentError::CellEncoding)?;
    if matches_versions(
        recipient,
        &[
            WalletVersion::V1R1,
            WalletVersion::V1R2,
            WalletVersion::V1R3,
            WalletVersion::V2R1,
            WalletVersion::V2R2,
        ],
        &data,
    )? {
        return Ok(());
    }

    // ton-org/ton's V3/V4 constructors add the workchain to the legacy ID.
    // Also recognize the fixed default used by the vendored constructors.
    let legacy_ids = [
        Some(WALLET_ID_DEFAULT),
        WALLET_ID_DEFAULT
            .checked_add(workchain)
            .filter(|id| *id != WALLET_ID_DEFAULT),
    ];
    for wallet_id in legacy_ids.into_iter().flatten() {
        let data = WalletV3Data::new(wallet_id, public_key.clone())
            .to_cell()
            .map_err(|_| EncryptedCommentError::CellEncoding)?;
        if matches_versions(
            recipient,
            &[WalletVersion::V3R1, WalletVersion::V3R2],
            &data,
        )? {
            return Ok(());
        }
        let data = WalletV4Data::new(wallet_id, public_key.clone())
            .to_cell()
            .map_err(|_| EncryptedCommentError::CellEncoding)?;
        if matches_versions(
            recipient,
            &[WalletVersion::V4R1, WalletVersion::V4R2],
            &data,
        )? {
            return Ok(());
        }
    }

    let (v5_id, wallet_id) = match network {
        Network::Mainnet => (WALLET_V5R1_ID_DEFAULT, WALLET_SUBWALLET_ID_DEFAULT),
        Network::Testnet => (
            WALLET_V5R1_ID_DEFAULT_TESTNET,
            WALLET_SUBWALLET_ID_DEFAULT_TESTNET,
        ),
    };
    let v5_ids = [
        Some(v5_id),
        v5_workchain_id(v5_id, workchain).filter(|id| *id != v5_id),
    ];
    for wallet_id in v5_ids.into_iter().flatten() {
        let data = WalletV5Data::new(wallet_id, public_key.clone())
            .to_cell()
            .map_err(|_| EncryptedCommentError::CellEncoding)?;
        if matches_versions(recipient, &[WalletVersion::V5R1], &data)? {
            return Ok(());
        }
    }

    if workchain == 0 {
        let data = WalletData::new(wallet_id, public_key)
            .to_cell()
            .map_err(|_| EncryptedCommentError::CellEncoding)?;
        if matches_versions(recipient, &[WalletVersion::Wallet], &data)? {
            return Ok(());
        }
    }

    Err(EncryptedCommentError::RecipientPublicKeyMismatch)
}

fn matches_versions(
    recipient: &TonAddressString,
    versions: &[WalletVersion],
    data: &TonCell,
) -> Result<bool, EncryptedCommentError> {
    for &version in versions {
        let code =
            WalletVersion::get_code(version).map_err(|_| EncryptedCommentError::CellEncoding)?;
        let address = StateInit::new(code.clone(), data.clone())
            .derive_address(recipient.as_address().workchain)
            .map_err(|_| EncryptedCommentError::CellEncoding)?;
        if address == *recipient.as_address() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// V5's client context encodes workchain as int8 immediately after its tag bit.
/// See ton-org/ton's `WalletV5R1WalletId.ts`; the remaining default context bits
/// and network ID are already encoded in the workchain-zero constant.
fn v5_workchain_id(default_id: i32, workchain: i32) -> Option<i32> {
    let workchain = i8::try_from(workchain).ok()?;
    let workchain_bits = u32::from(u8::from_ne_bytes(workchain.to_ne_bytes())) << 23;
    Some(default_id ^ i32::from_ne_bytes(workchain_bits.to_ne_bytes()))
}

#[cfg(test)]
mod tests {
    use ton::ton_core::types::TonAddress;
    use ton::ton_wallet::{KeyPair, TonWallet};

    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const PUBLIC_KEY: [u8; 32] = [0x5a; 32];

    fn wallet_address(
        version: WalletVersion,
        workchain: i32,
        wallet_id: i32,
        network: Network,
    ) -> Result<TonAddressString, Box<dyn std::error::Error>> {
        let wallet = TonWallet::new_with_params(
            version,
            KeyPair {
                public_key: PUBLIC_KEY,
                secret_key: [0; 64],
            },
            workchain,
            wallet_id,
        )?;
        Ok(TonAddressString::from_address(&wallet.address, network))
    }

    #[test]
    fn binds_all_supported_initial_wallet_versions_on_both_networks() -> TestResult {
        for network in [Network::Mainnet, Network::Testnet] {
            for version in [
                WalletVersion::V1R1,
                WalletVersion::V1R2,
                WalletVersion::V1R3,
                WalletVersion::V2R1,
                WalletVersion::V2R2,
                WalletVersion::V3R1,
                WalletVersion::V3R2,
                WalletVersion::V4R1,
                WalletVersion::V4R2,
            ] {
                let recipient = wallet_address(version, 0, WALLET_ID_DEFAULT, network)?;
                verify_recipient_public_key(&recipient, &PUBLIC_KEY, network)?;
                assert!(matches!(
                    verify_recipient_public_key(&recipient, &[0x33; 32], network),
                    Err(EncryptedCommentError::RecipientPublicKeyMismatch)
                ));
            }
            let (v5_id, wallet_id) = match network {
                Network::Mainnet => (WALLET_V5R1_ID_DEFAULT, WALLET_SUBWALLET_ID_DEFAULT),
                Network::Testnet => (
                    WALLET_V5R1_ID_DEFAULT_TESTNET,
                    WALLET_SUBWALLET_ID_DEFAULT_TESTNET,
                ),
            };
            for (version, wallet_id) in [
                (WalletVersion::V5R1, v5_id),
                (WalletVersion::Wallet, wallet_id),
            ] {
                let recipient = wallet_address(version, 0, wallet_id, network)?;
                verify_recipient_public_key(&recipient, &PUBLIC_KEY, network)?;
                assert!(matches!(
                    verify_recipient_public_key(&recipient, &[0x33; 32], network),
                    Err(EncryptedCommentError::RecipientPublicKeyMismatch)
                ));
                let other_network = match network {
                    Network::Mainnet => Network::Testnet,
                    Network::Testnet => Network::Mainnet,
                };
                assert!(matches!(
                    verify_recipient_public_key(&recipient, &PUBLIC_KEY, other_network),
                    Err(EncryptedCommentError::RecipientPublicKeyMismatch)
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn binds_workchain_aware_standard_ids_and_address_spellings() -> TestResult {
        for (version, wallet_id) in [
            (WalletVersion::V3R2, 698_983_190),
            (WalletVersion::V4R2, 698_983_190),
            // Values independently pinned by the official V5 wallet-ID module.
            (WalletVersion::V5R1, 8_388_369),
        ] {
            let recipient = wallet_address(version, -1, wallet_id, Network::Mainnet)?;
            verify_recipient_public_key(&recipient, &PUBLIC_KEY, Network::Mainnet)?;
            let raw = TonAddressString::try_from(recipient.as_address().to_hex())?;
            verify_recipient_public_key(&raw, &PUBLIC_KEY, Network::Mainnet)?;
        }
        let recipient = wallet_address(WalletVersion::V5R1, -1, 8_388_605, Network::Testnet)?;
        verify_recipient_public_key(&recipient, &PUBLIC_KEY, Network::Testnet)?;
        Ok(())
    }

    #[test]
    fn rejects_nondefault_ids_unknown_contracts_and_unrelated_addresses() -> TestResult {
        let custom = wallet_address(WalletVersion::V4R2, 0, 42, Network::Mainnet)?;
        assert!(matches!(
            verify_recipient_public_key(&custom, &PUBLIC_KEY, Network::Mainnet),
            Err(EncryptedCommentError::RecipientPublicKeyMismatch)
        ));
        let unsupported = wallet_address(
            WalletVersion::HLV2R2,
            0,
            WALLET_ID_DEFAULT,
            Network::Mainnet,
        )?;
        assert!(matches!(
            verify_recipient_public_key(&unsupported, &PUBLIC_KEY, Network::Mainnet),
            Err(EncryptedCommentError::RecipientPublicKeyMismatch)
        ));
        let unrelated = TonAddressString::from_address(
            &TonAddress::new(0, TonHash::from_slice(&[0x11; 32])?),
            Network::Mainnet,
        );
        assert!(matches!(
            verify_recipient_public_key(&unrelated, &PUBLIC_KEY, Network::Mainnet),
            Err(EncryptedCommentError::RecipientPublicKeyMismatch)
        ));
        Ok(())
    }
}
