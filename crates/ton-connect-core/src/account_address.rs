//! TON Connect account address accepting raw or TEP-2 friendly wire forms.

use std::{fmt, str::FromStr as _};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

use crate::{FriendlyAddress, RawAccountAddress};

/// Validated TON account address where the protocol permits raw or friendly form.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum AccountAddress {
    /// Canonical `workchain:64-hex` account identity.
    Raw(RawAccountAddress),
    /// TEP-2 base64url address retaining display flags.
    Friendly(FriendlyAddress),
}

impl AccountAddress {
    /// Returns the address identity without friendly display flags.
    #[must_use]
    pub const fn raw_address(&self) -> RawAccountAddress {
        match self {
            Self::Raw(address) => *address,
            Self::Friendly(address) => address.raw_address(),
        }
    }
}

impl TryFrom<&str> for AccountAddress {
    type Error = AccountAddressError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if let Ok(address) = RawAccountAddress::from_str(value) {
            Ok(Self::Raw(address))
        } else {
            FriendlyAddress::try_from(value)
                .map(Self::Friendly)
                .map_err(|_| AccountAddressError)
        }
    }
}

impl TryFrom<String> for AccountAddress {
    type Error = AccountAddressError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl fmt::Display for AccountAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raw(address) => fmt::Display::fmt(address, formatter),
            Self::Friendly(address) => fmt::Display::fmt(address, formatter),
        }
    }
}

impl Serialize for AccountAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for AccountAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
    }
}

/// A value is neither a canonical raw nor a valid friendly TON address.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("value must be a canonical raw or TEP-2 friendly TON address")]
pub struct AccountAddressError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_raw_and_friendly_but_rejects_placeholders() {
        let raw = "0:1111111111111111111111111111111111111111111111111111111111111111";
        let friendly = "Ef8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADAU";

        assert!(matches!(
            AccountAddress::try_from(raw),
            Ok(AccountAddress::Raw(_))
        ));
        assert!(matches!(
            AccountAddress::try_from(friendly),
            Ok(AccountAddress::Friendly(_))
        ));
        for invalid in ["", "EQ", "0:1234", "not-an-address"] {
            assert!(AccountAddress::try_from(invalid).is_err());
        }
    }
}
