//! Recovery-phrase scheme detection for wallet import.
//!
//! One entered phrase can be a valid mnemonic under more than one scheme, so
//! [`detect_mnemonic_schemes`] runs every check the engine knows and reports
//! all matches instead of silently picking one
//! ([TEP-0003 section 8](https://github.com/ton-blockchain/TEPs/blob/master/text/0003-wallets.md#8-wallet-import-mnemonic-scheme-detection)).
//!
//! Wallet import accepts only the rotation scheme. The other schemes are
//! recognized so an application can tell the user *what* they entered instead
//! of showing a generic "invalid phrase" message. The wording of that message
//! stays in the application; the engine only classifies.

use zeroize::Zeroizing;

use super::mnemonic::{RotationMnemonic, is_bip39_24_phrase, is_ton_mnemonic};

/// A recovery-phrase scheme recognized by [`detect_mnemonic_schemes`].
///
/// The names follow
/// [TEP-0003](https://github.com/ton-blockchain/TEPs/blob/master/text/0003-wallets.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum MnemonicScheme {
    /// A rotation mnemonic (TEP-0003 section 3.3): 12 words before the
    /// wallet's one-time key rotation or 24 words after it. The only scheme
    /// wallet import accepts.
    Rotation,
    /// A passwordless legacy 24-word TON mnemonic (TEP-0003 section 3.1).
    /// Recognized so the application can explain the rejection; never
    /// imported, and no key is derived from it.
    Ton,
    /// A standard 24-word BIP-39 phrase - the Multichain mnemonic of TEP-0003
    /// section 3.2. Recognized so the application can explain the rejection;
    /// never imported, and no key is derived from it.
    Bip39,
}

/// Reports every scheme under which the entered recovery words validate.
///
/// Pass the words exactly as the user recorded them, one word per element -
/// the same value an `ImportWalletRequest` would carry. Each word is trimmed
/// and lowercased before validation, exactly like wallet import.
///
/// The result lists all matching schemes in the fixed order
/// [`Rotation`](MnemonicScheme::Rotation), [`Ton`](MnemonicScheme::Ton),
/// [`Bip39`](MnemonicScheme::Bip39); the checks are independent, so one
/// phrase can match more than one scheme. An empty result means the words
/// validate under no scheme the engine knows.
///
/// Wallet import succeeds exactly when the result contains
/// [`MnemonicScheme::Rotation`]. A result without it explains why import
/// reports `InvalidRecoveryPhrase`; the user-facing wording of that
/// explanation belongs to the application.
///
/// The function derives no key material and logs nothing. Password-protected
/// TON mnemonics are not detectable without the password and report no match.
#[uniffi::export]
#[must_use]
pub fn detect_mnemonic_schemes(words: Vec<String>) -> Vec<MnemonicScheme> {
    let words = Zeroizing::new(words);
    let refs = words.iter().map(String::as_str).collect::<Vec<_>>();

    let mut schemes = Vec::new();
    if RotationMnemonic::new(&refs).is_ok() {
        schemes.push(MnemonicScheme::Rotation);
    }
    if is_ton_mnemonic(&refs) {
        schemes.push(MnemonicScheme::Ton);
    }
    if is_bip39_24_phrase(&refs) {
        schemes.push(MnemonicScheme::Bip39);
    }
    schemes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::mnemonic::{Bip39Half, ENTROPY_LEN};

    /// A valid pre-rotation phrase: all-zero entropy.
    const ROTATION_12: &str = "abandon abandon abandon abandon abandon abandon \
                               abandon abandon abandon abandon abandon about";

    /// A valid post-rotation phrase with two distinct halves.
    const ROTATION_24: &str = "notice tortoise soup strong gun divide offer process salon siren general carry \
                               clump left year void clutch tool case burden fix income champion lounge";

    /// The passwordless TON mnemonic from the vendored `ton` crate's tests.
    const TON_24: &str = "dose ice enrich trigger test dove century still betray \
                          gas diet dune use other base gym mad law immense village \
                          world example praise game";

    /// The all-zero-entropy 24-word BIP-39 vector from `trezor/python-mnemonic`.
    const BIP39_24: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
                            abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art";

    fn detect(phrase: &str) -> Vec<MnemonicScheme> {
        detect_mnemonic_schemes(phrase.split_whitespace().map(str::to_owned).collect())
    }

    #[test]
    fn classifies_each_scheme() {
        assert_eq!(detect(ROTATION_12), vec![MnemonicScheme::Rotation]);
        assert_eq!(detect(ROTATION_24), vec![MnemonicScheme::Rotation]);
        assert_eq!(detect(TON_24), vec![MnemonicScheme::Ton]);
        assert_eq!(detect(BIP39_24), vec![MnemonicScheme::Bip39]);
    }

    #[test]
    fn reports_no_scheme_for_unrecognized_words() {
        assert_eq!(detect(""), vec![]);
        assert_eq!(detect("abandon"), vec![]);
        // A valid phrase truncated to 23 words fits no scheme.
        let truncated = TON_24.split_whitespace().take(23).collect::<Vec<_>>();
        assert_eq!(
            detect_mnemonic_schemes(truncated.iter().map(|w| (*w).to_owned()).collect()),
            vec![]
        );
        assert_eq!(
            detect_mnemonic_schemes(vec!["notaword".to_owned(); 24]),
            vec![]
        );
    }

    /// Detection mirrors import: one element is one word, never a phrase.
    #[test]
    fn does_not_split_words_itself() {
        assert_eq!(detect_mnemonic_schemes(vec![TON_24.to_owned()]), vec![]);
    }

    #[test]
    fn normalizes_case_and_whitespace_like_import() {
        let messy = TON_24
            .split_whitespace()
            .map(|word| format!("  {}  ", word.to_uppercase()))
            .collect::<Vec<_>>();
        assert_eq!(detect_mnemonic_schemes(messy), vec![MnemonicScheme::Ton]);
    }

    /// The checks are independent: one phrase can match several schemes.
    ///
    /// Deterministically searches rotation phrases built from constant-byte
    /// entropy until the 24 words also carry a valid 24-word BIP-39 checksum,
    /// which one candidate in 256 does on average. Candidates are filtered
    /// with the cheap checksum test; only the hit runs full detection.
    #[test]
    fn one_phrase_can_validate_under_more_than_one_scheme() {
        let half_phrase = |byte: u8| {
            Bip39Half::from_entropy(&[byte; ENTROPY_LEN])
                .expect("intact word list")
                .to_phrase()
        };

        for anchor_byte in 0_u8..=255 {
            let anchor = half_phrase(anchor_byte);
            for signing_byte in 0_u8..=255 {
                let phrase = format!("{} {}", anchor.as_str(), half_phrase(signing_byte).as_str());
                if !is_bip39_24_phrase(&phrase.split_whitespace().collect::<Vec<_>>()) {
                    continue;
                }

                let schemes = detect(&phrase);
                assert!(schemes.contains(&MnemonicScheme::Rotation), "{phrase}");
                assert!(schemes.contains(&MnemonicScheme::Bip39), "{phrase}");
                return;
            }
        }

        panic!("no rotation phrase with a coincidental BIP-39 checksum found");
    }
}
