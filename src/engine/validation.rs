//! Validation of public configuration and send requests.

use std::str::FromStr;

use ton::ton_core::types::TonAddress;
use url::{Host, Url};

use crate::MAX_PROVIDER_REQUEST_TIMEOUT_MS;
use crate::types::parse_positive_decimal;
use crate::wallet::crypto::derive_v5r1_public_state;
use crate::{SendAmount, SendPreviewRequest, SendRequest, WalletClientConfig, WalletClientError};

pub(super) fn validate_config(config: &WalletClientConfig) -> Result<(), WalletClientError> {
    if config.record_id.trim().is_empty()
        || config.send_validity_seconds == 0
        || config.providers.request_timeout_ms == 0
        || config.providers.request_timeout_ms > MAX_PROVIDER_REQUEST_TIMEOUT_MS
        || config
            .local_secret_ref
            .as_ref()
            .is_some_and(|secret_ref| secret_ref.value.trim().is_empty())
    {
        return Err(WalletClientError::InvalidConfig);
    }

    let configured_address = config.parsed_address()?;
    let (derived_address, _) = derive_v5r1_public_state(&config.public_key, config.network)
        .map_err(|_| WalletClientError::InvalidConfig)?;
    if derived_address != configured_address {
        return Err(WalletClientError::InvalidConfig);
    }
    validate_provider_url(&config.providers.toncenter_base_url)?;
    Ok(())
}

pub(super) fn validate_send(request: &SendRequest) -> Result<(), WalletClientError> {
    if request.operation_id.trim().is_empty()
        || request.destination.trim().is_empty()
        || TonAddress::from_str(&request.destination).is_err()
        || !valid_send_amount(&request.amount)
    {
        return Err(WalletClientError::InvalidSendRequest);
    }

    Ok(())
}

pub(super) fn validate_send_preview(request: &SendPreviewRequest) -> Result<(), WalletClientError> {
    if request.destination.trim().is_empty()
        || TonAddress::from_str(&request.destination).is_err()
        || !valid_send_amount(&request.amount)
    {
        return Err(WalletClientError::InvalidSendRequest);
    }

    Ok(())
}

fn valid_send_amount(amount: &SendAmount) -> bool {
    match amount {
        SendAmount::Exact { nanograms } => parse_positive_decimal(nanograms).is_some(),
        SendAmount::All => true,
    }
}

impl WalletClientConfig {
    pub(super) fn parsed_address(&self) -> Result<TonAddress, WalletClientError> {
        TonAddress::from_str(&self.address).map_err(|_| WalletClientError::InvalidConfig)
    }
}

fn validate_provider_url(value: &str) -> Result<(), WalletClientError> {
    let url = Url::parse(value).map_err(|_| WalletClientError::InvalidConfig)?;
    let host = url.host().ok_or(WalletClientError::InvalidConfig)?;
    let path = url.path().trim_end_matches('/');
    let secure = url.scheme() == "https";
    let loopback_http = url.scheme() == "http"
        && match host {
            Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
            Host::Ipv4(address) => address.is_loopback(),
            Host::Ipv6(address) => address.is_loopback(),
        };
    let api_specific_path =
        path.ends_with("/api/v2") || path.ends_with("/api/v3") || path.ends_with("/api/emulate/v1");

    if (!secure && !loopback_http)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || api_specific_path
    {
        return Err(WalletClientError::InvalidConfig);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_config, validate_provider_url};
    use crate::wallet::crypto::derive_v5r1_public_state;
    use crate::{
        MAX_PROVIDER_REQUEST_TIMEOUT_MS, Network, ProtectedSecretRef, ProviderConfig,
        WalletClientConfig, WalletClientError,
    };

    #[test]
    fn provider_url_accepts_secure_and_loopback_transports() {
        assert_eq!(validate_provider_url("https://example.com"), Ok(()));
        assert_eq!(
            validate_provider_url("https://example.com/toncenter"),
            Ok(())
        );
        assert_eq!(validate_provider_url("http://127.0.0.1:8080"), Ok(()));
        assert_eq!(validate_provider_url("http://[::1]:8080"), Ok(()));
        assert_eq!(validate_provider_url("http://localhost:8080"), Ok(()));
    }

    #[test]
    fn provider_url_rejects_insecure_remote_transports() {
        assert_eq!(
            validate_provider_url("http://example.com"),
            Err(WalletClientError::InvalidConfig)
        );
        assert_eq!(
            validate_provider_url("http://192.168.1.10:8080"),
            Err(WalletClientError::InvalidConfig)
        );
    }

    #[test]
    fn provider_url_rejects_api_specific_paths_and_query_parameters() {
        for value in [
            "https://example.com/api/v2",
            "https://example.com/custom/api/v3/",
            "https://example.com/api/emulate/v1",
            "https://example.com?api_key=secret",
        ] {
            assert_eq!(
                validate_provider_url(value),
                Err(WalletClientError::InvalidConfig),
                "provider base unexpectedly accepted {value}"
            );
        }
    }

    #[test]
    fn config_rejects_provider_timeouts_outside_the_supported_range() {
        let mut config = valid_config();
        config.providers.request_timeout_ms = 0;
        assert_eq!(
            validate_config(&config),
            Err(WalletClientError::InvalidConfig)
        );

        config.providers.request_timeout_ms = MAX_PROVIDER_REQUEST_TIMEOUT_MS + 1;
        assert_eq!(
            validate_config(&config),
            Err(WalletClientError::InvalidConfig)
        );

        config.providers.request_timeout_ms = MAX_PROVIDER_REQUEST_TIMEOUT_MS;
        assert_eq!(validate_config(&config), Ok(()));
    }

    #[test]
    fn client_config_requires_application_identity_and_send_validity() {
        assert_eq!(validate_config(&valid_config()), Ok(()));

        let mut config = valid_config();
        config.record_id = "   ".to_owned();
        assert_eq!(
            validate_config(&config),
            Err(WalletClientError::InvalidConfig)
        );

        let mut config = valid_config();
        config.send_validity_seconds = 0;
        assert_eq!(
            validate_config(&config),
            Err(WalletClientError::InvalidConfig)
        );

        let mut config = valid_config();
        config.local_secret_ref = Some(ProtectedSecretRef {
            value: "  ".to_owned(),
        });
        assert_eq!(
            validate_config(&config),
            Err(WalletClientError::InvalidConfig)
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
    fn client_config_binds_the_public_key_to_the_source_address() {
        let mut config = valid_config();
        config.public_key[0] = 1;
        assert_eq!(
            validate_config(&config),
            Err(WalletClientError::InvalidConfig)
        );

        let mut config = valid_config();
        config.public_key.pop();
        assert_eq!(
            validate_config(&config),
            Err(WalletClientError::InvalidConfig)
        );
    }

    fn valid_config() -> WalletClientConfig {
        let public_key = vec![0; 32];
        let (address, _) = derive_v5r1_public_state(&public_key, Network::Testnet)
            .expect("test public key must derive a wallet");
        WalletClientConfig {
            record_id: "validation-wallet".to_owned(),
            address: address.to_string(),
            public_key,
            local_secret_ref: None,
            network: Network::Testnet,
            send_validity_seconds: 300,
            resolution_margin_seconds: 60,
            providers: ProviderConfig::standard(Network::Testnet),
        }
    }
}
