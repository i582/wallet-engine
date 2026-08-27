//! The two BIP-39 halves of a rotation mnemonic.
//!
//! A rotation mnemonic is 24 words made of two 12-word BIP-39 mnemonics: an
//! anchor half that fixes the account address, and a signing half that is
//! replaced on rotation. The halves are never joined into a single 24-word
//! mnemonic and their entropies are never combined.
//!
//! Before the wallet's one-time key rotation the two halves are identical, so
//! the user holds a single 12-word phrase. [`RotationMnemonic::parse`] accepts
//! that form and expands it; applications never duplicate words themselves.
//!
//! <https://github.com/ton-blockchain/TEPs/blob/master/text/0003-wallets.md#33-rotation-mnemonic>
//!
//! Twelve words is the only BIP-39 length the engine derives keys from. For
//! scheme detection only, [`is_bip39_24_phrase`] recognizes the standalone
//! 24-word BIP-39 form - the Multichain mnemonic of TEP-0003 - and
//! [`is_ton_mnemonic`] recognizes the legacy TON mnemonic. The remaining
//! BIP-39 lengths (15, 18, and 21 words) appear in no supported scheme and are
//! not implemented.
//!
//! This module owns the word encoding only. Key derivation lives in
//! [`super::crypto`].
use std::fmt;
use std::sync::LazyLock;

use pbkdf2::pbkdf2_hmac;
use sha2::{Digest as _, Sha256, Sha512};
use ton::ton_wallet::{Mnemonic as TonMnemonic, WORDLIST_EN_SET};
use zeroize::{Zeroize as _, Zeroizing};

/// Words in one BIP-39 half of a rotation mnemonic.
pub(crate) const HALF_WORD_COUNT: usize = 12;

/// Total words in a rotation mnemonic phrase.
pub(crate) const ROTATION_WORD_COUNT: usize = HALF_WORD_COUNT * 2;

/// Entropy bytes behind a 12-word phrase: 128 bits.
pub(crate) const ENTROPY_LEN: usize = 16;

/// Words in a standalone 24-word BIP-39 (Multichain) phrase.
///
/// Numerically equal to [`ROTATION_WORD_COUNT`], but the two schemes pack
/// their bits differently: one 256-bit entropy block with an 8-bit checksum
/// versus two independently checksummed 128-bit halves.
const BIP39_24_WORD_COUNT: usize = 24;

/// Entropy bytes behind a 24-word phrase: 256 bits.
const BIP39_24_ENTROPY_LEN: usize = 32;

/// Words in the BIP-39 English list. Each word therefore encodes 11 bits.
const WORDLIST_LEN: usize = 2048;

/// Bits encoded per BIP-39 word.
const BITS_PER_WORD: usize = 11;

/// Checksum bits BIP-39 appends to 128 entropy bits: one per 32 bits.
const CHECKSUM_BITS: usize = 4;

/// Mask selecting the leading [`CHECKSUM_BITS`] of a byte.
const CHECKSUM_MASK: u8 = !(0xff_u8 >> CHECKSUM_BITS);

/// Bytes in a BIP-39 seed.
pub(crate) const SEED_LEN: usize = 64;

/// PBKDF2 rounds BIP-39 specifies for the seed.
const SEED_ROUNDS: u32 = 2048;

/// Salt prefix BIP-39 puts in front of the passphrase.
const SEED_SALT_PREFIX: &str = "mnemonic";

/// A failure while reading or writing a BIP-39 half.
///
/// No variant carries a word or any entropy byte, so these values are safe to
/// log alongside the rest of the engine's diagnostics.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MnemonicError {
    #[error("a BIP-39 half needs {expected} words, got {got}")]
    WordCount { expected: usize, got: usize },
    #[error("phrase contains a word outside the BIP-39 English list")]
    UnknownWord,
    #[error("BIP-39 checksum mismatch")]
    Checksum,
    #[error(
        "a rotation phrase needs {HALF_WORD_COUNT} words before rotation or \
         {ROTATION_WORD_COUNT} after it, got {got}"
    )]
    RotationWordCount { got: usize },
    #[error("the BIP-39 English word list is not {WORDLIST_LEN} words")]
    WordList,
}

