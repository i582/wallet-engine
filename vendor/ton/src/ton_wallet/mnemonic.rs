use crate::errors::TonError;
use ed25519_dalek::{KEYPAIR_LENGTH, PUBLIC_KEY_LENGTH, SECRET_KEY_LENGTH, SecretKey, SigningKey};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha2::{Digest, Sha512};
use std::collections::HashSet;
use std::sync::LazyLock;
use std::{cmp, convert::TryInto, fmt};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub const WORDLIST_EN: &str = include_str!("../../resources/mnemonics/wordlist_en.txt");
const PBKDF_ITERATIONS: u32 = 100000;

pub static WORDLIST_EN_SET: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| WORDLIST_EN.split('\n').filter(|w| !w.is_empty()).collect());

pub struct Mnemonic {
    words: Vec<String>,
    password: Option<String>,
}

impl Drop for Mnemonic {
    fn drop(&mut self) {
        self.words.zeroize();
        self.password.zeroize();
    }
}

#[derive(PartialEq, Eq, Hash, Zeroize, ZeroizeOnDrop)]
pub struct KeyPair {
    pub public_key: [u8; PUBLIC_KEY_LENGTH],
    pub secret_key: [u8; KEYPAIR_LENGTH],
}

impl fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("KeyPair")
            .field("public_key", &self.public_key)
            .field("secret_key", &"***REDACTED***")
            .finish()
    }
}

impl Mnemonic {
    pub fn new(words: Vec<&str>, password: Option<String>) -> Result<Mnemonic, TonError> {
        let normalized_words: Vec<String> = words.iter().map(|w| w.trim().to_lowercase()).collect();
        let mnemonic = Mnemonic {
            words: normalized_words,
            password,
        };

        // Check words
        if mnemonic.words.len() != 24 {
            return Err(TonError::MnemonicWordsCount(mnemonic.words.len()));
        }
        for word in &mnemonic.words {
            if !WORDLIST_EN_SET.contains(word.as_str()) {
                return Err(TonError::MnemonicWord(word.clone()));
            }
        }

        // Check password validity
        match mnemonic.password.as_deref() {
            Some(s) if !s.is_empty() => {
                let passless_entropy = to_entropy(&mnemonic.words, None)?;
                let seed = pbkdf2_sha512(&passless_entropy, "TON fast seed version", 1, 64);
                if seed[0] != 1 {
                    return Err(TonError::MnemonicFirstByte(seed[0]));
                }
                // Make that this also is not a valid passwordless mnemonic
                let entropy = to_entropy(&mnemonic.words, mnemonic.password.as_deref())?;
                let seed = pbkdf2_sha512(
                    &entropy,
                    "TON seed version",
                    cmp::max(1, PBKDF_ITERATIONS / 256),
                    64,
                );
                if seed[0] == 0 {
                    return Err(TonError::MnemonicFirstByte(seed[0]));
                }
            }
            _ => {
                let entropy = to_entropy(&mnemonic.words, None)?;
                let seed = pbkdf2_sha512(
                    &entropy,
                    "TON seed version",
                    cmp::max(1, PBKDF_ITERATIONS / 256),
                    64,
                );
                if seed[0] != 0 {
                    return Err(TonError::MnemonicFirstBytePassless(seed[0]));
                }
            }
        }

        Ok(mnemonic)
    }

    pub fn from_str(s: &str, password: Option<String>) -> Result<Mnemonic, TonError> {
        let words: Vec<&str> = s
            .split(' ')
            .map(|w| w.trim())
            .filter(|w| !w.is_empty())
            .collect();
        Mnemonic::new(words, password)
    }

    pub fn to_key_pair(&self) -> Result<KeyPair, TonError> {
        let entropy = to_entropy(&self.words, self.password.as_deref())?;
        let seed = pbkdf2_sha512(&entropy, "TON default seed", PBKDF_ITERATIONS, 64);

        let secret_key_bytes: &SecretKey = seed
            .get(..SECRET_KEY_LENGTH)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| {
                TonError::Custom(format!(
                    "Invalid Ed25519 secret key length: got {}, expected {}",
                    seed.len(),
                    SECRET_KEY_LENGTH
                ))
            })?;

