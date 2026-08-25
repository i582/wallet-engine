use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac as _};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha512;
use ton::ton_wallet::KeyPair;

/// A stable rotation mnemonic and the testnet wallet address derived from it.
///
/// Keep these values together. Changing one without the other would make
/// lifecycle, signing, and localnet scenarios test different wallets.
pub(crate) struct TestWalletFixture;

impl TestWalletFixture {
    /// Two independent 12-word BIP-39 halves: an anchor half and a signing
    /// half, per TEP-0003 section 3.3.
    const RECOVERY_PHRASE: &'static str = "notice tortoise soup strong gun divide offer process salon siren general carry clump left year void clutch tool case burden fix income champion lounge";

    const TESTNET_ADDRESS: &'static str = "0QCecG6hpl_o16_bYZO_x3rNzyfbhH7Ur6jTh49taojyvNCE";

    const OTHER_RECOVERY_PHRASE: &'static str = "april style market avoid find artist van spy salute broccoli daughter imitate lunch peasant crazy floor priority still aunt proof cradle fork afford blouse";

    pub(crate) const fn recovery_phrase_bytes(&self) -> &'static [u8] {
        Self::RECOVERY_PHRASE.as_bytes()
    }

    pub(crate) fn recovery_words(&self) -> Vec<String> {
        Self::RECOVERY_PHRASE
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    pub(crate) const fn testnet_address(&self) -> &'static str {
        Self::TESTNET_ADDRESS
    }

    pub(crate) fn public_key(&self) -> Vec<u8> {
        rotation_anchor_key_pair(Self::RECOVERY_PHRASE)
            .expect("test recovery phrase must derive a key pair")
            .public_key
            .to_vec()
    }

    pub(crate) const fn other_recovery_phrase_bytes(&self) -> &'static [u8] {
        Self::OTHER_RECOVERY_PHRASE.as_bytes()
    }
}

pub(crate) const fn test_wallet() -> TestWalletFixture {
    TestWalletFixture
}

/// Derives the anchor key pair of a rotation mnemonic.
///
/// An independent copy of the engine's derivation so integration tests do not
/// depend on crate-private modules: BIP-39 seed of words 1-12 with no
/// passphrase, then SLIP-0010 ed25519 along the hardened `m/44'/607'/0'`.
pub(crate) fn rotation_anchor_key_pair(phrase: &str) -> Result<KeyPair, String> {
    let anchor_half = phrase
        .split_whitespace()
        .take(12)
        .collect::<Vec<_>>()
        .join(" ");

    let mut seed = [0_u8; 64];
    pbkdf2_hmac::<Sha512>(anchor_half.as_bytes(), b"mnemonic", 2048, &mut seed);

    let mut node = hmac_sha512(b"ed25519 seed", &[&seed])?;
    for index in [44_u32, 607, 0] {
        let hardened = (index | 0x8000_0000).to_be_bytes();
        let (key, chain) = node
            .split_at_checked(32)
            .ok_or_else(|| "HMAC-SHA512 output must hold a key and chain code".to_owned())?;
        node = hmac_sha512(chain, &[&[0_u8], key, &hardened])?;
    }

    let private_key = node
        .get(..32)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| "derived node must start with a 32-byte key".to_owned())?;
    let signing_key = SigningKey::from_bytes(&private_key);

    Ok(KeyPair {
        public_key: signing_key.verifying_key().to_bytes(),
        secret_key: signing_key.to_keypair_bytes(),
    })
}

fn hmac_sha512(key: &[u8], parts: &[&[u8]]) -> Result<[u8; 64], String> {
    let mut mac = Hmac::<Sha512>::new_from_slice(key).map_err(|error| error.to_string())?;
    for part in parts {
        mac.update(part);
    }

    <[u8; 64]>::try_from(mac.finalize().into_bytes().as_slice()).map_err(|error| error.to_string())
}
