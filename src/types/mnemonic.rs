//! TON mnemonic word-list utilities.

use ton::ton_wallet::WORDLIST_EN;

/// Returns the English word list accepted by TON mnemonic validation.
///
/// The 2048 words keep the source order from the embedded TON word-list file.
#[uniffi::export]
#[must_use]
pub fn mnemonic_wordlist() -> Vec<String> {
    WORDLIST_EN.lines().map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::mnemonic_wordlist;
    use ton::ton_wallet::WORDLIST_EN;

    #[test]
    fn exports_the_complete_ton_wordlist_in_source_order() {
        let words = mnemonic_wordlist();

        assert_eq!(words.len(), 2048);
        assert_eq!(words.first().map(String::as_str), Some("abandon"));
        assert_eq!(words.last().map(String::as_str), Some("zoo"));
        assert_eq!(
            words,
            WORDLIST_EN.lines().map(str::to_owned).collect::<Vec<_>>()
        );
    }
}
