//! Unsigned arbitrary-precision integers represented as decimal strings at
//! portable API boundaries.

use std::{
    fmt::{Display, Formatter},
    ops::Add,
    str::FromStr,
};

use num_bigint::BigUint;

/// An unsigned arbitrary-precision integer carried as a base-10 string across
/// serialization and FFI boundaries.
///
/// Swift and Kotlin do not share Rust's arbitrary-precision integer ABI. Public
/// wallet records therefore use this type when the value must remain exact but
/// can exceed a platform integer. `Serde` and `UniFFI` expose the value as an
/// ordinary string.
///
/// Rust stores the parsed [`BigUint`], so invalid input cannot survive boundary
/// conversion and no later operation needs to parse the value again.
/// Construction rejects signs, whitespace, leading zeroes, and empty or
/// nonnumeric values. Transfer amounts, balances, and fees can validly be zero.
#[repr(transparent)]
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct UnsignedDecimalString(BigUint);

impl UnsignedDecimalString {
    /// Borrows the arbitrary-precision integer stored by this value.
    #[must_use]
    pub const fn as_biguint(&self) -> &BigUint {
        &self.0
    }

    /// Consumes the boundary value and returns its arbitrary-precision integer.
    #[must_use]
    pub fn into_biguint(self) -> BigUint {
        self.0
    }

    /// Returns the canonical base-10 boundary representation.
    #[must_use]
    pub fn to_decimal_string(&self) -> String {
        self.0.to_str_radix(10)
    }

    /// Converts the stored integer to a caller-selected numeric type.
    ///
    /// The target type can reject a value outside its numeric range. This
    /// conversion operates directly on [`BigUint`] without a string round trip.
    pub fn try_to<'a, T>(&'a self) -> Result<T, <T as TryFrom<&'a BigUint>>::Error>
    where
        T: TryFrom<&'a BigUint>,
    {
        T::try_from(&self.0)
    }

    /// Clones the stored arbitrary-precision integer.
    ///
    /// Prefer [`Self::as_biguint`] when a borrowed value is sufficient, or
    /// [`Self::into_biguint`] when this value is no longer needed.
    #[must_use]
    pub fn to_biguint(&self) -> BigUint {
        self.0.clone()
    }
}

impl TryFrom<String> for UnsignedDecimalString {
    type Error = UnsignedDecimalStringError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if !is_canonical_unsigned_decimal(&value) {
            return Err(UnsignedDecimalStringError);
        }

        BigUint::from_str(&value)
            .map(Self)
            .map_err(|_| UnsignedDecimalStringError)
    }
}

impl TryFrom<&str> for UnsignedDecimalString {
    type Error = UnsignedDecimalStringError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl From<BigUint> for UnsignedDecimalString {
    fn from(value: BigUint) -> Self {
        Self(value)
    }
}

impl From<&BigUint> for UnsignedDecimalString {
    fn from(value: &BigUint) -> Self {
        Self(value.clone())
    }
}

impl From<u64> for UnsignedDecimalString {
    fn from(value: u64) -> Self {
        Self(BigUint::from(value))
    }
}

impl From<UnsignedDecimalString> for String {
    fn from(value: UnsignedDecimalString) -> Self {
        value.0.to_str_radix(10)
    }
}

impl Display for UnsignedDecimalString {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, formatter)
    }
}

impl Add<&UnsignedDecimalString> for &UnsignedDecimalString {
    type Output = UnsignedDecimalString;

    fn add(self, rhs: &UnsignedDecimalString) -> Self::Output {
        #[allow(
            clippy::arithmetic_side_effects,
            reason = "BigUint addition cannot overflow"
        )]
        let sum = &self.0 + &rhs.0;
        UnsignedDecimalString(sum)
    }
}

fn is_canonical_unsigned_decimal(value: &str) -> bool {
    let bytes = value.as_bytes();
    match bytes {
        [b'0'] => true,
        [first, rest @ ..] => {
            first.is_ascii_digit() && *first != b'0' && rest.iter().all(u8::is_ascii_digit)
        }
        [] => false,
    }
}

/// Reports a noncanonical unsigned base-10 integer string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("value must be a canonical unsigned base-10 integer")]
pub struct UnsignedDecimalStringError;

uniffi::custom_type!(UnsignedDecimalString, String);

#[cfg(test)]
mod tests {
    use num_bigint::BigUint;
    use uniffi::{Lift, Lower};

    use super::UnsignedDecimalString;

    #[test]
    fn serde_keeps_the_portable_representation_as_a_string() {
        let value = UnsignedDecimalString::try_from("18446744073709551616000000000")
            .expect("canonical decimal must be accepted");
        let encoded = serde_json::to_string(&value).expect("decimal string must serialize");
        assert_eq!(encoded, r#""18446744073709551616000000000""#);

        let decoded = serde_json::from_str::<UnsignedDecimalString>(&encoded)
            .expect("serialized decimal string must deserialize");
        assert_eq!(decoded, value);
    }

    #[test]
    fn conversion_to_and_from_string_is_lossless() {
        let original = "10000000000000000000000000000".to_owned();
        let value = UnsignedDecimalString::try_from(original.clone())
            .expect("canonical decimal must be accepted");

        assert_eq!(value.to_decimal_string(), original);
        assert_eq!(String::from(value), original);
    }

    #[test]
    fn construction_rejects_noncanonical_values() {
        for value in ["", "00", "01", "+1", "-1", " 1", "1 ", "1.0", "one"] {
            assert!(
                UnsignedDecimalString::try_from(value).is_err(),
                "accepted {value:?}"
            );
        }

        for value in ["0", "1", "10"] {
            assert!(
                UnsignedDecimalString::try_from(value).is_ok(),
                "rejected {value:?}"
            );
        }
    }

    #[test]
    fn uniffi_rejects_a_negative_client_value_while_lifting() {
        let ffi_value = <String as Lower<crate::UniFfiTag>>::lower("-100".to_owned());
        let result = <UnsignedDecimalString as Lift<crate::UniFfiTag>>::try_lift(ffi_value);

        assert!(result.is_err(), "UniFFI accepted a negative decimal string");
    }

    #[test]
    fn biguint_conversion_is_canonical() {
        let value = BigUint::from(u128::MAX);

        assert_eq!(
            UnsignedDecimalString::from(&value).to_decimal_string(),
            value.to_str_radix(10)
        );
        assert_eq!(
            UnsignedDecimalString::from(value.clone()),
            UnsignedDecimalString::from(&value)
        );
        assert_eq!(
            UnsignedDecimalString::from(value.clone()).to_biguint(),
            value
        );
        assert_eq!(UnsignedDecimalString::from(&value).as_biguint(), &value);
        assert_eq!(UnsignedDecimalString::from(&value).into_biguint(), value);
        assert_eq!(
            UnsignedDecimalString::try_from("0")
                .expect("zero is canonical")
                .to_biguint(),
            BigUint::default()
        );
    }

    #[test]
    fn values_compare_and_add_as_unsigned_integers() {
        let ten = UnsignedDecimalString::try_from("10").expect("canonical decimal");
        let two = UnsignedDecimalString::try_from("2").expect("canonical decimal");

        assert!(ten > two);
        assert_eq!(&ten + &two, UnsignedDecimalString::from(12_u64));
        assert_eq!(ten.try_to::<u64>(), Ok(10));
        assert_eq!(ten.to_string(), "10");
    }
}
