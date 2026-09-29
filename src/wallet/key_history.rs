//! Wallet rev00 signing-key history.
//!
//! A rotation replaces words 13-24 of the recovery phrase, so every signing key
//! between the anchor and the current key is lost from the phrase. The rotation
//! request therefore carries the replaced key encrypted with the new one, and the
//! contract publishes it in its key-changed log. Walking these logs from the
//! current key backwards recovers every earlier signing key, which is what
//! decrypting comments sent to those keys needs.

use std::collections::HashMap;

use ed25519_dalek::SigningKey;
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

/// Domain-separation salt of the encrypted old private key, version 1.
const KEY_CHANGE_SALT: &[u8] = b"keyChangeSaltV1";

/// One successful signing-key rotation of this wallet, as published by an indexer.
///
/// Both values are public: the new key is stored on-chain, and the encrypted
/// old key can be opened only with the new private key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyChange {
    /// The signing public key the rotation installed.
    pub(crate) new_public_key: [u8; 32],
    /// `sha256(new_private_key ‖ salt) XOR old_private_key`.
    pub(crate) encrypted_old_private_key: [u8; 32],
}

/// Encrypts the replaced signing key for the rotation request:
/// `sha256(new_private_key ‖ "keyChangeSaltV1") XOR old_private_key`.
///
/// Private keys are 32-byte Ed25519 seeds.
pub(crate) fn encrypt_old_private_key(old_key: &SigningKey, new_key: &SigningKey) -> [u8; 32] {
    *xor_with_mask(old_key.as_bytes(), new_key)
}

/// Opens an encrypted old private key with the key its rotation installed.
pub(crate) fn decrypt_old_private_key(
    encrypted_old_private_key: &[u8; 32],
    new_key: &SigningKey,
) -> SigningKey {
    SigningKey::from_bytes(&xor_with_mask(encrypted_old_private_key, new_key))
}

/// Recovers the signing keys that preceded `current`, newest first.
///
/// The walk opens the encrypted old key of the rotation that installed
/// `current`, then repeats from the rotation that installed the opened key,
/// until it reaches the anchor, which the recovery phrase always holds. The
/// result therefore never contains the anchor. Rotation order in `changes`
/// does not matter.
pub(crate) fn recover_signing_keys(
    anchor_public_key: &[u8; 32],
    current: &SigningKey,
    changes: &[KeyChange],
) -> Vec<SigningKey> {
    let installed_by: HashMap<[u8; 32], [u8; 32]> = changes
        .iter()
        .map(|change| (change.new_public_key, change.encrypted_old_private_key))
        .collect();

    // Every rotation installs one key, so a longer walk could only be a cycle.
    // The capacity is also final: no reallocation leaves unwiped seed copies.
    let mut recovered: Vec<SigningKey> = Vec::with_capacity(installed_by.len());
    let mut public_key = current.verifying_key().to_bytes();
    while recovered.len() < installed_by.len() {
        let Some(encrypted) = installed_by.get(&public_key) else {
            break;
        };
        let older = decrypt_old_private_key(encrypted, recovered.last().unwrap_or(current));
        public_key = older.verifying_key().to_bytes();
        if public_key == *anchor_public_key {
            break;
        }
        recovered.push(older);
    }
    recovered
}

