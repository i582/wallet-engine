//! Pure parsing for standard `ton://transfer/` deep links.

use ton_connect_core::RawAccountAddress;
use url::Url;

use crate::{Boc, SendExpiration, TonAddressString, UnsignedDecimalString};

/// The asset selected by a parsed TON transfer link.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TonTransferAsset {
    /// Transfer Gram.
    Gram,
    /// Transfer a jetton identified by its master contract.
    Jetton {
        /// The jetton master contract address.
        master: TonAddressString,
    },
}

/// The optional payload carried by a parsed TON transfer link.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TonTransferPayload {
    /// The link contains no message payload.
    None,
    /// The link contains a plaintext comment, which may intentionally be empty.
    Text {
        /// The decoded UTF-8 comment.
        text: String,
    },
    /// The link contains a validated single-root BOC.
    Boc {
        /// The complete payload cell BOC.
        boc: Boc,
    },
}

/// A syntax-validated transfer invoice parsed from a `ton://transfer/` link.
///
/// This value is not an executable send request. Network policy, expiration,
/// asset resolution, bounce selection, emulation, and user approval remain the
/// responsibility of the later admission and send stages.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ParsedTonTransferLink {
    /// The Gram recipient or the jetton owner's account address.
    pub recipient: TonAddressString,
    /// The asset requested by the link.
    pub asset: TonTransferAsset,
    /// The exact amount in the asset's elementary units, when supplied.
    pub amount: Option<UnsignedDecimalString>,
    /// The optional decoded text or binary payload.
    pub payload: TonTransferPayload,
    /// The requested Unix expiration, or the engine-default policy when absent.
    pub expiration: SendExpiration,
}

/// Reports why a `ton://transfer/` link could not be parsed.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    thiserror::Error,
    serde::Serialize,
    serde::Deserialize,
    uniffi::Error,
)]
#[serde(rename_all = "camelCase")]
pub enum TonTransferLinkError {
    /// The URI belongs to a different scheme or command.
    #[error("URI is not a ton://transfer link")]
    NotTonTransfer,
    /// The URI structure or encoding is invalid.
    #[error("ton transfer URI is malformed")]
    InvalidUrl,
    /// The transfer command has no recipient path segment.
    #[error("ton transfer URI has no recipient")]
    MissingRecipient,
    /// The recipient is not a valid TON account address.
    #[error("ton transfer recipient is invalid")]
    InvalidRecipient,
    /// A singleton baseline query parameter occurs more than once.
    #[error("ton transfer parameter `{name}` occurs more than once")]
    DuplicateParameter {
        /// The decoded duplicate parameter name.
        name: String,
    },
    /// The strict baseline does not recognize a query parameter.
    #[error("ton transfer parameter `{name}` is unsupported")]
    UnsupportedParameter {
        /// The decoded unsupported parameter name.
        name: String,
    },
    /// `amount` is not a canonical unsigned decimal integer.
    #[error("ton transfer amount is invalid")]
    InvalidAmount,
    /// `exp` is not a canonical unsigned `u64` Unix timestamp.
    #[error("ton transfer expiration is invalid")]
    InvalidExpiration,
    /// `jetton` is not a valid TON account address.
    #[error("ton transfer jetton master is invalid")]
    InvalidJettonMaster,
    /// `bin` is not standard Base64 containing one valid BOC root.
    #[error("ton transfer binary payload is invalid")]
    InvalidBinaryPayload,
    /// `text` and `bin` cannot both define the payload.
    #[error("ton transfer contains conflicting text and binary payloads")]
    ConflictingPayloads,
    /// Strict baseline mode does not define the meaning of `bin` for a jetton.
    #[error("ton transfer cannot combine a jetton with a binary payload")]
    BinaryJettonConflict,
}

/// Parses the strict baseline `ton://transfer/` format without reading chain or clock state.
///
/// Query names are case-sensitive. Percent escapes are decoded exactly once,
/// and literal `+` characters remain plus signs instead of becoming spaces.
#[uniffi::export]
#[allow(
    clippy::needless_pass_by_value,
    reason = "UniFFI free functions use owned strings at the language boundary"
)]
pub fn parse_ton_transfer_link(
    value: String,
) -> Result<ParsedTonTransferLink, TonTransferLinkError> {
    parse_ton_transfer_link_ref(&value)
}

