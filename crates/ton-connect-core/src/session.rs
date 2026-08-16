use std::fmt;

use crypto_box::{Nonce, PublicKey, SalsaBox, SecretKey, aead::Aead};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

use crate::ClientId;

const KEY_LENGTH: usize = 32;
const NONCE_LENGTH: usize = 24;
const AUTHENTICATION_TAG_LENGTH: usize = 16;
const MIN_ENCRYPTED_MESSAGE_LENGTH: usize = NONCE_LENGTH + AUTHENTICATION_TAG_LENGTH;

/// A failure to restore persisted TON Connect session key material.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SessionKeyPairError {
    /// The serialized secret key was not exactly 32 lowercase-hex bytes.
    #[error("session secret key must be exactly 64 lowercase hexadecimal characters")]
    InvalidSecretKey,
    /// The persisted public key does not correspond to the persisted secret key.
    #[error("persisted session public and secret keys do not match")]
    PublicKeyMismatch,
}

/// A session encryption or decryption failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SessionCryptoError {
    /// The platform cryptographically secure random source was unavailable.
    #[error("platform secure random source is unavailable")]
    EntropyUnavailable,
    /// Encryption failed.
    #[error("TON Connect session encryption failed")]
    EncryptionFailed,
    /// The message cannot contain a nonce and an authentication tag.
    #[error("TON Connect encrypted message is truncated")]
    TruncatedMessage,
    /// Authentication failed because the key, nonce, or ciphertext is invalid.
    #[error("TON Connect encrypted message authentication failed")]
    AuthenticationFailed,
}

struct SecretKeyBytes([u8; KEY_LENGTH]);

impl SecretKeyBytes {
    fn from_hex(value: &str) -> Result<Self, SessionKeyPairError> {
        if value.len() != KEY_LENGTH.saturating_mul(2)
            || !value
                .as_bytes()
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(SessionKeyPairError::InvalidSecretKey);
        }

        let mut bytes = [0_u8; KEY_LENGTH];
        hex::decode_to_slice(value, &mut bytes)
            .map_err(|_| SessionKeyPairError::InvalidSecretKey)?;
        Ok(Self(bytes))
    }
}

impl Drop for SecretKeyBytes {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Serialize for SecretKeyBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let encoded = Zeroizing::new(hex::encode(self.0));
        serializer.serialize_str(encoded.as_str())
    }
}

impl<'de> Deserialize<'de> for SecretKeyBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = Zeroizing::new(String::deserialize(deserializer)?);
        Self::from_hex(encoded.as_str()).map_err(de::Error::custom)
    }
}

/// Serializable key material required to resume one HTTP-bridge session.
///
/// This type intentionally does not implement `Debug` or `Clone`: its
/// `secretKey` grants the ability to decrypt and impersonate the session.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedSessionKeyPair {
    public_key: ClientId,
    secret_key: SecretKeyBytes,
}

impl PersistedSessionKeyPair {
    /// Returns the public bridge client identifier.
    #[must_use]
    pub const fn client_id(&self) -> ClientId {
        self.public_key
    }
}

/// `NaCl` `crypto_box` session cryptography used by the TON Connect HTTP bridge.
///
/// The bridge remains untrusted: every message is authenticated and encrypted
/// end-to-end with `X25519`, `XSalsa20`, and `Poly1305`.
pub struct SessionCrypto {
    client_id: ClientId,
    secret_key: SecretKeyBytes,
}

impl SessionCrypto {
    /// Generates a fresh session keypair with the operating system CSPRNG.
    pub fn generate() -> Result<Self, SessionCryptoError> {
        let mut bytes = [0_u8; KEY_LENGTH];
        getrandom::fill(&mut bytes).map_err(|_| SessionCryptoError::EntropyUnavailable)?;
        Ok(Self::from_secret_key(bytes))
    }

    /// Restores a session and verifies that both persisted keys match.
    pub fn from_persisted(
        persisted: &PersistedSessionKeyPair,
    ) -> Result<Self, SessionKeyPairError> {
        let secret_bytes = persisted.secret_key.0;
        let restored = Self::from_secret_key(secret_bytes);
        if restored.client_id != persisted.public_key {
            return Err(SessionKeyPairError::PublicKeyMismatch);
        }
        Ok(restored)
    }

