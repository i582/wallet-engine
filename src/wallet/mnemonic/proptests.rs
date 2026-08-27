//! Differential property tests for [`super`] against the `bip39` crate.
//!
//! `bip39` from `rust-bitcoin` implements the same standard in an independent
//! code base, so it decides what the engine's own encoder should have produced
//! for every generated input. It is a dev-dependency: its word tables and its
//! CC0-1.0 license never reach the build.
//!
//! # Case counts
//!
//! A fault that a fraction `p` of inputs reaches survives `n` independent
//! draws with probability `(1 - p)^n`. Bounding that by a miss rate `d` gives
//! `n >= ln d / ln (1 - p)`. Every count below solves that at `d = 0.001` for
//! the narrowest fault the property's own generator can express.
use bip39::{Language, Mnemonic};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;

use super::*;
use crate::wallet::crypto::derive_rotation_keys;
use crate::wallet::mnemonic_scheme::{MnemonicScheme, detect_mnemonic_schemes};
use crate::wallet::slip_0010::{TON_ACCOUNT_PATH, derive_path, signing_key};

/// Cases for a property that goes through word encoding.
///
/// The narrowest fault there is one wrong slot of the 2048-word list. Twelve
/// drawn words reach a given slot with `p = 1 - (2047/2048)^12`, which needs
/// this many cases.
///
/// The count also makes the one-in-sixteen events these properties rely on
/// certain in practice, such as a swapped word that still checksums.
const WORD_SLOT_CASES: u32 = 1179;

/// Cases for a seed property whose case draws one half.
///
/// Seed derivation never branches on a word index, so the narrowest fault is
/// one wrong byte value. Sixteen entropy bytes reach a given value with
/// `p = 1 - (255/256)^16`, which needs this many cases.
const ENTROPY_BYTE_CASES: u32 = 111;

/// Cases for the 24-word detection property.
///
/// Detection only accepts or rejects, so the rarest event it relies on is a
/// swapped list word whose phrase still checksums: one swap in 256, since a
/// 24-word phrase carries an 8-bit checksum. That event is missed by `n`
/// draws with `(255/256)^n`, which needs this many cases at `d = 0.001`.
const BIP39_24_SWAP_CASES: u32 = 1765;

/// Cases for a property that goes through 24-word encoding.
///
/// Same fault model as [`WORD_SLOT_CASES`] with twenty-four drawn words
/// reaching a given slot of the 2048-word list:
/// `p = 1 - (2047/2048)^24`, which needs this many cases.
const WORD_SLOT_24_CASES: u32 = 590;

/// Cases for the other-length rejection property.
///
/// Rejection happens on the word count alone, so the narrowest fault is one
/// mishandled length of the three drawn: `p = 1/3`, which needs this many
/// cases.
const OTHER_LENGTH_CASES: u32 = 18;

/// Cases for a seed property whose case draws both halves.
///
/// Thirty-two entropy bytes per case double `p`, so half the cases cover the
/// same byte values. Seed derivation is by far the slowest work here, which
/// makes the difference worth spelling out.
const PAIRED_ENTROPY_BYTE_CASES: u32 = 56;

/// The BIP-39 English list, as the oracle sees it.
///
/// The return type ties the oracle's list length to [`WORDLIST_LEN`].
fn oracle_wordlist() -> &'static [&'static str; WORDLIST_LEN] {
    Language::English.word_list()
}

/// The oracle's twelve-word phrase for `entropy`.
fn oracle(entropy: &[u8; ENTROPY_LEN]) -> Mnemonic {
    Mnemonic::from_entropy(entropy).expect("128 bits is a valid BIP-39 entropy length")
}

fn oracle_words(entropy: &[u8; ENTROPY_LEN]) -> Vec<String> {
    oracle(entropy).words().map(str::to_owned).collect()
}

fn entropy() -> impl Strategy<Value = [u8; ENTROPY_LEN]> {
    any::<[u8; ENTROPY_LEN]>()
}

/// A word position inside one half.
fn position() -> impl Strategy<Value = usize> {
    0..HALF_WORD_COUNT
}