fn parse_ton_transfer_link_ref(value: &str) -> Result<ParsedTonTransferLink, TonTransferLinkError> {
    if value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(TonTransferLinkError::InvalidUrl);
    }

    let url = Url::parse(value).map_err(|_| TonTransferLinkError::InvalidUrl)?;

    if !url.scheme().eq_ignore_ascii_case("ton")
        || !url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("transfer"))
    {
        return Err(TonTransferLinkError::NotTonTransfer);
    }

    if !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return Err(TonTransferLinkError::InvalidUrl);
    }

    let (raw_path, raw_query) = raw_transfer_components(value)?;
    if raw_path.is_empty() || raw_path == "/" {
        return Err(TonTransferLinkError::MissingRecipient);
    }
    let raw_recipient = raw_path
        .strip_prefix('/')
        .filter(|path| !path.is_empty() && !path.contains('/'))
        .ok_or(TonTransferLinkError::InvalidUrl)?;
    let recipient_text = percent_decode_once(raw_recipient)?;
    if !is_canonical_raw_address(&recipient_text) {
        return Err(TonTransferLinkError::InvalidRecipient);
    }
    let recipient = TonAddressString::try_from(recipient_text)
        .map_err(|_| TonTransferLinkError::InvalidRecipient)?;

    let parameters = parse_parameters(raw_query)?;
    let amount = parameters
        .amount
        .map(UnsignedDecimalString::try_from)
        .transpose()
        .map_err(|_| TonTransferLinkError::InvalidAmount)?;
    let expiration = parse_expiration(parameters.exp)?;
    let jetton = parameters
        .jetton
        .map(|value| {
            if !is_canonical_raw_address(&value) {
                return Err(TonTransferLinkError::InvalidJettonMaster);
            }
            TonAddressString::try_from(value).map_err(|_| TonTransferLinkError::InvalidJettonMaster)
        })
        .transpose()?;
    let binary = parameters
        .binary
        .map(Boc::try_from)
        .transpose()
        .map_err(|_| TonTransferLinkError::InvalidBinaryPayload)?;

    if parameters.text.is_some() && binary.is_some() {
        return Err(TonTransferLinkError::ConflictingPayloads);
    }
    if jetton.is_some() && binary.is_some() {
        return Err(TonTransferLinkError::BinaryJettonConflict);
    }

    let asset = match jetton {
        Some(master) => TonTransferAsset::Jetton { master },
        None => TonTransferAsset::Gram,
    };
    let payload = match (parameters.text, binary) {
        (_, Some(boc)) => TonTransferPayload::Boc { boc },
        (Some(text), None) => TonTransferPayload::Text { text },
        (None, None) => TonTransferPayload::None,
    };

    Ok(ParsedTonTransferLink {
        recipient,
        asset,
        amount,
        payload,
        expiration,
    })
}

fn raw_transfer_components(value: &str) -> Result<(&str, Option<&str>), TonTransferLinkError> {
    let (scheme, after_scheme) = value
        .split_once(':')
        .ok_or(TonTransferLinkError::InvalidUrl)?;
    if !scheme.eq_ignore_ascii_case("ton") {
        return Err(TonTransferLinkError::InvalidUrl);
    }

    let authority_and_rest = after_scheme
        .strip_prefix("//")
        .ok_or(TonTransferLinkError::InvalidUrl)?;
    let (authority, path_and_query) = match authority_and_rest.find(['/', '?', '#']) {
        Some(index) => authority_and_rest.split_at(index),
        None => (authority_and_rest, ""),
    };
    if authority.bytes().any(|byte| matches!(byte, b'@' | b':')) {
        return Err(TonTransferLinkError::InvalidUrl);
    }

    let (raw_path, raw_query) = match path_and_query.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (path_and_query, None),
    };
    if raw_path.bytes().any(|byte| byte == b' ') {
        return Err(TonTransferLinkError::InvalidUrl);
    }
    Ok((raw_path, raw_query))
}