/// The BIP-39 English word list in index order.
///
/// The vendored TON word list is the canonical BIP-39 English list, which is
/// ASCII-sorted, so sorting recovers the BIP-39 index of every word.
/// `wordlist_is_the_canonical_bip39_english_list` pins that assumption.
static WORDLIST: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut words = WORDLIST_EN_SET.iter().copied().collect::<Vec<_>>();
    words.sort_unstable();
    words
});

/// Returns the BIP-39 index of `word`, or `None` when it is not in the list.
fn word_index(word: &str) -> Option<u16> {
    WORDLIST
        .binary_search(&word)
        .ok()
        .and_then(|index| u16::try_from(index).ok())
}

/// First byte of `SHA-256(entropy)`, which carries the BIP-39 checksum bits.
fn checksum_byte(entropy: &[u8]) -> u8 {
    let mut digest = Sha256::digest(entropy);
    let checksum = digest.first().copied().unwrap_or_default();
    digest.as_mut_slice().zeroize();
    checksum
}

/// Writes the low [`BITS_PER_WORD`] bits of `value` MSB-first at bit `offset`.
#[allow(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    reason = "offset stays inside a buffer sized for all twelve words"
)]
fn write_word(buf: &mut [u8], offset: usize, value: u16) {
    for bit in 0..BITS_PER_WORD {
        if value >> (BITS_PER_WORD - 1 - bit) & 1 == 1 {
            let position = offset + bit;
            if let Some(byte) = buf.get_mut(position / 8) {
                *byte |= 1 << (7 - position % 8);
            }
        }
    }
}

/// Reads [`BITS_PER_WORD`] bits MSB-first from bit `offset`.
#[allow(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    reason = "eleven bits cannot overflow a u16 and the shifts stay in range"
)]
fn read_word(buf: &[u8], offset: usize) -> u16 {
    (0..BITS_PER_WORD).fold(0_u16, |value, bit| {
        let position = offset + bit;
        let set = buf
            .get(position / 8)
            .map_or(0, |byte| byte >> (7 - position % 8) & 1);
        value << 1 | u16::from(set)
    })
}

/// A checksummed 12-word BIP-39 phrase: one half of a rotation mnemonic.
///
/// The TON mnemonic of `ton::ton_wallet::Mnemonic` shares this word list but
/// has no checksum and its own key derivation. This type is plain BIP-39 and
/// carries no password.
pub(crate) struct Bip39Half {
    words: Vec<String>,
}

impl Drop for Bip39Half {
    fn drop(&mut self) {
        self.words.zeroize();
    }
}

impl fmt::Debug for Bip39Half {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bip39Half")
            .field("words", &"***REDACTED***")
            .finish()
    }
}

impl Bip39Half {
    /// Normalizes and validates twelve words against the list and the checksum.
    pub(crate) fn new(words: &[&str]) -> Result<Self, MnemonicError> {
        if words.len() != HALF_WORD_COUNT {
            return Err(MnemonicError::WordCount {
                expected: HALF_WORD_COUNT,
                got: words.len(),
            });
        }

        let normalized = words
            .iter()
            .map(|word| word.trim().to_lowercase())
            .collect::<Vec<_>>();

        if normalized.iter().any(|word| word_index(word).is_none()) {
            return Err(MnemonicError::UnknownWord);
        }

        let half = Self { words: normalized };
        let _entropy = half.to_entropy()?;

        Ok(half)
    }

    /// Splits a whitespace-separated phrase and validates it.
    #[cfg(test)]
    pub(crate) fn parse(phrase: &str) -> Result<Self, MnemonicError> {
        Self::new(&phrase.split_whitespace().collect::<Vec<_>>())
    }

    /// The normalized words, in order.
    #[cfg(test)]
    pub(crate) fn words(&self) -> &[String] {
        &self.words
    }

    /// Joins the words into a single space-separated phrase.
    pub(crate) fn to_phrase(&self) -> Zeroizing<String> {
        Zeroizing::new(self.words.join(" "))
    }