/// Strings no BIP-39 English word can equal: every word is at most 8 letters,
/// which `word_list_matches_the_oracle_slot_for_slot` pins.
fn unknown_word() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z]{9,16}").expect("valid regex")
}

/// Whitespace runs that may separate or surround words.
fn spacing() -> impl Strategy<Value = Vec<&'static str>> {
    prop::collection::vec(
        prop::sample::select(&[" ", "  ", "\t", "\n"][..]),
        HALF_WORD_COUNT,
    )
}

/// Passphrases stay printable ASCII, which is already NFKD-normalized.
///
/// The empty passphrase is the one the rotation scheme uses, so it is drawn
/// far more often than a uniform string strategy would give it.
fn passphrase() -> impl Strategy<Value = String> {
    prop_oneof![
        1 => Just(String::new()),
        1 => prop::string::string_regex("[ -~]{1,24}").expect("valid regex"),
    ]
}

fn config(cases: u32) -> ProptestConfig {
    ProptestConfig {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
            "proptest-regressions",
        ))),
        ..ProptestConfig::default()
    }
}

/// The two lists have to agree slot for slot, since every index is a word
/// index. This also pins the eight-character bound [`unknown_word`] needs.
#[test]
fn word_list_matches_the_oracle_slot_for_slot() {
    assert_eq!(WORDLIST.as_slice(), oracle_wordlist().as_slice());

    for (index, word) in oracle_wordlist().iter().enumerate() {
        assert_eq!(word_index(word), u16::try_from(index).ok(), "{word}");
        assert!(word.len() <= 8, "{word}");
    }
}

