//! Strict TEP-2 user-friendly TON address representation.

use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use crc::{CRC_16_XMODEM, Crc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

use crate::RawAccountAddress;

const FRIENDLY_ADDRESS_BYTES: usize = 36;
const BOUNCEABLE_TAG: u8 = 0x11;
const NON_BOUNCEABLE_TAG: u8 = 0x51;
const TEST_ONLY_TAG: u8 = 0x80;

/// Validated TEP-2 base64url address with bounce and test-only flags intact.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FriendlyAddress {
    encoded: String,
    raw: RawAccountAddress,
    bounceable: bool,
    test_only: bool,
}

impl FriendlyAddress {
    /// Returns the exact canonical base64url wire representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.encoded
    }

    /// Returns the account identity without display flags.
    #[must_use]
    pub const fn raw_address(&self) -> RawAccountAddress {
        self.raw
    }

    /// Returns the bounce flag that must be used for the outgoing message.
    #[must_use]
    pub const fn is_bounceable(&self) -> bool {
        self.bounceable
    }

    /// Reports whether the address carries the test-only flag.
    #[must_use]
    pub const fn is_test_only(&self) -> bool {
        self.test_only
    }
}

impl TryFrom<&str> for FriendlyAddress {
    type Error = FriendlyAddressError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.contains('=') {
            return Err(FriendlyAddressError);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| FriendlyAddressError)?;
        if bytes.len() != FRIENDLY_ADDRESS_BYTES || URL_SAFE_NO_PAD.encode(&bytes) != value {
            return Err(FriendlyAddressError);
        }

        let payload = bytes.get(..34).ok_or(FriendlyAddressError)?;
        let checksum = bytes.get(34..).ok_or(FriendlyAddressError)?;
        if Crc::<u16>::new(&CRC_16_XMODEM)
            .checksum(payload)
            .to_be_bytes()
            != checksum
        {
            return Err(FriendlyAddressError);
        }

        let tag = *bytes.first().ok_or(FriendlyAddressError)?;
        let test_only = tag & TEST_ONLY_TAG != 0;
        let bounceable = match tag & !TEST_ONLY_TAG {
            BOUNCEABLE_TAG => true,
            NON_BOUNCEABLE_TAG => false,
            _ => return Err(FriendlyAddressError),
        };
        let workchain = i8::from_be_bytes([*bytes.get(1).ok_or(FriendlyAddressError)?]);
        let hash = <[u8; 32]>::try_from(bytes.get(2..34).ok_or(FriendlyAddressError)?)
            .map_err(|_| FriendlyAddressError)?;
        Ok(Self {
            encoded: value.to_owned(),
            raw: RawAccountAddress::new(i32::from(workchain), hash),
            bounceable,
            test_only,
        })
    }
}

impl TryFrom<String> for FriendlyAddress {
    type Error = FriendlyAddressError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl fmt::Debug for FriendlyAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for FriendlyAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for FriendlyAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FriendlyAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
    }
}

/// A value is not a canonical TEP-2 user-friendly address.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("address must be a canonical unpadded TEP-2 base64url value")]
pub struct FriendlyAddressError;

#[cfg(test)]
mod tests {
    use super::*;

    const MASTERCHAIN_ZERO: &str = "Ef8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADAU";
    const TESTNET_NON_BOUNCEABLE: &str = "0QA_6fh0aRAkD7n1MNfAUx8TvyCUw2iTQfzVM-0isMze2anN";

    #[test]
    fn preserves_workchain_and_display_flags() -> Result<(), Box<dyn std::error::Error>> {
        let masterchain = FriendlyAddress::try_from(MASTERCHAIN_ZERO)?;
        assert_eq!(
            masterchain.raw_address(),
            RawAccountAddress::new(-1, [0_u8; 32])
        );
        assert!(masterchain.is_bounceable());
        assert!(!masterchain.is_test_only());

        let testnet = FriendlyAddress::try_from(TESTNET_NON_BOUNCEABLE)?;
        assert_eq!(testnet.raw_address().workchain(), 0);
        assert!(!testnet.is_bounceable());
        assert!(testnet.is_test_only());
        let encoded = serde_json::to_string(&testnet)?;
        assert_eq!(serde_json::from_str::<FriendlyAddress>(&encoded)?, testnet);
        Ok(())
    }

    #[test]
    fn rejects_raw_padded_standard_and_corrupt_addresses() {
        let raw = "0:1111111111111111111111111111111111111111111111111111111111111111";
        let padded = format!("{MASTERCHAIN_ZERO}=");
        let standard = TESTNET_NON_BOUNCEABLE.replace('-', "+").replace('_', "/");
        let mut corrupt = MASTERCHAIN_ZERO.to_owned();
        let _ = corrupt.pop();
        corrupt.push('A');

        for value in [raw, padded.as_str(), standard.as_str(), corrupt.as_str()] {
            assert!(
                FriendlyAddress::try_from(value).is_err(),
                "accepted {value}"
            );
        }
    }
}