    /// Derives the BIP-39 seed that SLIP-0010 derivation starts from.
    ///
    /// PBKDF2-HMAC-SHA512 over the phrase, salted with `"mnemonic"` followed
    /// by `passphrase`, for [`SEED_ROUNDS`] rounds. Pass an empty string for
    /// the passphraseless form the rotation scheme uses; the parameter exists
    /// so tests can reproduce the official vectors, which use `"TREZOR"`.
    ///
    /// BIP-39 requires both inputs to be NFKD-normalized. The English word
    /// list is ASCII, so the phrase is already normalized. A caller that ever
    /// passes a non-empty passphrase has to normalize it first.
    pub(crate) fn to_seed(&self, passphrase: &str) -> Zeroizing<[u8; SEED_LEN]> {
        let phrase = self.to_phrase();
        let mut salt = Zeroizing::new(String::from(SEED_SALT_PREFIX));
        salt.push_str(passphrase);

        let mut seed = Zeroizing::new([0_u8; SEED_LEN]);
        pbkdf2_hmac::<Sha512>(
            phrase.as_bytes(),
            salt.as_bytes(),
            SEED_ROUNDS,
            seed.as_mut_slice(),
        );

        seed
    }

    /// Decodes the phrase into its entropy, verifying the checksum.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "index is below twelve, so the bit offset cannot overflow"
    )]
    pub(crate) fn to_entropy(&self) -> Result<Zeroizing<Vec<u8>>, MnemonicError> {
        let mut packed = Zeroizing::new(vec![0_u8; ENTROPY_LEN + 1]);
        for (index, word) in self.words.iter().enumerate() {
            let value = word_index(word).ok_or(MnemonicError::UnknownWord)?;
            write_word(&mut packed, index * BITS_PER_WORD, value);
        }

        let (entropy, trailing) = packed.split_at(ENTROPY_LEN);
        let entropy = Zeroizing::new(entropy.to_vec());
        let actual = trailing.first().copied().unwrap_or_default();

        if actual & CHECKSUM_MASK != checksum_byte(&entropy) & CHECKSUM_MASK {
            return Err(MnemonicError::Checksum);
        }

        Ok(entropy)
    }

    /// Encodes entropy into a phrase. Inverse of [`Self::to_entropy`].
    ///
    /// The fixed-size input makes every length valid, so the only failure is a
    /// word list that is not the expected 2048 words.
    #[allow(
        clippy::arithmetic_side_effects,
        reason = "index is below twelve, so the bit offset cannot overflow"
    )]
    pub(crate) fn from_entropy(entropy: &[u8; ENTROPY_LEN]) -> Result<Self, MnemonicError> {
        let mut packed = Zeroizing::new(Vec::with_capacity(ENTROPY_LEN + 1));
        packed.extend_from_slice(entropy);
        packed.push(checksum_byte(entropy) & CHECKSUM_MASK);

        let words = (0..HALF_WORD_COUNT)
            .map(|index| {
                let value = read_word(&packed, index * BITS_PER_WORD);
                WORDLIST
                    .get(usize::from(value))
                    .map(|word| (*word).to_owned())
                    .ok_or(MnemonicError::WordList)
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self { words })
    }
}

/// A [`ROTATION_WORD_COUNT`]-word phrase holding two independent BIP-39 halves.
///
/// The anchor half fixes the wallet account address and authorizes the first
/// rotation. The signing half signs ordinary outgoing messages and is replaced
/// on rotation.
pub(crate) struct RotationMnemonic {
    anchor: Bip39Half,
    signing: Bip39Half,
}

impl fmt::Debug for RotationMnemonic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RotationMnemonic")
            .field("anchor", &"***REDACTED***")
            .field("signing", &"***REDACTED***")
            .finish()
    }
}

impl RotationMnemonic {
    /// Validates a rotation phrase and splits it into its two halves.
    ///
    /// Accepts both phrase forms: [`ROTATION_WORD_COUNT`] words after the
    /// wallet's key rotation, or [`HALF_WORD_COUNT`] words before it, in
    /// which case the single half serves as both anchor and signing half.
    pub(crate) fn new(words: &[&str]) -> Result<Self, MnemonicError> {
        match words.len() {
            HALF_WORD_COUNT => Ok(Self::from_single_half(Bip39Half::new(words)?)),
            ROTATION_WORD_COUNT => {
                let (anchor, signing) = words.split_at(HALF_WORD_COUNT);
                Ok(Self::from_halves(
                    Bip39Half::new(anchor)?,
                    Bip39Half::new(signing)?,
                ))
            }
            got => Err(MnemonicError::RotationWordCount { got }),
        }
    }