    fn from_secret_key(bytes: [u8; KEY_LENGTH]) -> Self {
        let secret_key = SecretKey::from(bytes);
        let client_id = ClientId::from_bytes(secret_key.public_key().to_bytes());
        Self {
            client_id,
            secret_key: SecretKeyBytes(bytes),
        }
    }

    /// Returns the lowercase-hex public key used as bridge `client_id`.
    #[must_use]
    pub const fn client_id(&self) -> ClientId {
        self.client_id
    }

    /// Copies the keypair into the explicit persistence representation.
    ///
    /// The returned value must be stored as confidential session material.
    #[must_use]
    pub fn persisted_keypair(&self) -> PersistedSessionKeyPair {
        PersistedSessionKeyPair {
            public_key: self.client_id,
            secret_key: SecretKeyBytes(self.secret_key.0),
        }
    }

    /// Encrypts bytes for a peer and returns `nonce || ciphertext`.
    ///
    /// A fresh 24-byte nonce is generated by the operating system CSPRNG for
    /// every call, as required by the session protocol.
    pub fn encrypt(&self, peer: ClientId, plaintext: &[u8]) -> Result<Vec<u8>, SessionCryptoError> {
        let mut nonce = [0_u8; NONCE_LENGTH];
        getrandom::fill(&mut nonce).map_err(|_| SessionCryptoError::EntropyUnavailable)?;
        self.encrypt_with_nonce(peer, plaintext, nonce)
    }

    fn encrypt_with_nonce(
        &self,
        peer: ClientId,
        plaintext: &[u8],
        nonce_bytes: [u8; NONCE_LENGTH],
    ) -> Result<Vec<u8>, SessionCryptoError> {
        let peer_key = PublicKey::from(peer.to_bytes());
        let secret_key = SecretKey::from(self.secret_key.0);
        let cipher = SalsaBox::new(&peer_key, &secret_key);
        let nonce = Nonce::from(nonce_bytes);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|_| SessionCryptoError::EncryptionFailed)?;

        let mut result = Vec::new();
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    /// Authenticates and decrypts a `nonce || ciphertext` bridge payload.
    ///
    /// Invalid messages are returned as an opaque error and never exposed as
    /// plaintext. Errors intentionally contain no key or message material.
    pub fn decrypt(&self, peer: ClientId, encrypted: &[u8]) -> Result<Vec<u8>, SessionCryptoError> {
        if encrypted.len() < MIN_ENCRYPTED_MESSAGE_LENGTH {
            return Err(SessionCryptoError::TruncatedMessage);
        }

        let nonce_slice = encrypted
            .get(..NONCE_LENGTH)
            .ok_or(SessionCryptoError::TruncatedMessage)?;
        let ciphertext = encrypted
            .get(NONCE_LENGTH..)
            .ok_or(SessionCryptoError::TruncatedMessage)?;
        let nonce_bytes: [u8; NONCE_LENGTH] = nonce_slice
            .try_into()
            .map_err(|_| SessionCryptoError::TruncatedMessage)?;

        let peer_key = PublicKey::from(peer.to_bytes());
        let secret_key = SecretKey::from(self.secret_key.0);
        let cipher = SalsaBox::new(&peer_key, &secret_key);
        cipher
            .decrypt(&Nonce::from(nonce_bytes), ciphertext)
            .map_err(|_| SessionCryptoError::AuthenticationFailed)
    }
}