fn is_canonical_raw_address(value: &str) -> bool {
    if !value.contains(':') {
        return true;
    }
    value
        .parse::<RawAccountAddress>()
        .is_ok_and(|address| address.to_string() == value)
}

#[derive(Default)]
struct RawParameters {
    amount: Option<String>,
    text: Option<String>,
    exp: Option<String>,
    jetton: Option<String>,
    binary: Option<String>,
}

fn parse_parameters(raw_query: Option<&str>) -> Result<RawParameters, TonTransferLinkError> {
    let Some(raw_query) = raw_query.filter(|query| !query.is_empty()) else {
        return Ok(RawParameters::default());
    };
    let mut parameters = RawParameters::default();

    for pair in raw_query.split('&') {
        let (raw_name, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let name = percent_decode_once(raw_name)?;
        let value = percent_decode_once(raw_value)?;
        match name.as_str() {
            "amount" => set_once(&mut parameters.amount, name, value)?,
            "text" => set_once(&mut parameters.text, name, value)?,
            "exp" => set_once(&mut parameters.exp, name, value)?,
            "jetton" => set_once(&mut parameters.jetton, name, value)?,
            "bin" => set_once(&mut parameters.binary, name, value)?,
            _ => return Err(TonTransferLinkError::UnsupportedParameter { name }),
        }
    }

    Ok(parameters)
}

fn set_once(
    slot: &mut Option<String>,
    name: String,
    value: String,
) -> Result<(), TonTransferLinkError> {
    if slot.is_some() {
        return Err(TonTransferLinkError::DuplicateParameter { name });
    }
    *slot = Some(value);
    Ok(())
}

fn parse_expiration(value: Option<String>) -> Result<SendExpiration, TonTransferLinkError> {
    let Some(value) = value else {
        return Ok(SendExpiration::EngineDefault);
    };
    let canonical = UnsignedDecimalString::try_from(value)
        .map_err(|_| TonTransferLinkError::InvalidExpiration)?;
    let unix_timestamp = canonical
        .try_to::<u64>()
        .map_err(|_| TonTransferLinkError::InvalidExpiration)?;
    Ok(SendExpiration::Exact { unix_timestamp })
}

fn percent_decode_once(value: &str) -> Result<String, TonTransferLinkError> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.as_bytes().iter().copied();

    while let Some(byte) = input.next() {
        if byte != b'%' {
            bytes.push(byte);
            continue;
        }

        let high = input
            .next()
            .and_then(hex_value)
            .ok_or(TonTransferLinkError::InvalidUrl)?;
        let low = input
            .next()
            .and_then(hex_value)
            .ok_or(TonTransferLinkError::InvalidUrl)?;
        bytes.push(high.wrapping_mul(16).wrapping_add(low));
    }

    String::from_utf8(bytes).map_err(|_| TonTransferLinkError::InvalidUrl)
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte.wrapping_sub(b'0')),
        b'a'..=b'f' => Some(byte.wrapping_sub(b'a').wrapping_add(10)),
        b'A'..=b'F' => Some(byte.wrapping_sub(b'A').wrapping_add(10)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use ton::ton_core::cell::TonCell;
    use ton::ton_core::traits::tlb::TLB;

    use super::*;

    const RECIPIENT: &str = "0:0000000000000000000000000000000000000000000000000000000000000000";
    const JETTON: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn parses_recipient_only_without_inventing_an_amount_or_payload() {
        let parsed = parse(&format!("ton://transfer/{RECIPIENT}"));

        assert_eq!(parsed.recipient.as_str(), RECIPIENT);
        assert_eq!(parsed.asset, TonTransferAsset::Gram);
        assert_eq!(parsed.amount, None);
        assert_eq!(parsed.payload, TonTransferPayload::None);
        assert_eq!(parsed.expiration, SendExpiration::EngineDefault);
    }

    #[test]
    fn accepts_url_safe_and_percent_encoded_standard_friendly_addresses() {
        let url_safe = "EQDk2VTvn04SUKJrW7rXahzdF8_Qi6utb0wj43InCu9vdjrR";
        let standard = "EQDk2VTvn04SUKJrW7rXahzdF8/Qi6utb0wj43InCu9vdjrR";
        let standard_path = "EQDk2VTvn04SUKJrW7rXahzdF8%2FQi6utb0wj43InCu9vdjrR";

        assert_eq!(
            parse(&format!("ton://transfer/{url_safe}"))
                .recipient
                .as_str(),
            url_safe
        );
        assert_eq!(
            parse(&format!("ton://transfer/{standard_path}"))
                .recipient
                .as_str(),
            standard
        );
    }

    #[test]
    fn parses_gram_amount_text_and_expiration() {
        let parsed = parse(&format!(
            "TON://TRANSFER/{RECIPIENT}?amount=1000000000&text=hello%20TON&exp=18446744073709551615"
        ));

        assert_eq!(
            parsed
                .amount
                .as_ref()
                .map(UnsignedDecimalString::to_decimal_string),
            Some("1000000000".to_owned())
        );
        assert_eq!(
            parsed.payload,
            TonTransferPayload::Text {
                text: "hello TON".to_owned()
            }
        );
        assert_eq!(
            parsed.expiration,
            SendExpiration::Exact {
                unix_timestamp: u64::MAX
            }
        );
    }

    #[test]
    fn distinguishes_an_empty_comment_from_an_absent_comment() {
        let parsed = parse(&format!("ton://transfer/{RECIPIENT}?text="));

        assert_eq!(
            parsed.payload,
            TonTransferPayload::Text {
                text: String::new()
            }
        );
    }

    #[test]
    fn parses_jetton_amount_comment_and_expiration() {
        let parsed = parse(&format!(
            "ton://transfer/{RECIPIENT}?jetton={JETTON}&amount=999999999999999999999999999999&text=jetton&exp=1"
        ));

        assert_eq!(
            parsed.asset,
            TonTransferAsset::Jetton {
                master: TonAddressString::try_from(JETTON).expect("fixture address must be valid")
            }
        );
        assert_eq!(
            parsed
                .amount
                .as_ref()
                .map(UnsignedDecimalString::to_decimal_string),
            Some("999999999999999999999999999999".to_owned())
        );
    }

    #[test]
    fn parses_a_standard_base64_single_root_boc() {
        let encoded = Boc::try_from(TonCell::EMPTY_BOC.to_vec())
            .expect("empty cell BOC must be valid")
            .to_base64();
        let parsed = parse(&format!("ton://transfer/{RECIPIENT}?bin={encoded}"));

        assert_eq!(
            parsed.payload,
            TonTransferPayload::Boc {
                boc: Boc::try_from(encoded).expect("fixture BOC must be valid")
            }
        );
    }

    #[test]
    fn preserves_reserved_base64_characters_in_raw_and_percent_encoded_forms() {
        for character in ['+', '/', '='] {
            let raw = valid_boc_with(character);
            let percent_encoded = raw
                .replace('%', "%25")
                .replace('+', "%2B")
                .replace('/', "%2F")
                .replace('=', "%3D");
            let literal = parse(&format!("ton://transfer/{RECIPIENT}?bin={raw}"));
            let encoded = parse(&format!("ton://transfer/{RECIPIENT}?bin={percent_encoded}"));

            assert_eq!(literal.payload, encoded.payload);
        }
    }

    #[test]
    fn keeps_literal_and_percent_encoded_plus_characters() {
        assert_eq!(percent_decode_once("a+b").expect("valid text"), "a+b");
        assert_eq!(percent_decode_once("a%2Bb").expect("valid text"), "a+b");

        let literal = parse(&format!("ton://transfer/{RECIPIENT}?text=a+b"));
        let encoded = parse(&format!("ton://transfer/{RECIPIENT}?text=a%2Bb"));
        assert_eq!(literal.payload, encoded.payload);
    }

    #[test]
    fn splits_query_before_decoding_and_decodes_once() {
        let parsed = parse(&format!(
            "ton://transfer/{RECIPIENT}?text=one%26two%3Dthree%2526four"
        ));

        assert_eq!(
            parsed.payload,
            TonTransferPayload::Text {
                text: "one&two=three%26four".to_owned()
            }
        );
    }

    #[test]
    fn rejects_invalid_amount_forms() {
        for amount in ["", "00", "+1", "-1", " 1", "1.0", "1e9"] {
            assert_eq!(
                parse_error(&format!("ton://transfer/{RECIPIENT}?amount={amount}")),
                TonTransferLinkError::InvalidAmount
            );
        }
    }

    #[test]
    fn accepts_zero_and_arbitrary_precision_amounts() {
        for amount in ["0", "18446744073709551616000000000000000000"] {
            let parsed = parse(&format!("ton://transfer/{RECIPIENT}?amount={amount}"));
            assert_eq!(
                parsed.amount.map(|value| value.to_decimal_string()),
                Some(amount.to_owned())
            );
        }
    }

    #[test]
    fn rejects_invalid_or_overflowing_expirations() {
        for expiration in ["", "00", "+1", "-1", "1.0", "18446744073709551616"] {
            assert_eq!(
                parse_error(&format!("ton://transfer/{RECIPIENT}?exp={expiration}")),
                TonTransferLinkError::InvalidExpiration
            );
        }
    }

    #[test]
    fn rejects_duplicate_baseline_parameters_after_name_decoding() {
        for parameter in ["amount", "text", "exp", "jetton", "bin"] {
            assert_eq!(
                parse_error(&format!(
                    "ton://transfer/{RECIPIENT}?{parameter}=x&{parameter}=x"
                )),
                TonTransferLinkError::DuplicateParameter {
                    name: parameter.to_owned()
                }
            );
        }

        assert_eq!(
            parse_error(&format!("ton://transfer/{RECIPIENT}?text=x&%74ext=y")),
            TonTransferLinkError::DuplicateParameter {
                name: "text".to_owned()
            }
        );
    }

    #[test]
    fn rejects_unknown_and_case_variant_parameters() {
        for parameter in [
            "Text",
            "init",
            "stateInit",
            "nft",
            "fee-amount",
            "forward-amount",
        ] {
            assert_eq!(
                parse_error(&format!("ton://transfer/{RECIPIENT}?{parameter}=x")),
                TonTransferLinkError::UnsupportedParameter {
                    name: parameter.to_owned()
                }
            );
        }
    }

    #[test]
    fn rejects_malformed_percent_encoding_and_utf8() {
        for query in ["text=%", "text=%0", "text=%GG", "text=%FF"] {
            assert_eq!(
                parse_error(&format!("ton://transfer/{RECIPIENT}?{query}")),
                TonTransferLinkError::InvalidUrl
            );
        }
    }

    #[test]
    fn rejects_source_characters_that_the_url_parser_would_discard() {
        for value in [
            format!(" ton://transfer/{RECIPIENT}"),
            format!("ton://transfer/{RECIPIENT} "),
            format!("ton://transfer/{RECIPIENT}?text=a\tb"),
            format!("ton://transfer/{RECIPIENT}?amount=1\n"),
            format!("ton://transfer/{RECIPIENT}?exp=1\r"),
        ] {
            assert_eq!(parse_error(&value), TonTransferLinkError::InvalidUrl);
        }
    }

    #[test]
    fn rejects_payload_and_asset_conflicts() {
        let boc = Boc::try_from(TonCell::EMPTY_BOC.to_vec())
            .expect("empty cell BOC must be valid")
            .to_base64();
        assert_eq!(
            parse_error(&format!("ton://transfer/{RECIPIENT}?text=x&bin={boc}")),
            TonTransferLinkError::ConflictingPayloads
        );
        assert_eq!(
            parse_error(&format!(
                "ton://transfer/{RECIPIENT}?jetton={JETTON}&bin={boc}"
            )),
            TonTransferLinkError::BinaryJettonConflict
        );
    }

    #[test]
    fn rejects_invalid_binary_payloads() {
        for binary in ["", "not-base64", "AAAAAA=="] {
            assert_eq!(
                parse_error(&format!("ton://transfer/{RECIPIENT}?bin={binary}")),
                TonTransferLinkError::InvalidBinaryPayload
            );
        }
    }

    #[test]
    fn classifies_route_and_structure_errors() {
        assert_eq!(
            parse_error(&format!("tc://transfer/{RECIPIENT}")),
            TonTransferLinkError::NotTonTransfer
        );
        assert_eq!(
            parse_error("ton://transfer"),
            TonTransferLinkError::MissingRecipient
        );
        assert_eq!(
            parse_error("ton://transfer/"),
            TonTransferLinkError::MissingRecipient
        );
        assert_eq!(
            parse_error(&format!("ton://transfer/{RECIPIENT}/extra")),
            TonTransferLinkError::InvalidUrl
        );
        for path in [
            format!("ignored/../{RECIPIENT}"),
            format!("ignored/%2e%2e/{RECIPIENT}"),
            format!("%2e/{RECIPIENT}"),
        ] {
            assert_eq!(
                parse_error(&format!("ton://transfer/{path}")),
                TonTransferLinkError::InvalidUrl
            );
        }
        assert_eq!(
            parse_error(&format!("ton://user@transfer/{RECIPIENT}")),
            TonTransferLinkError::InvalidUrl
        );
        assert_eq!(
            parse_error(&format!("ton://@transfer/{RECIPIENT}")),
            TonTransferLinkError::InvalidUrl
        );
        assert_eq!(
            parse_error(&format!("ton://transfer:80/{RECIPIENT}")),
            TonTransferLinkError::InvalidUrl
        );
        assert_eq!(
            parse_error(&format!("ton://transfer:/{RECIPIENT}")),
            TonTransferLinkError::InvalidUrl
        );
        assert_eq!(
            parse_error(&format!("ton://transfer/{RECIPIENT}#fragment")),
            TonTransferLinkError::InvalidUrl
        );
    }

    #[test]
    fn rejects_invalid_recipient_and_jetton_addresses() {
        assert_eq!(
            parse_error("ton://transfer/not-an-address"),
            TonTransferLinkError::InvalidRecipient
        );
        assert_eq!(
            parse_error(&format!("ton://transfer/{RECIPIENT}?jetton=not-an-address")),
            TonTransferLinkError::InvalidJettonMaster
        );
    }

    #[test]
    fn rejects_noncanonical_raw_recipient_and_jetton_addresses() {
        for address in [
            format!("0:{}", "AB".repeat(32)),
            format!("00:{}", "ab".repeat(32)),
            format!("+0:{}", "ab".repeat(32)),
            format!("-0:{}", "ab".repeat(32)),
        ] {
            assert_eq!(
                parse_error(&format!("ton://transfer/{address}")),
                TonTransferLinkError::InvalidRecipient
            );
            assert_eq!(
                parse_error(&format!("ton://transfer/{RECIPIENT}?jetton={address}")),
                TonTransferLinkError::InvalidJettonMaster
            );
        }
    }

    fn parse(value: &str) -> ParsedTonTransferLink {
        parse_ton_transfer_link(value.to_owned()).expect("fixture link must be valid")
    }

    fn parse_error(value: &str) -> TonTransferLinkError {
        parse_ton_transfer_link(value.to_owned()).expect_err("fixture link must be invalid")
    }

    fn valid_boc_with(character: char) -> String {
        if character == '=' {
            return Boc::try_from(TonCell::EMPTY_BOC.to_vec())
                .expect("empty cell BOC must be valid")
                .to_base64();
        }
        (0_u16..=u16::MAX)
            .find_map(|value| {
                let mut builder = TonCell::builder();
                builder.write_num(&value, 16).ok()?;
                let encoded = builder.build().ok()?.to_boc().ok()?;
                let encoded = Boc::try_from(encoded).ok()?.to_base64();
                encoded.contains(character).then_some(encoded)
            })
            .expect("the finite two-byte cell set must contain every standard Base64 character")
    }
}