    /// Builds the pre-rotation mnemonic: one half used for both keys.
    ///
    /// Until the wallet's one-time key rotation the signing key equals the
    /// anchor key, so the user holds a single 12-word phrase. The engine owns
    /// this expansion; applications never duplicate the phrase themselves.
    pub(crate) fn from_single_half(half: Bip39Half) -> Self {
        let signing = Bip39Half {
            words: half.words.clone(),
        };

        Self {
            anchor: half,
            signing,
        }
    }

    /// Whether both halves are identical, i.e. the key was never rotated.
    pub(crate) fn is_pre_rotation(&self) -> bool {
        self.anchor.words == self.signing.words
    }

    /// Splits a whitespace-separated phrase and validates it.
    pub(crate) fn parse(phrase: &str) -> Result<Self, MnemonicError> {
        Self::new(&phrase.split_whitespace().collect::<Vec<_>>())
    }

    /// Assembles a rotation mnemonic from two already-validated halves.
    pub(crate) const fn from_halves(anchor: Bip39Half, signing: Bip39Half) -> Self {
        Self { anchor, signing }
    }

    /// Words 1-12, which derive the anchor key.
    pub(crate) const fn anchor(&self) -> &Bip39Half {
        &self.anchor
    }

    /// Words 13-24, which derive the signing key.
    pub(crate) const fn signing(&self) -> &Bip39Half {
        &self.signing
    }

    /// Joins both halves back into one 24-word phrase.
    #[cfg(test)]
    pub(crate) fn to_phrase(&self) -> Zeroizing<String> {
        let mut phrase = Zeroizing::new(String::new());
        phrase.push_str(&self.anchor.to_phrase());
        phrase.push(' ');
        phrase.push_str(&self.signing.to_phrase());
        phrase
    }
}

/// Reports whether `words` form one checksummed 24-word BIP-39 mnemonic.
///
/// This is the Multichain mnemonic of
/// [TEP-0003 section 3.2](https://github.com/ton-blockchain/TEPs/blob/master/text/0003-wallets.md#32-multichain-mnemonic):
/// 256 entropy bits followed by an 8-bit checksum, encoded as one 24-word
/// phrase. Words are normalized like [`Bip39Half::new`].
///
/// Detection only: the engine never derives a seed or key from this scheme.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "index is below twenty-four, so the bit offset cannot overflow"
)]
pub(crate) fn is_bip39_24_phrase(words: &[&str]) -> bool {
    if words.len() != BIP39_24_WORD_COUNT {
        return false;
    }

    let mut packed = Zeroizing::new(vec![0_u8; BIP39_24_ENTROPY_LEN + 1]);
    for (index, word) in words.iter().enumerate() {
        let normalized = Zeroizing::new(word.trim().to_lowercase());
        let Some(value) = word_index(&normalized) else {
            return false;
        };
        write_word(&mut packed, index * BITS_PER_WORD, value);
    }

    let (entropy, trailing) = packed.split_at(BIP39_24_ENTROPY_LEN);
    trailing.first().copied() == Some(checksum_byte(entropy))
}

/// Reports whether `words` form a passwordless 24-word TON mnemonic.
///
/// TON mnemonics
/// ([TEP-0003 section 3.1](https://github.com/ton-blockchain/TEPs/blob/master/text/0003-wallets.md#31-ton-mnemonic))
/// draw from the same English word list but carry no positional checksum;
/// validity is a first-byte condition on a PBKDF2 seed, checked by the
/// vendored `ton` implementation. A password-protected TON mnemonic fails
/// this check by design: without the password its validity is not observable.
///
/// Detection only: the engine never derives a key pair from this scheme.
pub(crate) fn is_ton_mnemonic(words: &[&str]) -> bool {
    TonMnemonic::new(words.to_vec(), None).is_ok()
}

#[cfg(test)]
mod proptests;

#[cfg(test)]
mod tests {
    use super::*;

