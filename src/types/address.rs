//! Validated TON addresses and internal formatting helpers.

use std::{
    borrow::Cow,
    fmt::{Debug, Display, Formatter},
    str::FromStr,
};

use ton::ton_core::types::TonAddress;
use ton_connect_core::{FriendlyAddress, RawAccountAddress};

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

/// Selects a raw representation or a user-friendly representation with flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TonAddressFormat {
    /// `workchain:64-hex` without display or network flags.
    Raw,
    /// TEP-2 user-friendly Base64 with a checksum and display flags.
    UserFriendly {
        /// Whether senders should use a bounceable internal message.
        bounceable: bool,
        /// Whether mainnet applications must reject the address.
        testnet: bool,
    },
}

/// Parsed TON address identity and any flags carried by its input representation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TonAddressInfo {
    /// Canonical lowercase `workchain:64-hex` account identity.
    pub raw: String,
    /// Signed workchain identifier from the address.
    pub workchain: i32,
    /// Representation and flags used by the parsed input.
    pub format: TonAddressFormat,
}

/// Reports invalid address text or a raw address that cannot use TEP-2 formatting.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    thiserror::Error,
    serde::Serialize,
    serde::Deserialize,
    uniffi::Error,
)]
#[serde(rename_all = "camelCase")]
pub enum TonAddressError {
    /// The input is neither a valid raw address nor a valid TEP-2 friendly address.
    #[error("value must be a valid raw or user-friendly TON internal address")]
    InvalidAddress,
    /// The workchain cannot be encoded by the supported TEP-2 friendly format.
    #[error("TON address workchain cannot be represented in user-friendly format")]
    UnsupportedWorkchain,
}

/// Parses a raw or user-friendly TON address without changing its account identity.
///
/// Friendly input accepts the standard and URL-safe Base64 alphabets and returns
/// the flags protected by its checksum in [`TonAddressFormat::UserFriendly`].
#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "UniFFI free functions use owned strings at the language boundary"
)]
pub fn parse_ton_address(value: String) -> Result<TonAddressInfo, TonAddressError> {
    parse_ton_address_parts(&value).map(|parsed| parsed.info())
}

/// Reports whether `value` is a valid raw or user-friendly TON address.
#[uniffi::export]
#[must_use]
#[allow(
    clippy::needless_pass_by_value,
    reason = "UniFFI free functions use owned strings at the language boundary"
)]
pub fn is_valid_ton_address(value: String) -> bool {
    parse_ton_address_parts(&value).is_ok()
}

