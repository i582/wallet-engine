//! Non-blank strings validated at portable API boundaries.

use std::{
    fmt::{Display, Formatter},
    ops::Deref,
};

/// A string that contains at least one non-whitespace character.
///
/// `Serde` and `UniFFI` expose this value as an ordinary string, but reject
/// empty and whitespace-only input while lifting it into Rust. The original
/// text is preserved: this type validates the invariant without trimming or
/// otherwise normalizing identifiers.
#[repr(transparent)]
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct NonEmptyString(String);

impl NonEmptyString {
    /// Borrows the validated string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the original string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl TryFrom<String> for NonEmptyString {
    type Error = NonEmptyStringError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            return Err(NonEmptyStringError);
        }

        Ok(Self(value))
    }
}

impl TryFrom<&str> for NonEmptyString {
    type Error = NonEmptyStringError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::try_from(value.to_owned())
    }
}

impl From<NonEmptyString> for String {
    fn from(value: NonEmptyString) -> Self {
        value.0
    }
}

impl AsRef<str> for NonEmptyString {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for NonEmptyString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl Display for NonEmptyString {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Reports an empty or whitespace-only boundary string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("value must contain at least one non-whitespace character")]
pub struct NonEmptyStringError;

#[cfg(feature = "ffi")]
uniffi::custom_type!(NonEmptyString, String);

#[cfg(test)]
mod tests {
    #[cfg(feature = "ffi")]
    use uniffi::{Lift, Lower};

    use super::NonEmptyString;

    #[test]
    fn construction_rejects_blank_values_without_normalizing_valid_text() {
        for value in ["", " ", "\t", "\r\n"] {
            assert!(
                NonEmptyString::try_from(value).is_err(),
                "accepted {value:?}"
            );
        }

        let value = NonEmptyString::try_from(" operation ").expect("non-blank text is valid");
        assert_eq!(value.as_str(), " operation ");
        assert_eq!(value.into_string(), " operation ");
    }

    #[test]
    fn serde_keeps_the_portable_representation_as_a_string() {
        let value = NonEmptyString::try_from("operation").expect("non-blank text is valid");
        let encoded = serde_json::to_string(&value).expect("value must serialize");
        assert_eq!(encoded, r#""operation""#);

        assert_eq!(
            serde_json::from_str::<NonEmptyString>(&encoded).expect("value must deserialize"),
            value
        );
        assert!(serde_json::from_str::<NonEmptyString>(r#"" ""#).is_err());
    }

    #[test]
    fn borrowed_and_formatted_views_preserve_the_validated_text() {
        let value = NonEmptyString::try_from(" operation ").expect("non-blank text is valid");

        assert_eq!(AsRef::<str>::as_ref(&value), " operation ");
        assert_eq!(&*value, " operation ");
        assert_eq!(value.to_string(), " operation ");
    }

    #[cfg(feature = "ffi")]
    #[test]
    fn uniffi_rejects_a_blank_client_value_while_lifting() {
        let ffi_value = <String as Lower<crate::UniFfiTag>>::lower("   ".to_owned());
        let result = <NonEmptyString as Lift<crate::UniFfiTag>>::try_lift(ffi_value);

        assert!(result.is_err(), "UniFFI accepted a blank string");
    }
}