    /// Official 12-word BIP-39 English vectors from `trezor/python-mnemonic`.
    ///
    /// Entropy, phrase, and the seed those vectors derive with the `"TREZOR"`
    /// passphrase.
    const VECTORS: &[([u8; ENTROPY_LEN], &str, &str)] = &[
        (
            [
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00,
            ],
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "c55257c360c07c72029aebc1b53c05ed0362ada38ead3e3e9efa3708e53495531f09a6987599d18264c1e1c92f2cf141630c7a3c4ab7c81b2f001698e7463b04",
        ),
        (
            [
                0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f, 0x7f,
                0x7f, 0x7f,
            ],
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
            "2e8905819b8723fe2c1d161860e5ee1830318dbf49a83bd451cfb8440c28bd6fa457fe1296106559a3c80937a1c1069be3a3a5bd381ee6260e8d9739fce1f607",
        ),
        (
            [
                0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80,
                0x80, 0x80,
            ],
            "letter advice cage absurd amount doctor acoustic avoid letter advice cage above",
            "d71de856f81a8acc65e6fc851a38d4d7ec216fd0796d0a6827a3ad6ed5511a30fa280f12eb2e47ed2ac03b5c462a0358d18d69fe4f985ec81778c1b370b652a8",
        ),
        (
            [
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                0xff, 0xff,
            ],
            "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong",
            "ac27495480225222079d7be181583751e86f571027b0497b5b5d11218e0a8a13332572917f0f8e5a589620c6f15b11c61dee327651a14c34e18231052e48c069",
        ),
        (
            [
                0x9e, 0x88, 0x5d, 0x95, 0x2a, 0xd3, 0x62, 0xca, 0xeb, 0x4e, 0xfe, 0x34, 0xa8, 0xe9,
                0x1b, 0xd2,
            ],
            "ozone drill grab fiber curtain grace pudding thank cruise elder eight picnic",
            "274ddc525802f7c828d8ef7ddbcdc5304e87ac3535913611fbbfa986d0c9e5476c91689f9c8a54fd55bd38606aa6a8595ad213d4c9c9f9aca3fb217069a41028",
        ),
        (
            [
                0xc0, 0xba, 0x5a, 0x8e, 0x91, 0x41, 0x11, 0x21, 0x0f, 0x2b, 0xd1, 0x31, 0xf3, 0xd5,
                0xe0, 0x8d,
            ],
            "scheme spot photo card baby mountain device kick cradle pact join borrow",
            "ea725895aaae8d4c1cf682c1bfd2d358d52ed9f0f0591131b559e2724bb234fca05aa9c02c57407e04ee9dc3b454aa63fbff483a8b11de949624b9f1831a9612",
        ),
        (
            [
                0x23, 0xdb, 0x81, 0x60, 0xa3, 0x1d, 0x3e, 0x0d, 0xca, 0x36, 0x88, 0xed, 0x94, 0x1a,
                0xdb, 0xf3,
            ],
            "cat swing flag economy stadium alone churn speed unique patch report train",
            "deb5f45449e615feff5640f2e49f933ff51895de3b4381832b3139941c57b59205a42480c52175b6efcffaa58a2503887c1e8b363a707256bdd2b587b46541f5",
        ),
        (
            [
                0xf3, 0x0f, 0x8c, 0x1d, 0xa6, 0x65, 0x47, 0x8f, 0x49, 0xb0, 0x01, 0xd9, 0x4c, 0x5f,
                0xc4, 0x52,
            ],
            "vessel ladder alter error federal sibling chat ability sun glass valve picture",
            "2aaa9242daafcee6aa9d7269f17d4efe271e1b9a529178d7dc139cd18747090bf9d60295d0ce74309a78852a9caadf0af48aae1c6253839624076224374bc63f",
        ),
    ];

    /// An anchor half of all-zero entropy and a signing half of all-one entropy.
    const ROTATION_PHRASE: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about \
                                   zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong";

    /// Pins the vendored word list to the canonical BIP-39 English list.
    ///
    /// Index lookup assumes the two are identical. A re-vendored `ton` crate
    /// that changes the file has to fail here rather than silently shift every
    /// BIP-39 index.
    #[test]
    fn wordlist_is_the_canonical_bip39_english_list() {
        const CANONICAL_SHA256: &str =
            "2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda";

        assert_eq!(WORDLIST.len(), WORDLIST_LEN);
        assert!(WORDLIST.is_sorted());

        let mut joined = String::new();
        for word in WORDLIST.iter() {
            joined.push_str(word);
            joined.push('\n');
        }
        assert_eq!(hex(&Sha256::digest(joined.as_bytes())), CANONICAL_SHA256);
        assert_eq!(word_index("abandon"), Some(0));
        assert_eq!(word_index("zoo"), Some(2047));
        assert_eq!(word_index("notaword"), None);
    }

