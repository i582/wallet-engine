//! Validated TON addresses and internal formatting helpers.

use std::{
    fmt::{Debug, Display, Formatter},
    str::FromStr,
};

use ton::ton_core::types::TonAddress;

use crate::Network;

/// A validated raw or user-friendly TON internal address.
///
/// The portable representation remains an ordinary string. Lifting through
/// `Serde` or `UniFFI` validates the address once and stores the parsed
/// [`TonAddress`] for later message construction. The original spelling is
/// retained so boundary round trips do not discard friendly-address metadata.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TonAddressString {
    value: String,
    address: TonAddress,
}

impl TonAddressString {
    /// Creates a validated boundary address from an already parsed TON address.
    #[must_use]
    pub(crate) fn from_address(address: &TonAddress, network: Network) -> Self {
        Self {
            value: address.to_base64(network == Network::Mainnet, false, true),
            address: address.clone(),
        }
    }

    /// Borrows the original validated boundary representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Borrows the parsed address used for TON message construction.
    #[must_use]
    pub const fn as_address(&self) -> &TonAddress {
        &self.address
    }

    /// Consumes the wrapper and returns the original boundary representation.
    #[must_use]
    pub fn into_string(self) -> String {
        self.value
    }

    /// Consumes the wrapper and returns the parsed TON address.
    #[must_use]
    pub fn into_address(self) -> TonAddress {
        self.address
    }
}

impl TryFrom<String> for TonAddressString {
    type Error = TonAddressStringError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let address = TonAddress::from_str(&value).map_err(|_| TonAddressStringError)?;
        Ok(Self { value, address })
    }
}

impl TryFrom<&str> for TonAddressString {
    type Error = TonAddressStringError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl From<TonAddressString> for String {
    fn from(value: TonAddressString) -> Self {
        value.value
    }
}

impl AsRef<str> for TonAddressString {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Debug for TonAddressString {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("TonAddressString")
            .field(&self.value)
            .finish()
    }
}

impl Display for TonAddressString {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl PartialEq for TonAddressString {
    fn eq(&self, other: &Self) -> bool {
        self.address == other.address
    }
}

impl Eq for TonAddressString {}

/// Reports a malformed raw or user-friendly TON internal address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("value must be a valid TON internal address")]
pub struct TonAddressStringError;

uniffi::custom_type!(TonAddressString, String);

pub(crate) trait TonAddressExt {
    /// Encodes the address for display and public API responses.
    ///
    /// The result is non-bounceable and URL-safe. Its network flag matches the
    /// wallet configuration.
    fn to_user_friendly(&self, network: Network) -> String;
}

impl TonAddressExt for TonAddress {
    fn to_user_friendly(&self, network: Network) -> String {
        self.to_base64(network == Network::Mainnet, false, true)
    }
}

#[cfg(test)]
mod tests {
    use uniffi::{Lift, Lower};

    use super::TonAddressString;

    const RAW_ADDRESS: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";
    const FRIENDLY_ADDRESS: &str = "0QA_6fh0aRAkD7n1MNfAUx8TvyCUw2iTQfzVM-0isMze2anN";

    #[test]
    fn accepts_raw_and_user_friendly_addresses_without_changing_the_boundary_text() {
        for original in [RAW_ADDRESS, FRIENDLY_ADDRESS] {
            let value = TonAddressString::try_from(original).expect("TON address must be valid");
            assert_eq!(value.as_str(), original);

            let encoded = serde_json::to_string(&value).expect("address must serialize");
            assert_eq!(
                serde_json::from_str::<TonAddressString>(&encoded)
                    .expect("address must deserialize"),
                value
            );
        }
    }

    #[test]
    fn rejects_malformed_or_padded_addresses_at_every_boundary() {
        for value in ["", " ", "not-an-address", RAW_ADDRESS.trim_end_matches('1')] {
            assert!(
                TonAddressString::try_from(value).is_err(),
                "accepted {value:?}"
            );
        }

        let padded = format!(" {RAW_ADDRESS}");
        assert!(serde_json::from_str::<TonAddressString>(&format!("{padded:?}")).is_err());

        let ffi_value = <String as Lower<crate::UniFfiTag>>::lower("invalid".to_owned());
        let result = <TonAddressString as Lift<crate::UniFfiTag>>::try_lift(ffi_value);
        assert!(result.is_err(), "UniFFI accepted an invalid TON address");
    }
}
