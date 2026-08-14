//! Validation of public configuration and send requests.

use std::str::FromStr;

use ton::ton_core::types::TonAddress;
use url::Url;

use crate::types::parse_positive_decimal;
use crate::{SendRequest, WalletClientConfig, WalletClientError};

pub(super) fn validate_config(config: &WalletClientConfig) -> Result<(), WalletClientError> {
    if config.record_id.trim().is_empty() || config.send_validity_seconds == 0 {
        return Err(WalletClientError::InvalidConfig);
    }

    config.parsed_address()?;
    validate_https_url(&config.providers.toncenter_base_url)?;
    Ok(())
}

pub(super) fn validate_send(request: &SendRequest) -> Result<(), WalletClientError> {
    if request.operation_id.trim().is_empty()
        || request.destination.trim().is_empty()
        || TonAddress::from_str(&request.destination).is_err()
        || request.secret_ref.value.trim().is_empty()
        || parse_positive_decimal(&request.amount_nanograms).is_none()
    {
        return Err(WalletClientError::InvalidConfig);
    }

    Ok(())
}

impl WalletClientConfig {
    pub(super) fn parsed_address(&self) -> Result<TonAddress, WalletClientError> {
        TonAddress::from_str(&self.address).map_err(|_| WalletClientError::InvalidConfig)
    }
}

fn validate_https_url(value: &str) -> Result<(), WalletClientError> {
    let url = Url::parse(value).map_err(|_| WalletClientError::InvalidConfig)?;
    if url.scheme() != "https" || url.host_str().is_none() || url.fragment().is_some() {
        return Err(WalletClientError::InvalidConfig);
    }

    Ok(())
}
