//! Internal arbitrary-precision decimal helpers.

use std::str::FromStr;

use num_bigint::BigUint;

/// Parses a positive canonical base-10 integer.
///
/// Comparing the normalized value rejects leading zeros and alternate forms.
pub(crate) fn parse_positive_decimal(value: &str) -> Option<BigUint> {
    let parsed = parse_canonical_decimal(value)?;

    (parsed != BigUint::default()).then_some(parsed)
}

/// Parses a canonical nonnegative base-10 integer.
///
/// Provider fee fields can validly contain zero. Comparing the normalized
/// representation still rejects signs, whitespace, and leading zeros.
pub(crate) fn parse_canonical_decimal(value: &str) -> Option<BigUint> {
    let parsed = BigUint::from_str(value).ok()?;

    if parsed.to_str_radix(10) != value {
        return None;
    }

    Some(parsed)
}

#[cfg(test)]
mod tests {
    use super::{parse_canonical_decimal, parse_positive_decimal};

    #[test]
    fn canonical_decimal_accepts_zero_without_weakening_positive_amounts() {
        assert_eq!(
            parse_canonical_decimal("0").map(|value| value.to_string()),
            Some("0".to_owned())
        );
        assert!(parse_positive_decimal("0").is_none());
        assert!(parse_canonical_decimal("00").is_none());
        assert!(parse_canonical_decimal("+1").is_none());
    }
}
