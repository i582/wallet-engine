//! Validation of public configuration and send requests.

use std::str::FromStr;

use ton::ton_core::types::TonAddress;
use url::{Host, Url};

use crate::types::parse_positive_decimal;
use crate::{SendRequest, WalletClientConfig, WalletClientError};

pub(super) fn validate_config(config: &WalletClientConfig) -> Result<(), WalletClientError> {
    if config.record_id.trim().is_empty() || config.send_validity_seconds == 0 {
        return Err(WalletClientError::InvalidConfig);
    }

    config.parsed_address()?;
    validate_provider_url(&config.providers.toncenter_base_url)?;
    Ok(())
}

pub(super) fn validate_send(request: &SendRequest) -> Result<(), WalletClientError> {
    if request.operation_id.trim().is_empty()
        || request.destination.trim().is_empty()
        || TonAddress::from_str(&request.destination).is_err()
        || request.secret_ref.value.trim().is_empty()
        || parse_positive_decimal(&request.amount_nanograms).is_none()
    {
        return Err(WalletClientError::InvalidSendRequest);
    }

    Ok(())
}

impl WalletClientConfig {
    pub(super) fn parsed_address(&self) -> Result<TonAddress, WalletClientError> {
        TonAddress::from_str(&self.address).map_err(|_| WalletClientError::InvalidConfig)
    }
}

fn validate_provider_url(value: &str) -> Result<(), WalletClientError> {
    let url = Url::parse(value).map_err(|_| WalletClientError::InvalidConfig)?;
    let host = url.host().ok_or(WalletClientError::InvalidConfig)?;
    let secure = url.scheme() == "https";
    let loopback_http = url.scheme() == "http"
        && match host {
            Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
            Host::Ipv4(address) => address.is_loopback(),
            Host::Ipv6(address) => address.is_loopback(),
        };

    if (!secure && !loopback_http)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(WalletClientError::InvalidConfig);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_config, validate_provider_url};
    use crate::{Network, ProviderConfig, WalletClientConfig, WalletClientError};

    const ADDRESS: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn provider_url_accepts_secure_and_loopback_transports() {
        assert_eq!(validate_provider_url("https://example.com/api/v2"), Ok(()));
        assert_eq!(
            validate_provider_url("http://127.0.0.1:8080/api/v2"),
            Ok(())
        );
        assert_eq!(validate_provider_url("http://[::1]:8080/api/v2"), Ok(()));
        assert_eq!(
            validate_provider_url("http://localhost:8080/api/v2"),
            Ok(())
        );
    }

    #[test]
    fn provider_url_rejects_insecure_remote_transports() {
        assert_eq!(
            validate_provider_url("http://example.com/api/v2"),
            Err(WalletClientError::InvalidConfig)
        );
        assert_eq!(
            validate_provider_url("http://192.168.1.10:8080/api/v2"),
            Err(WalletClientError::InvalidConfig)
        );
    }

    #[test]
    fn client_config_requires_application_identity_and_send_validity() {
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
    }

    fn valid_config() -> WalletClientConfig {
        WalletClientConfig {
            record_id: "validation-wallet".to_owned(),
            address: ADDRESS.to_owned(),
            network: Network::Testnet,
            send_validity_seconds: 300,
            providers: ProviderConfig::standard(Network::Testnet),
        }
    }
}