fn xor_with_mask(value: &[u8; 32], new_key: &SigningKey) -> Zeroizing<[u8; 32]> {
    let mut hasher = Sha256::new();
    hasher.update(new_key.as_bytes());
    hasher.update(KEY_CHANGE_SALT);
    let mask = Zeroizing::new(<[u8; 32]>::from(hasher.finalize()));
    let mut output = Zeroizing::new([0_u8; 32]);
    for ((output, value), mask) in output.iter_mut().zip(value).zip(mask.iter()) {
        *output = value ^ mask;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> SigningKey {
        SigningKey::from_bytes(&[byte; 32])
    }

    fn public(key: &SigningKey) -> [u8; 32] {
        key.verifying_key().to_bytes()
    }

    fn rotation(old: &SigningKey, new: &SigningKey) -> KeyChange {
        KeyChange {
            new_public_key: public(new),
            encrypted_old_private_key: encrypt_old_private_key(old, new),
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn encryption_matches_the_documented_formula() {
        let old = key(0x11);
        let new = key(0x22);
        let mut preimage = new.as_bytes().to_vec();
        preimage.extend_from_slice(b"keyChangeSaltV1");
        let mask = Sha256::digest(&preimage);
        let expected: Vec<u8> = mask
            .iter()
            .zip(old.as_bytes())
            .map(|(mask, old)| mask ^ old)
            .collect();

        let encrypted = encrypt_old_private_key(&old, &new);

        assert_eq!(encrypted.to_vec(), expected);
        assert_eq!(
            hex(&encrypted),
            "57a12a6e8d1d18cf5dfdc42a8a63b30630522d31a9530a692f9713fd14815255",
            "the salt and field order are part of the on-chain format"
        );
        assert_eq!(
            decrypt_old_private_key(&encrypted, &new).as_bytes(),
            old.as_bytes()
        );
    }

    #[test]
    fn only_the_new_key_opens_the_old_key() {
        let old = key(0x11);
        let new = key(0x22);
        let encrypted = encrypt_old_private_key(&old, &new);

        assert_ne!(encrypted, *old.as_bytes(), "the old key is never published");
        assert_ne!(
            decrypt_old_private_key(&encrypted, &old).as_bytes(),
            old.as_bytes()
        );
        assert_ne!(
            decrypt_old_private_key(&encrypted, &key(0x33)).as_bytes(),
            old.as_bytes()
        );
    }

    #[test]
    fn walks_a_rotation_chain_back_to_the_anchor() {
        let anchor = key(1);
        let keys: Vec<SigningKey> = (2..=5).map(key).collect();
        let mut changes = vec![rotation(&anchor, &keys[0])];
        for pair in keys.windows(2) {
            changes.push(rotation(&pair[0], &pair[1]));
        }
        changes.reverse();

        let recovered = recover_signing_keys(&public(&anchor), &keys[3], &changes);

        let recovered: Vec<[u8; 32]> = recovered.iter().map(|key| *key.as_bytes()).collect();
        assert_eq!(
            recovered,
            vec![
                *keys[2].as_bytes(),
                *keys[1].as_bytes(),
                *keys[0].as_bytes()
            ]
        );
    }

    #[test]
    fn rotation_order_and_duplicates_do_not_matter() {
        let anchor = key(1);
        let first = key(2);
        let second = key(3);
        let third = key(4);
        let changes = vec![
            rotation(&first, &second),
            rotation(&anchor, &first),
            rotation(&second, &third),
            rotation(&first, &second),
        ];

        let recovered = recover_signing_keys(&public(&anchor), &third, &changes);

        assert_eq!(recovered.len(), 2);
        assert_eq!(recovered[0].as_bytes(), second.as_bytes());
        assert_eq!(recovered[1].as_bytes(), first.as_bytes());
    }

    #[test]
    fn unrotated_and_once_rotated_wallets_have_no_lost_keys() {
        let anchor = key(1);
        let first = key(2);
        let changes = vec![rotation(&anchor, &first)];

        assert!(recover_signing_keys(&public(&anchor), &anchor, &changes).is_empty());
        assert!(recover_signing_keys(&public(&anchor), &first, &changes).is_empty());
    }

    #[test]
    fn history_without_the_current_key_recovers_nothing() {
        let anchor = key(1);
        let first = key(2);
        let second = key(3);
        let third = key(4);
        let lagging = vec![rotation(&first, &second), rotation(&anchor, &first)];

        assert!(recover_signing_keys(&public(&anchor), &third, &lagging).is_empty());
    }

    #[test]
    fn a_stale_phrase_recovers_the_keys_before_its_own_key() {
        let anchor = key(1);
        let first = key(2);
        let second = key(3);
        let newer = key(4);
        let changes = vec![
            rotation(&second, &newer),
            rotation(&first, &second),
            rotation(&anchor, &first),
        ];

        let recovered = recover_signing_keys(&public(&anchor), &second, &changes);

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].as_bytes(), first.as_bytes());
    }

    #[test]
    fn a_foreign_ciphertext_ends_the_walk() {
        let anchor = key(1);
        let first = key(2);
        let second = key(3);
        let third = key(4);
        let foreign = vec![
            rotation(&second, &third),
            KeyChange {
                new_public_key: public(&second),
                encrypted_old_private_key: [0x5a; 32],
            },
            rotation(&anchor, &first),
        ];

        let recovered = recover_signing_keys(&public(&anchor), &third, &foreign);

        assert_eq!(
            recovered.len(),
            2,
            "no rotation installed the unrelated key"
        );
        assert_eq!(recovered[0].as_bytes(), second.as_bytes());
        assert_ne!(recovered[1].as_bytes(), first.as_bytes());
    }

    #[test]
    fn a_cyclic_history_terminates() {
        let anchor = key(1);
        let first = key(2);
        let second = key(3);
        let changes = vec![rotation(&second, &first), rotation(&first, &second)];

        let recovered = recover_signing_keys(&public(&anchor), &second, &changes);

        assert_eq!(recovered.len(), changes.len());
        assert_eq!(recovered[0].as_bytes(), first.as_bytes());
    }
}
