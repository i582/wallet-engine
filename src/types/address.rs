//! Internal TON address formatting helpers.

use ton::ton_core::types::TonAddress;

use crate::Network;

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

pub(crate) mod raw_serde {
    use std::str::FromStr;

    use serde::{Deserialize as _, Deserializer, Serializer, de::Error as _};
    use ton::ton_core::types::TonAddress;

    pub(crate) fn serialize<S>(address: &TonAddress, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&address.to_hex())
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<TonAddress, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        TonAddress::from_str(&value).map_err(D::Error::custom)
    }
}
