//! Validation of public configuration and send requests.

use std::str::FromStr;

use ton::ton_core::types::TonAddress;
use url::Url;

use crate::{SendRequest, WalletClientConfig, WalletClientError};

pub(super) fn validate_config(config: &WalletClientConfig) -> Result<(), WalletClientError> {
    if config.record_id.trim().is_empty() || config.send_validity_seconds == 0 {
        return Err(WalletClientError::InvalidConfig);
    }

    config.parsed_address()?;
    validate_https_url(&config.providers.toncenter_base_url)?;

    match (
        &config.providers.toncenter_credential,
        &config.providers.toncenter_credential_origin,
    ) {
        (Some(_), Some(origin)) => {
            validate_https_origin(origin)?;
            if effective_origin(&config.providers.toncenter_base_url)? != *origin {
                return Err(WalletClientError::InvalidConfig);
            }
            Ok(())
        }
        (None, None) => Ok(()),
        _ => Err(WalletClientError::InvalidConfig),
    }
}

pub(super) fn validate_send(request: &SendRequest) -> Result<(), WalletClientError> {
    if request.operation_id.trim().is_empty()
        || request.destination.trim().is_empty()
        || request.secret_ref.value.trim().is_empty()
        || request.amount_nanograms.is_empty()
        || !request
            .amount_nanograms
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        || request.amount_nanograms.bytes().all(|byte| byte == b'0')
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

fn effective_origin(value: &str) -> Result<String, WalletClientError> {
    let url = Url::parse(value).map_err(|_| WalletClientError::InvalidConfig)?;
    let host = url.host_str().ok_or(WalletClientError::InvalidConfig)?;
    let port = url
        .port_or_known_default()
        .ok_or(WalletClientError::InvalidConfig)?;

    Ok(format!("{}://{host}:{port}", url.scheme()))
}

fn validate_https_url(value: &str) -> Result<(), WalletClientError> {
    let url = Url::parse(value).map_err(|_| WalletClientError::InvalidConfig)?;
    if url.scheme() != "https" || url.host_str().is_none() || url.fragment().is_some() {
        return Err(WalletClientError::InvalidConfig);
    }

    Ok(())
}

fn validate_https_origin(value: &str) -> Result<(), WalletClientError> {
    let url = Url::parse(value).map_err(|_| WalletClientError::InvalidConfig)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || url.port_or_known_default() != Some(443)
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(WalletClientError::InvalidConfig);
    }

    Ok(())
}
