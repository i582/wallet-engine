//! Bounded formatting for diagnostics that cross trust boundaries.
//!
//! This module does not remove secrets. Hosts and providers must not include
//! secrets in diagnostics. It only replaces control characters and limits the
//! amount of text retained in public state or errors.

pub(crate) fn bounded_diagnostic(message: impl AsRef<str>) -> String {
    const DIAGNOSTIC_MAX_CHARS: usize = 512;

    message
        .as_ref()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(DIAGNOSTIC_MAX_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}
