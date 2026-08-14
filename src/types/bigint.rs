//! Internal arbitrary-precision decimal helpers.

use std::str::FromStr;

use num_bigint::BigUint;

/// Parses a positive canonical base-10 integer.
///
/// Comparing the normalized value rejects leading zeros and alternate forms.
pub(crate) fn parse_positive_decimal(value: &str) -> Option<BigUint> {
    let parsed = BigUint::from_str(value).ok()?;

    if parsed == BigUint::default() || parsed.to_str_radix(10) != value {
        return None;
    }

    Some(parsed)
}