impl fmt::Debug for SessionCrypto {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionCrypto")
            .field("client_id", &self.client_id)
            .field("secret_key", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const ALICE_SECRET: [u8; 32] = [
        0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2, 0x66,
        0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5, 0x1d, 0xb9,
        0x2c, 0x2a,
    ];
    const BOB_SECRET: [u8; 32] = [
        0x5d, 0xab, 0x08, 0x7e, 0x62, 0x4a, 0x8a, 0x4b, 0x79, 0xe1, 0x7f, 0x8b, 0x83, 0x80, 0x0e,
        0xe6, 0x6f, 0x3b, 0xb1, 0x29, 0x26, 0x18, 0xb6, 0xfd, 0x1c, 0x2f, 0x8b, 0x27, 0xff, 0x88,
        0xe0, 0xeb,
    ];
    const NONCE: [u8; 24] = [
        0x69, 0x69, 0x6e, 0xe9, 0x55, 0xb6, 0x2b, 0x73, 0xcd, 0x62, 0xbd, 0xa8, 0x75, 0xfc, 0x73,
        0xd6, 0x82, 0x19, 0xe0, 0x03, 0x6b, 0x7a, 0x0b, 0x37,
    ];

    #[test]
    fn peers_round_trip_binary_payloads() {
        let alice = SessionCrypto::from_secret_key(ALICE_SECRET);
        let bob = SessionCrypto::from_secret_key(BOB_SECRET);
        let encrypted = alice.encrypt(bob.client_id(), b"\0TON Connect\xff");
        assert_eq!(
            encrypted.and_then(|message| bob.decrypt(alice.client_id(), &message)),
            Ok(b"\0TON Connect\xff".to_vec())
        );
    }

    #[test]
    fn encryption_matches_tweetnacl_golden_vector() {
        // Generated independently with tweetnacl 1.0.3. This locks the exact
        // X25519 + XSalsa20-Poly1305 construction and nonce-prefix framing.
        let alice = SessionCrypto::from_secret_key(ALICE_SECRET);
        let bob = SessionCrypto::from_secret_key(BOB_SECRET);
        let expected = hex::decode(
            "69696ee955b62b73cd62bda875fc73d68219e0036b7a0b37\
             4f143feef2ba7f7dabdcb006973180de64d12a7a37868ec868e1378cba7808d0",
        );
        assert_eq!(
            alice.encrypt_with_nonce(bob.client_id(), b"TON Connect core", NONCE),
            expected.map_err(|_| SessionCryptoError::EncryptionFailed)
        );
        assert_eq!(
            alice.client_id().to_string(),
            "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
        );
        assert_eq!(
            bob.client_id().to_string(),
            "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"
        );
    }

    #[test]
    fn tampered_ciphertext_is_never_returned_as_plaintext() {
        let alice = SessionCrypto::from_secret_key(ALICE_SECRET);
        let bob = SessionCrypto::from_secret_key(BOB_SECRET);
        let encrypted = alice.encrypt_with_nonce(bob.client_id(), b"request", NONCE);
        let mut tampered = encrypted.unwrap_or_default();
        if let Some(byte) = tampered.last_mut() {
            *byte ^= 1;
        }
        assert_eq!(
            bob.decrypt(alice.client_id(), &tampered),
            Err(SessionCryptoError::AuthenticationFailed)
        );
    }

    #[test]
    fn persisted_public_key_is_verified() {
        let alice = SessionCrypto::from_secret_key(ALICE_SECRET);
        let mut serialized = serde_json::to_value(alice.persisted_keypair()).unwrap_or_default();
        if let Some(public_key) = serialized.get_mut("publicKey") {
            *public_key = serde_json::Value::String("00".repeat(32));
        }
        let restored = serde_json::from_value::<PersistedSessionKeyPair>(serialized)
            .map_err(|error| error.to_string())
            .and_then(|pair| {
                SessionCrypto::from_persisted(&pair).map_err(|error| error.to_string())
            });
        assert_eq!(
            restored.err(),
            Some(SessionKeyPairError::PublicKeyMismatch.to_string())
        );
    }

    proptest! {
        #[test]
        fn authenticated_encryption_round_trips_arbitrary_bytes(plaintext in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let alice = SessionCrypto::from_secret_key(ALICE_SECRET);
            let bob = SessionCrypto::from_secret_key(BOB_SECRET);
            let encrypted = alice.encrypt_with_nonce(bob.client_id(), &plaintext, NONCE);
            let decrypted = encrypted.and_then(|message| bob.decrypt(alice.client_id(), &message));
            prop_assert_eq!(decrypted, Ok(plaintext));
        }
    }
}