proptest! {
    #![proptest_config(config(WORD_SLOT_CASES))]

    /// Encoding is the oracle's encoding, word for word.
    #[test]
    fn encodes_entropy_into_the_oracle_phrase(entropy in entropy()) {
        let half = Bip39Half::from_entropy(&entropy).expect("intact word list");
        let phrase = half.to_phrase();

        prop_assert_eq!(phrase.as_str(), oracle(&entropy).to_string());
    }

    /// Every phrase the oracle accepts decodes here, back to the same entropy.
    #[test]
    fn decodes_oracle_phrases_into_their_entropy(entropy in entropy()) {
        let phrase = oracle(&entropy).to_string();
        let half = Bip39Half::parse(&phrase).expect("the oracle only emits valid phrases");
        let decoded = half.to_entropy().expect("valid checksum");
        let expected = oracle_words(&entropy);

        prop_assert_eq!(decoded.as_slice(), &entropy);
        prop_assert_eq!(half.words(), expected.as_slice());
    }

    /// Swapping one word for another list word has to be accepted or rejected
    /// exactly as the oracle decides, including the one-in-sixteen swaps that
    /// still checksum.
    #[test]
    fn agrees_with_the_oracle_on_swapped_words(
        entropy in entropy(),
        position in position(),
        replacement in 0_usize..WORDLIST_LEN,
    ) {
        let mut words = oracle_words(&entropy);
        words[position] = oracle_wordlist()[replacement].to_owned();
        let phrase = words.join(" ");

        let ours = Bip39Half::parse(&phrase);
        let theirs = Mnemonic::parse_normalized(&phrase);
        prop_assert_eq!(ours.is_ok(), theirs.is_ok(), "{}", phrase);

        match ours {
            Ok(ours) => {
                let ours = ours.to_entropy().expect("valid checksum");
                let theirs = theirs.expect("both parsers just agreed").to_entropy();

                prop_assert_eq!(ours.as_slice(), theirs.as_slice());
            }
            // Every swapped-in word is a list word, so only the checksum can break.
            Err(error) => prop_assert!(matches!(error, MnemonicError::Checksum)),
        }
    }

    /// A word outside the list is reported as unknown, never as a checksum
    /// failure, so a caller can tell a typo from a corrupted phrase.
    #[test]
    fn rejects_words_outside_the_list(
        entropy in entropy(),
        position in position(),
        intruder in unknown_word(),
    ) {
        let mut words = oracle_words(&entropy);
        words[position] = intruder;
        let phrase = words.join(" ");

        prop_assert!(matches!(
            Bip39Half::parse(&phrase),
            Err(MnemonicError::UnknownWord)
        ));
        prop_assert!(Mnemonic::parse_normalized(&phrase).is_err());
    }

    /// Case and whitespace are input noise that the oracle rejects and the
    /// engine has to absorb, without changing the decoded entropy.
    #[test]
    fn normalizes_case_and_spacing_without_changing_the_entropy(
        entropy in entropy(),
        uppercase in prop::collection::vec(any::<bool>(), HALF_WORD_COUNT),
        spacing in spacing(),
    ) {
        let mut phrase = String::new();
        for ((word, upper), space) in oracle_words(&entropy).iter().zip(uppercase).zip(spacing) {
            phrase.push_str(space);
            phrase.push_str(&if upper { word.to_uppercase() } else { word.clone() });
        }

        let half = Bip39Half::parse(&phrase).expect("only case and spacing changed");
        let decoded = half.to_entropy().expect("valid checksum");

        prop_assert_eq!(decoded.as_slice(), &entropy);
    }

    /// A 24-word phrase splits into two halves that each decode on their own.
    #[test]
    fn rotation_halves_decode_independently(anchor in entropy(), signing in entropy()) {
        let phrase = format!("{} {}", oracle(&anchor), oracle(&signing));
        let mnemonic = RotationMnemonic::parse(&phrase).expect("two valid halves");
        let decoded_anchor = mnemonic.anchor().to_entropy().expect("valid checksum");
        let decoded_signing = mnemonic.signing().to_entropy().expect("valid checksum");

        prop_assert_eq!(decoded_anchor.as_slice(), &anchor);
        prop_assert_eq!(decoded_signing.as_slice(), &signing);
        prop_assert_eq!(mnemonic.is_pre_rotation(), anchor == signing);
    }

    /// The pre-rotation form is one half used twice, never a truncated phrase.
    #[test]
    fn twelve_words_expand_into_two_identical_halves(entropy in entropy()) {
        let phrase = oracle(&entropy).to_string();
        let mnemonic = RotationMnemonic::parse(&phrase).expect("one valid half");
        let rejoined = mnemonic.to_phrase();
        let expected = oracle_words(&entropy);

        prop_assert!(mnemonic.is_pre_rotation());
        prop_assert_eq!(mnemonic.anchor().words(), expected.as_slice());
        prop_assert_eq!(rejoined.as_str(), format!("{phrase} {phrase}"));
    }

    /// A 12-word oracle phrase is exactly a pre-rotation phrase: the TON and
    /// 24-word BIP-39 schemes need 24 words, so they can never match it.
    #[test]
    fn detection_reports_rotation_alone_for_every_oracle_half(entropy in entropy()) {
        prop_assert_eq!(
            detect_mnemonic_schemes(oracle_words(&entropy)),
            vec![MnemonicScheme::Rotation]
        );
    }

    /// No other length is a rotation phrase, however valid the words are.
    #[test]
    fn rejects_every_other_word_count(
        entropy in entropy(),
        count in 0_usize..=ROTATION_WORD_COUNT + HALF_WORD_COUNT,
    ) {
        prop_assume!(count != HALF_WORD_COUNT && count != ROTATION_WORD_COUNT);

        let source = oracle_words(&entropy);
        let words = source
            .iter()
            .cycle()
            .take(count)
            .map(String::as_str)
            .collect::<Vec<_>>();

        let rejected = RotationMnemonic::new(&words).err();

        prop_assert!(
            matches!(rejected, Some(MnemonicError::RotationWordCount { got }) if got == count),
            "{} words must be rejected",
            count
        );
    }
}

proptest! {
    #![proptest_config(config(BIP39_24_SWAP_CASES))]

    /// Every 24-word phrase the oracle emits is detected as BIP-39, and one
    /// swapped list word keeps or breaks detection exactly as the oracle
    /// decides, including the one-in-256 swaps that still checksum.
    #[test]
    fn detects_24_word_phrases_as_the_oracle_does(
        entropy in any::<[u8; BIP39_24_ENTROPY_LEN]>(),
        position in 0_usize..BIP39_24_WORD_COUNT,
        replacement in 0_usize..WORDLIST_LEN,
    ) {
        let oracle = Mnemonic::from_entropy(&entropy)
            .expect("256 bits is a valid BIP-39 entropy length");
        let mut words = oracle.words().map(str::to_owned).collect::<Vec<_>>();
        prop_assert!(is_bip39_24_phrase(
            &words.iter().map(String::as_str).collect::<Vec<_>>()
        ));

        words[position] = oracle_wordlist()[replacement].to_owned();
        let ours = is_bip39_24_phrase(&words.iter().map(String::as_str).collect::<Vec<_>>());
        let theirs = Mnemonic::parse_normalized(&words.join(" ")).is_ok();

        prop_assert_eq!(ours, theirs, "{}", words.join(" "));
    }
}