/// Converts a TON address to the requested canonical representation.
///
/// Raw output is lowercase `workchain:64-hex`. User-friendly output is
/// unpadded URL-safe TEP-2 Base64. Requested friendly flags replace any flags
/// carried by the input.
#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "UniFFI free functions use owned strings at the language boundary"
)]
pub fn convert_ton_address(
    value: String,
    format: TonAddressFormat,
) -> Result<String, TonAddressError> {
    let parsed = parse_ton_address_parts(&value)?;
    match format {
        TonAddressFormat::Raw => Ok(parsed.raw.to_string()),
        TonAddressFormat::UserFriendly {
            bounceable,
            testnet,
        } => FriendlyAddress::from_raw(parsed.raw, bounceable, testnet)
            .map(|address| address.to_string())
            .map_err(|_| TonAddressError::UnsupportedWorkchain),
    }
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
        let address = parse_ton_address_parts(&value)
            .map_err(|_| TonAddressStringError)?
            .address;
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

struct ParsedTonAddressParts {
    address: TonAddress,
    raw: RawAccountAddress,
    format: TonAddressFormat,
}

impl ParsedTonAddressParts {
    fn info(&self) -> TonAddressInfo {
        TonAddressInfo {
            raw: self.raw.to_string(),
            workchain: self.raw.workchain(),
            format: self.format,
        }
    }
}

fn parse_ton_address_parts(value: &str) -> Result<ParsedTonAddressParts, TonAddressError> {
    if value.contains(':') {
        let address = TonAddress::from_str(value).map_err(|_| TonAddressError::InvalidAddress)?;
        let raw = raw_account_address(&address)?;
        return Ok(ParsedTonAddressParts {
            address,
            raw,
            format: TonAddressFormat::Raw,
        });
    }

    let canonical = canonical_user_friendly(value)?;
    let friendly = FriendlyAddress::try_from(canonical.as_ref())
        .map_err(|_| TonAddressError::InvalidAddress)?;
    let address = TonAddress::from_str(value).map_err(|_| TonAddressError::InvalidAddress)?;
    Ok(ParsedTonAddressParts {
        address,
        raw: friendly.raw_address(),
        format: TonAddressFormat::UserFriendly {
            bounceable: friendly.is_bounceable(),
            testnet: friendly.is_test_only(),
        },
    })
}

fn raw_account_address(address: &TonAddress) -> Result<RawAccountAddress, TonAddressError> {
    let hash = <[u8; 32]>::try_from(address.hash.as_slice())
        .map_err(|_| TonAddressError::InvalidAddress)?;
    Ok(RawAccountAddress::new(address.workchain, hash))
}

fn canonical_user_friendly(value: &str) -> Result<Cow<'_, str>, TonAddressError> {
    let uses_standard_alphabet = value.bytes().any(|byte| matches!(byte, b'+' | b'/'));
    let uses_url_safe_alphabet = value.bytes().any(|byte| matches!(byte, b'-' | b'_'));
    if uses_standard_alphabet && uses_url_safe_alphabet {
        return Err(TonAddressError::InvalidAddress);
    }
    if uses_standard_alphabet {
        Ok(Cow::Owned(
            value
                .chars()
                .map(|character| match character {
                    '+' => '-',
                    '/' => '_',
                    other => other,
                })
                .collect(),
        ))
    } else {
        Ok(Cow::Borrowed(value))
    }
}

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
    use std::str::FromStr;

    use ton::ton_core::types::TonAddress;
    use uniffi::{Lift, Lower};

    use super::{
        TonAddressError, TonAddressExt as _, TonAddressFormat, TonAddressInfo, TonAddressString,
        convert_ton_address, is_valid_ton_address, parse_ton_address,
    };
    use crate::Network;

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

    #[test]
    fn conversions_and_formatters_preserve_the_selected_boundary_text() {
        let address = TonAddress::from_str(RAW_ADDRESS).expect("raw TON address must parse");
        let testnet = TonAddressString::from_address(&address, Network::Testnet);
        let mainnet = TonAddressString::from_address(&address, Network::Mainnet);

        assert_eq!(testnet.as_str(), address.to_user_friendly(Network::Testnet));
        assert_eq!(mainnet.as_str(), address.to_user_friendly(Network::Mainnet));
        assert_ne!(testnet.as_str(), mainnet.as_str());
        assert_eq!(AsRef::<str>::as_ref(&testnet), testnet.as_str());
        assert_eq!(testnet.to_string(), testnet.as_str());
        assert_eq!(
            format!("{testnet:?}"),
            format!("TonAddressString({:?})", testnet.as_str())
        );

        let expected = mainnet.as_str().to_owned();
        assert_eq!(mainnet.clone().into_string(), expected);
        assert_eq!(String::from(mainnet), expected);
    }

    #[test]
    fn parses_raw_and_both_friendly_base64_alphabets() -> Result<(), TonAddressError> {
        let raw = "0:e4d954ef9f4e1250a26b5bbad76a1cdd17cfd08babad6f4c23e372270aef6f76";
        let url_safe = "EQDk2VTvn04SUKJrW7rXahzdF8_Qi6utb0wj43InCu9vdjrR";
        let standard = "EQDk2VTvn04SUKJrW7rXahzdF8/Qi6utb0wj43InCu9vdjrR";

        assert_eq!(
            parse_ton_address(raw.to_owned())?,
            TonAddressInfo {
                raw: raw.to_owned(),
                workchain: 0,
                format: TonAddressFormat::Raw,
            }
        );
        for friendly in [url_safe, standard] {
            assert_eq!(
                parse_ton_address(friendly.to_owned())?,
                TonAddressInfo {
                    raw: raw.to_owned(),
                    workchain: 0,
                    format: TonAddressFormat::UserFriendly {
                        bounceable: true,
                        testnet: false,
                    },
                }
            );
            assert_eq!(
                convert_ton_address(friendly.to_owned(), TonAddressFormat::Raw)?,
                raw
            );
        }
        Ok(())
    }

    #[test]
    fn formats_every_tep_2_flag_combination() -> Result<(), TonAddressError> {
        let raw = "0:ca6e321c7cce9ecedf0a8ca2492ec8592494aa5fb5ce0387dff96ef6af982a3e";
        for (friendly, bounceable, testnet) in [
            (
                "EQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPrHF",
                true,
                false,
            ),
            (
                "UQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPuwA",
                false,
                false,
            ),
            (
                "kQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPgpP",
                true,
                true,
            ),
            (
                "0QDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPleK",
                false,
                true,
            ),
        ] {
            assert_eq!(
                convert_ton_address(
                    raw.to_owned(),
                    TonAddressFormat::UserFriendly {
                        bounceable,
                        testnet,
                    }
                )?,
                friendly
            );
            let parsed = parse_ton_address(friendly.to_owned())?;
            assert_eq!(
                parsed.format,
                TonAddressFormat::UserFriendly {
                    bounceable,
                    testnet,
                }
            );
        }
        Ok(())
    }

    #[test]
    fn validation_rejects_bad_flags_checksums_and_mixed_alphabets() {
        let corrupt_checksum = "EQDk2VTvn04SUKJrW7rXahzdF8_Qi6utb0wj43InCu9vdjra";
        let invalid_friendly = "UQEzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzMzM2SU";
        let mixed_alphabet = FRIENDLY_ADDRESS.replacen('_', "/", 1);

        for invalid in [
            "",
            "not-an-address",
            corrupt_checksum,
            invalid_friendly,
            mixed_alphabet.as_str(),
        ] {
            assert!(!is_valid_ton_address(invalid.to_owned()));
            assert_eq!(
                parse_ton_address(invalid.to_owned()),
                Err(TonAddressError::InvalidAddress)
            );
        }
    }

    #[test]
    fn friendly_format_rejects_an_unsupported_workchain() {
        let raw = format!("1:{}", "11".repeat(32));
        assert!(is_valid_ton_address(raw.clone()));
        assert_eq!(
            convert_ton_address(
                raw,
                TonAddressFormat::UserFriendly {
                    bounceable: false,
                    testnet: false,
                }
            ),
            Err(TonAddressError::UnsupportedWorkchain)
        );
    }
}
