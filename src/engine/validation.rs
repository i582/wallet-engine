//! Validation of public configuration and send requests.

use crate::{WalletClientConfig, WalletClientError};

pub(super) fn validate_config(config: &WalletClientConfig) -> Result<(), WalletClientError> {
    if config
        .local_secret_ref
        .as_ref()
        .is_some_and(|secret_ref| secret_ref.value.trim().is_empty())
    {
        return Err(WalletClientError::InvalidLocalSecretReference);
    }

    if config.public_key.len() != 32 {
        return Err(WalletClientError::InvalidWalletPublicKey);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_config;
    use crate::wallet::crypto::{derive_wallet, derive_wallet_public_state};
    use crate::{
        Network, ProtectedSecretRef, ProviderConfig, WalletClientConfig, WalletClientError,
    };

    #[test]
    fn client_config_requires_a_valid_signing_reference() {
        assert_eq!(validate_config(&valid_config()), Ok(()));

        let mut config = valid_config();
        config.local_secret_ref = Some(ProtectedSecretRef {
            value: "  ".to_owned(),
        });
        assert_eq!(
            validate_config(&config),
            Err(WalletClientError::InvalidLocalSecretReference)
        );
    }

    #[test]
    fn client_config_accepts_local_and_public_key_only_signing_modes() {
        assert_eq!(validate_config(&valid_config()), Ok(()));

        let mut config = valid_config();
        config.local_secret_ref = Some(ProtectedSecretRef {
            value: "wallet:validation-wallet:mnemonic".to_owned(),
        });
        assert_eq!(validate_config(&config), Ok(()));
    }

    #[test]
    fn client_config_accepts_a_rotated_signing_key_with_the_original_address() {
        let mut config = valid_config();
        let wallet = derive_wallet(
            "notice tortoise soup strong gun divide offer process salon siren general carry clump left year void clutch tool case burden fix income champion lounge",
            config.network,
        )
        .expect("rotated mnemonic must derive a wallet");
        config.address = crate::TonAddressString::from_address(&wallet.address, config.network);
        config.public_key = wallet.signing_public_key().to_vec();
        let (derived_address, _) = derive_wallet_public_state(&config.public_key, config.network)
            .expect("rotated signing key must derive a different initial state");
        assert_ne!(config.address.as_address(), &derived_address);

        assert_eq!(validate_config(&config), Ok(()));
        config.local_secret_ref = Some(ProtectedSecretRef {
            value: "wallet:validation-wallet:mnemonic".to_owned(),
        });
        assert_eq!(validate_config(&config), Ok(()));
    }

    #[test]
    fn client_config_requires_a_32_byte_public_key() {
        for length in [0, 31, 33] {
            let mut config = valid_config();
            config.public_key = vec![0; length];
            assert_eq!(
                validate_config(&config),
                Err(WalletClientError::InvalidWalletPublicKey)
            );
        }
    }

    fn valid_config() -> WalletClientConfig {
        let public_key = vec![0; 32];
        let (address, _) = derive_wallet_public_state(&public_key, Network::Testnet)
            .expect("test public key must derive a wallet");
        WalletClientConfig {
            record_id: crate::NonEmptyString::try_from("validation-wallet")
                .expect("valid record identifier"),
            address: crate::TonAddressString::try_from(address.to_string())
                .expect("derived TON address must be valid"),
            public_key,
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig::standard(Network::Testnet),
        }
    }
}