proptest! {
    #![proptest_config(config(WORD_SLOT_24_CASES))]

    /// Every 24-word phrase the oracle emits is detected as BIP-39 at the
    /// public scheme level, however the words are cased and padded.
    #[test]
    fn detection_reports_bip39_for_every_oracle_24_word_phrase(
        entropy in any::<[u8; BIP39_24_ENTROPY_LEN]>(),
        uppercase in prop::collection::vec(any::<bool>(), BIP39_24_WORD_COUNT),
        padding in prop::collection::vec(
            prop::sample::select(&["", " ", "  ", "\t"][..]),
            BIP39_24_WORD_COUNT,
        ),
    ) {
        let oracle = Mnemonic::from_entropy(&entropy)
            .expect("256 bits is a valid BIP-39 entropy length");
        let words = oracle
            .words()
            .zip(uppercase)
            .zip(padding)
            .map(|((word, upper), pad)| {
                let cased = if upper { word.to_uppercase() } else { word.to_owned() };
                format!("{pad}{cased}{pad}")
            })
            .collect::<Vec<_>>();

        prop_assert!(detect_mnemonic_schemes(words).contains(&MnemonicScheme::Bip39));
    }
}

proptest! {
    #![proptest_config(config(OTHER_LENGTH_CASES))]

    /// A valid BIP-39 phrase of 15, 18, or 21 words is no scheme at all:
    /// detection accepts BIP-39 only at 24 words, and no other scheme uses
    /// these lengths.
    #[test]
    fn oracle_phrases_of_other_lengths_report_no_scheme(
        entropy_len in prop::sample::select(&[20_usize, 24, 28][..]),
        bytes in prop::collection::vec(any::<u8>(), 28),
    ) {
        let oracle = Mnemonic::from_entropy(&bytes[..entropy_len])
            .expect("160, 192, and 224 bits are valid BIP-39 entropy lengths");

        prop_assert!(!is_bip39_24_phrase(&oracle.words().collect::<Vec<_>>()));
        prop_assert_eq!(
            detect_mnemonic_schemes(oracle.words().map(str::to_owned).collect()),
            vec![]
        );
    }
}

proptest! {
    #![proptest_config(config(ENTROPY_BYTE_CASES))]

    /// The seed is the oracle's seed, for the production salt and any other.
    #[test]
    fn derives_the_oracle_seed(entropy in entropy(), passphrase in passphrase()) {
        let half = Bip39Half::from_entropy(&entropy).expect("intact word list");
        let seed = half.to_seed(&passphrase);
        let expected = oracle(&entropy).to_seed_normalized(&passphrase);

        prop_assert_eq!(seed.as_slice(), expected.as_slice());
    }
}

proptest! {
    #![proptest_config(config(PAIRED_ENTROPY_BYTE_CASES))]

    /// Both rotation keys follow from the oracle seed of their own half, so
    /// the two halves never leak into each other's derivation.
    #[test]
    fn rotation_keys_follow_the_oracle_seeds(anchor in entropy(), signing in entropy()) {
        let phrase = format!("{} {}", oracle(&anchor), oracle(&signing));
        let mnemonic = RotationMnemonic::parse(&phrase).expect("two valid halves");
        let keys = derive_rotation_keys(&mnemonic);

        for (entropy, derived) in [(anchor, &keys.anchor), (signing, &keys.signing)] {
            let seed = oracle(&entropy).to_seed_normalized("");
            let expected = signing_key(&derive_path(&seed, &TON_ACCOUNT_PATH));

            prop_assert_eq!(derived.to_bytes(), expected.to_bytes());
        }
    }
}