        let signing_key = SigningKey::from_bytes(secret_key_bytes);
        Ok(KeyPair {
            public_key: signing_key.verifying_key().to_bytes(),
            secret_key: signing_key.to_keypair_bytes(),
        })
    }
}

fn to_entropy(words: &[String], password: Option<&str>) -> Result<Zeroizing<Vec<u8>>, TonError> {
    let key = mnemonic_hmac_key(words);
    let mut mac = Hmac::<Sha512>::new_from_slice(key.as_ref())?;
    if let Some(s) = password {
        mac.update(s.as_bytes());
    }
    let mut code_bytes = mac.finalize().into_bytes();
    let entropy = Zeroizing::new(code_bytes.as_slice().to_vec());
    code_bytes.as_mut_slice().zeroize();
    Ok(entropy)
}

fn mnemonic_hmac_key(words: &[String]) -> Zeroizing<[u8; 128]> {
    let phrase_len = words.iter().map(String::len).sum::<usize>() + words.len().saturating_sub(1);
    let mut key = Zeroizing::new([0_u8; 128]);

    if phrase_len > key.len() {
        let mut digest = Sha512::new();
        update_phrase(words, |part| digest.update(part));
        let mut hashed = digest.finalize();
        key[..hashed.len()].copy_from_slice(&hashed);
        hashed.as_mut_slice().zeroize();
    } else {
        let mut offset = 0;
        update_phrase(words, |part| {
            let end = offset + part.len();
            key[offset..end].copy_from_slice(part);
            offset = end;
        });
    }

    key
}

fn update_phrase(words: &[String], mut update: impl FnMut(&[u8])) {
    for (index, word) in words.iter().enumerate() {
        if index != 0 {
            update(b" ");
        }
        update(word.as_bytes());
    }
}

fn pbkdf2_sha512(
    key: &[u8],
    salt: &str,
    rounds: u32,
    output_len_bytes: usize,
) -> Zeroizing<Vec<u8>> {
    let mut output = Zeroizing::new(vec![0_u8; output_len_bytes]);
    pbkdf2_hmac::<Sha512>(key, salt.as_bytes(), rounds, &mut output);
    output
}

///Based on https://github.com/tonwhales/ton-crypto/blob/master/src/mnemonic/mnemonic.spec.ts
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnemonic_parse_works() -> anyhow::Result<()> {
        let words = "dose ice enrich trigger test dove century still betray gas diet dune use other base gym mad law immense village world example praise game";
        let mnemonic = Mnemonic::from_str(words, None);
        assert!(mnemonic.is_ok());

        let words = " dose ice enrich trigger test dove \
        century still betray gas diet       dune use other base gym mad law \
        immense village world example praise game ";
        let mnemonic = Mnemonic::from_str(words, None);
        assert!(mnemonic.is_ok());
        Ok(())
    }

    #[test]
    fn mnemonic_validate_works() -> anyhow::Result<()> {
        let mnemonic = Mnemonic::new(
            vec![
                "dose", "ice", "enrich", "trigger", "test", "dove", "century", "still", "betray",
                "gas", "diet", "dune",
            ],
            None,
        );
        assert!(mnemonic.is_err());
        let mnemonic = Mnemonic::new(vec!["a"], None);
        assert!(mnemonic.is_err());
        Ok(())
    }

    #[test]
    fn mnemonic_to_private_key_works() -> anyhow::Result<()> {
        let mnemonic = Mnemonic::new(
            vec![
                "dose", "ice", "enrich", "trigger", "test", "dove", "century", "still", "betray",
                "gas", "diet", "dune", "use", "other", "base", "gym", "mad", "law", "immense",
                "village", "world", "example", "praise", "game",
            ],
            None,
        )?;
        let expected = "119dcf2840a3d56521d260b2f125eedc0d4f3795b9e627269a4b5a6dca8257bdc04ad1885c127fe863abb00752fa844e6439bb04f264d70de7cea580b32637ab";

        let kp = mnemonic.to_key_pair()?;
        let res = hex::encode(&kp.secret_key);

        assert_eq!(res, expected);

        Ok(())
    }
}