    /// The four constants that describe a 12-word phrase must stay consistent.
    #[test]
    fn twelve_words_encode_the_entropy_and_its_checksum() {
        assert_eq!(
            ENTROPY_LEN * 8 + CHECKSUM_BITS,
            HALF_WORD_COUNT * BITS_PER_WORD
        );
        assert_eq!(CHECKSUM_BITS * 32, ENTROPY_LEN * 8);
        assert_eq!(CHECKSUM_MASK.count_ones(), 4);
        assert_eq!(CHECKSUM_MASK.trailing_zeros(), 4);
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn round_trips_the_official_vectors() {
        for (entropy, phrase, _seed) in VECTORS {
            let encoded = Bip39Half::from_entropy(entropy).expect("intact word list");
            assert_eq!(encoded.to_phrase().as_str(), *phrase);

            let decoded = Bip39Half::parse(phrase).expect("valid phrase");
            assert_eq!(
                decoded.to_entropy().expect("valid checksum").as_slice(),
                entropy
            );
        }
    }

    /// The official seeds use the `"TREZOR"` passphrase.
    #[test]
    fn derives_the_official_seeds() {
        for (_entropy, phrase, seed) in VECTORS {
            let half = Bip39Half::parse(phrase).expect("valid phrase");
            assert_eq!(hex(half.to_seed("TREZOR").as_slice()), *seed, "{phrase}");
        }
    }

    /// The rotation scheme derives without a passphrase, which changes the salt.
    #[test]
    fn derives_the_passphraseless_seed() {
        const EXPECTED: &str = "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc1\
                                9a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4";

        let half = Bip39Half::from_entropy(&[0x00; ENTROPY_LEN]).expect("intact word list");
        let seed = half.to_seed("");

        assert_eq!(hex(seed.as_slice()), EXPECTED.replace(' ', ""));
        assert_ne!(seed.as_slice(), half.to_seed("TREZOR").as_slice());
    }

    #[test]
    fn rejects_bad_checksums_lengths_and_words() {
        // Valid words, last one swapped so the checksum no longer matches.
        let broken = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon zoo";
        assert!(matches!(
            Bip39Half::parse(broken),
            Err(MnemonicError::Checksum)
        ));
        assert!(matches!(
            Bip39Half::new(&["notaword"; HALF_WORD_COUNT]),
            Err(MnemonicError::UnknownWord)
        ));
        assert!(matches!(
            Bip39Half::new(&["abandon"; 24]),
            Err(MnemonicError::WordCount {
                expected: 12,
                got: 24
            })
        ));
    }

    #[test]
    fn normalizes_case_and_whitespace() {
        let messy = "  ABANDON abandon   Abandon abandon abandon abandon \
                     abandon abandon abandon abandon abandon ABOUT  ";
        let half = Bip39Half::parse(messy).expect("valid phrase");
        assert_eq!(
            half.to_entropy().expect("valid checksum").as_slice(),
            [0_u8; ENTROPY_LEN].as_slice()
        );
    }

    #[test]
    fn rotation_mnemonic_rejects_malformed_phrases() {
        for got in [11, 13, 23, 25] {
            assert!(
                matches!(
                    RotationMnemonic::new(&vec!["abandon"; got]),
                    Err(MnemonicError::RotationWordCount { got: reported }) if reported == got
                ),
                "{got} words must be rejected"
            );
        }

        // Accepted lengths, but no valid checksum in any half.
        assert!(matches!(
            RotationMnemonic::new(&["abandon"; ROTATION_WORD_COUNT]),
            Err(MnemonicError::Checksum)
        ));
        assert!(matches!(
            RotationMnemonic::new(&["abandon"; HALF_WORD_COUNT]),
            Err(MnemonicError::Checksum)
        ));
    }

    /// The 12-word pre-rotation form expands into two identical halves.
    #[test]
    fn twelve_word_phrase_expands_to_identical_halves() {
        let phrase = "abandon abandon abandon abandon abandon abandon \
                      abandon abandon abandon abandon abandon about";
        let mnemonic = RotationMnemonic::parse(phrase).expect("valid pre-rotation phrase");

        assert!(mnemonic.is_pre_rotation());
        assert_eq!(mnemonic.anchor().words(), mnemonic.signing().words());
        assert_eq!(
            mnemonic.to_phrase().split_whitespace().count(),
            ROTATION_WORD_COUNT
        );

        let rotated = RotationMnemonic::parse(ROTATION_PHRASE).expect("valid rotation phrase");
        assert!(!rotated.is_pre_rotation());
    }

    #[test]
    fn rotation_halves_decode_independently() {
        let mnemonic = RotationMnemonic::parse(ROTATION_PHRASE).expect("valid rotation phrase");

        let anchor = mnemonic.anchor().to_entropy().expect("valid checksum");
        let signing = mnemonic.signing().to_entropy().expect("valid checksum");

        assert_eq!(anchor.as_slice(), [0x00_u8; ENTROPY_LEN].as_slice());
        assert_eq!(signing.as_slice(), [0xff_u8; ENTROPY_LEN].as_slice());
        assert_eq!(mnemonic.anchor().words().len(), HALF_WORD_COUNT);
        assert_eq!(mnemonic.signing().words().len(), HALF_WORD_COUNT);
        assert_eq!(
            mnemonic.to_phrase().split_whitespace().count(),
            ROTATION_WORD_COUNT
        );
    }

    /// Official 24-word BIP-39 English phrases from `trezor/python-mnemonic`.
    const BIP39_24_VECTORS: &[&str] = &[
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon \
         abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art",
        "legal winner thank year wave sausage worth useful legal winner thank year \
         wave sausage worth useful legal winner thank year wave sausage worth title",
        "letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd \
         amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic bless",
        "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote",
    ];

    /// The passwordless TON mnemonic from the vendored `ton` crate's tests.
    const TON_MNEMONIC: &str = "dose ice enrich trigger test dove century still betray \
                                gas diet dune use other base gym mad law immense village \
                                world example praise game";

    fn split(phrase: &str) -> Vec<&str> {
        phrase.split_whitespace().collect()
    }

    #[test]
    fn recognizes_official_24_word_bip39_phrases() {
        for phrase in BIP39_24_VECTORS {
            assert!(is_bip39_24_phrase(&split(phrase)), "{phrase}");
        }

        let messy = "  ZOO zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo \
                     zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo Vote  ";
        assert!(is_bip39_24_phrase(&split(messy)));
    }

    #[test]
    fn rejects_non_bip39_24_word_inputs() {
        // Valid words, but the trailing checksum byte does not match.
        assert!(!is_bip39_24_phrase(&["zoo"; BIP39_24_WORD_COUNT]));
        // A valid 12-word half is a different length.
        assert!(!is_bip39_24_phrase(&split(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
        )));
        assert!(!is_bip39_24_phrase(&["notaword"; BIP39_24_WORD_COUNT]));
        // A rotation phrase packs two 4-bit half checksums, not one 8-bit one.
        assert!(!is_bip39_24_phrase(&split(ROTATION_PHRASE)));
    }

    #[test]
    fn recognizes_passwordless_ton_mnemonics() {
        assert!(is_ton_mnemonic(&split(TON_MNEMONIC)));

        let messy = TON_MNEMONIC.to_uppercase();
        assert!(is_ton_mnemonic(&split(&messy)));

        // TON mnemonics are always 24 words.
        let truncated = &split(TON_MNEMONIC)[..HALF_WORD_COUNT];
        assert!(!is_ton_mnemonic(truncated));
        assert!(!is_ton_mnemonic(&["notaword"; BIP39_24_WORD_COUNT]));
        // Valid words whose seed fails the passwordless first-byte condition.
        assert!(!is_ton_mnemonic(&["zoo"; BIP39_24_WORD_COUNT]));
    }

    #[test]
    fn debug_redacts_words() {
        let mnemonic = RotationMnemonic::parse(ROTATION_PHRASE).expect("valid rotation phrase");
        for rendered in [format!("{mnemonic:?}"), format!("{:?}", mnemonic.anchor())] {
            assert!(rendered.contains("REDACTED"));
            assert!(!rendered.contains("abandon"));
        }
    }
}
