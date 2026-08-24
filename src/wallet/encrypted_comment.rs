//! TON encrypted-comment cryptography and snake-cell encoding.

use aes::Aes256;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use curve25519_dalek::edwards::CompressedEdwardsY;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha512};
use subtle::ConstantTimeEq as _;
use ton::tep::snake_data::SnakeData;
use ton::ton_core::cell::TonCell;
use ton::ton_core::traits::tlb::TLB as _;
use zeroize::Zeroizing;

use super::crypto::derive_wallet;
use crate::{Boc, Network, TonAddressString};

/// TON encrypted-comment message-body opcode.
pub(crate) const ENCRYPTED_COMMENT_OPCODE: u32 = 0x2167_da4b;

pub(crate) const MAX_ENCRYPTED_COMMENT_BYTES: usize = 960;

const PUBLIC_KEY_BYTES: usize = 32;
const MESSAGE_KEY_BYTES: usize = 16;
const AES_BLOCK_BYTES: usize = 16;
const ROOT_PAYLOAD_BYTES: usize = 35;
const REF_PAYLOAD_BYTES: usize = 127;
const MAX_ENCRYPTED_PAYLOAD_BYTES: usize = 1024;

type HmacSha512 = Hmac<Sha512>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum EncryptedCommentError {
    #[error("the encrypted comment exceeds 960 UTF-8 bytes")]
    CommentTooLong,
    #[error("the protected mnemonic is invalid")]
    InvalidMnemonic,
    #[error("the protected mnemonic does not belong to this wallet")]
    WalletIdentityMismatch,
    #[error("the peer Ed25519 public key is invalid")]
    InvalidPeerPublicKey,
    #[error("secure random generation failed")]
    RandomGeneration,
    #[error("the encrypted comment body is malformed")]
    InvalidBody,
    #[error("the encrypted comment authentication failed")]
    AuthenticationFailed,
    #[error("the decrypted comment is not valid UTF-8")]
    InvalidUtf8,
    #[error("the encrypted comment cell could not be encoded")]
    CellEncoding,
}

/// Encrypts a UTF-8 comment and returns the complete message-body BOC.
pub(crate) fn encrypt_comment(
    mnemonic_bytes: &[u8],
    network: Network,
    expected_sender: &TonAddressString,
    recipient_public_key: &[u8],
    comment: &str,
) -> Result<Boc, EncryptedCommentError> {
    if comment.len() > MAX_ENCRYPTED_COMMENT_BYTES {
        return Err(EncryptedCommentError::CommentTooLong);
    }

    let wallet = wallet_for_comment(mnemonic_bytes, network, expected_sender)?;
    let sender_seed = Zeroizing::new(
        wallet.key_pair.secret_key[..PUBLIC_KEY_BYTES]
            .try_into()
            .map_err(|_| EncryptedCommentError::InvalidMnemonic)?,
    );
    let recipient_public_key: [u8; PUBLIC_KEY_BYTES] = recipient_public_key
        .try_into()
        .map_err(|_| EncryptedCommentError::InvalidPeerPublicKey)?;
    let prefix_length = padded_prefix_length(comment.len())?;
    let mut prefix = Zeroizing::new(vec![0_u8; prefix_length]);
    getrandom::fill(prefix.as_mut()).map_err(|_| EncryptedCommentError::RandomGeneration)?;
    *prefix
        .first_mut()
        .ok_or(EncryptedCommentError::InvalidBody)? =
        u8::try_from(prefix_length).map_err(|_| EncryptedCommentError::InvalidBody)?;

    encrypt_with_prefix(
        &sender_seed,
        wallet.key_pair.public_key,
        recipient_public_key,
        expected_sender,
        comment.as_bytes(),
        &prefix,
    )
}

/// Decrypts one complete encrypted-comment message-body BOC.
pub(crate) fn decrypt_comment(
    mnemonic_bytes: &[u8],
    network: Network,
    expected_wallet: &TonAddressString,
    sender: &TonAddressString,
    body: &Boc,
) -> Result<String, EncryptedCommentError> {
    let wallet = wallet_for_comment(mnemonic_bytes, network, expected_wallet)?;
    let own_public_key = wallet.key_pair.public_key;
    let own_seed = Zeroizing::new(
        wallet.key_pair.secret_key[..PUBLIC_KEY_BYTES]
            .try_into()
            .map_err(|_| EncryptedCommentError::InvalidMnemonic)?,
    );
    decrypt_with_key(&own_seed, own_public_key, sender, body)
}

fn decrypt_with_key(
    own_seed: &[u8; 32],
    own_public_key: [u8; 32],
    sender: &TonAddressString,
    body: &Boc,
) -> Result<String, EncryptedCommentError> {
    let payload = encrypted_payload(body)?;
    let (public_key_xor, remainder) = payload.split_at(PUBLIC_KEY_BYTES);
    let (message_key, ciphertext) = remainder.split_at(MESSAGE_KEY_BYTES);
    if ciphertext.is_empty() || ciphertext.len() % AES_BLOCK_BYTES != 0 {
        return Err(EncryptedCommentError::InvalidBody);
    }

    let mut peer_public_key = [0_u8; PUBLIC_KEY_BYTES];
    for ((output, own), xor) in peer_public_key
        .iter_mut()
        .zip(own_public_key)
        .zip(public_key_xor)
    {
        *output = own ^ xor;
    }

    let shared_secret = ed25519_shared_secret(own_seed, &peer_public_key)?;
    let x = hmac_sha512(shared_secret.as_ref(), message_key)?;
    let key: [u8; 32] = x
        .get(..32)
        .ok_or(EncryptedCommentError::InvalidBody)?
        .try_into()
        .map_err(|_| EncryptedCommentError::InvalidBody)?;
    let iv: [u8; 16] = x
        .get(32..48)
        .ok_or(EncryptedCommentError::InvalidBody)?
        .try_into()
        .map_err(|_| EncryptedCommentError::InvalidBody)?;
    let mut padded_data = Zeroizing::new(ciphertext.to_vec());
    decrypt_aes_256_cbc(&key, &iv, &mut padded_data)?;

    let expected_message_key = hmac_sha512(salt(sender).as_bytes(), &padded_data)?;
    if expected_message_key
        .get(..MESSAGE_KEY_BYTES)
        .ok_or(EncryptedCommentError::InvalidBody)?
        .ct_eq(message_key)
        .unwrap_u8()
        != 1
    {
        return Err(EncryptedCommentError::AuthenticationFailed);
    }

    let prefix_length = padded_data
        .first()
        .copied()
        .map(usize::from)
        .ok_or(EncryptedCommentError::InvalidBody)?;
    if !(16..=31).contains(&prefix_length) || prefix_length > padded_data.len() {
        return Err(EncryptedCommentError::InvalidBody);
    }
    let comment = padded_data
        .get(prefix_length..)
        .ok_or(EncryptedCommentError::InvalidBody)?;
    if comment.len() > MAX_ENCRYPTED_COMMENT_BYTES {
        return Err(EncryptedCommentError::InvalidBody);
    }

    std::str::from_utf8(comment)
        .map(str::to_owned)
        .map_err(|_| EncryptedCommentError::InvalidUtf8)
}

/// Returns whether a parsed cell declares the TON encrypted-comment opcode.
pub(crate) fn is_encrypted_comment_body(body: &TonCell) -> bool {
    let mut parser = body.parser();
    parser.read_num::<u32>(32).ok() == Some(ENCRYPTED_COMMENT_OPCODE)
}

pub(crate) fn validate_encrypted_comment_body(body: &Boc) -> Result<(), EncryptedCommentError> {
    encrypted_payload(body).map(|_| ())
}

fn wallet_for_comment(
    mnemonic_bytes: &[u8],
    network: Network,
    expected_wallet: &TonAddressString,
) -> Result<super::crypto::SensitiveWallet, EncryptedCommentError> {
    let mnemonic =
        std::str::from_utf8(mnemonic_bytes).map_err(|_| EncryptedCommentError::InvalidMnemonic)?;
    let wallet =
        derive_wallet(mnemonic, network).map_err(|_| EncryptedCommentError::InvalidMnemonic)?;
    if wallet.address != *expected_wallet.as_address() {
        return Err(EncryptedCommentError::WalletIdentityMismatch);
    }
    Ok(wallet)
}

fn encrypt_with_prefix(
    sender_seed: &[u8; 32],
    sender_public_key: [u8; 32],
    recipient_public_key: [u8; 32],
    sender: &TonAddressString,
    comment: &[u8],
    prefix: &[u8],
) -> Result<Boc, EncryptedCommentError> {
    let padded_length = prefix
        .len()
        .checked_add(comment.len())
        .ok_or(EncryptedCommentError::InvalidBody)?;
    if comment.len() > MAX_ENCRYPTED_COMMENT_BYTES
        || !(16..=31).contains(&prefix.len())
        || prefix.first().copied().map(usize::from) != Some(prefix.len())
        || !padded_length.is_multiple_of(AES_BLOCK_BYTES)
    {
        return Err(EncryptedCommentError::InvalidBody);
    }

    let mut padded_data = Zeroizing::new(Vec::with_capacity(padded_length));
    padded_data.extend_from_slice(prefix);
    padded_data.extend_from_slice(comment);

    let shared_secret = ed25519_shared_secret(sender_seed, &recipient_public_key)?;
    let message_key_hash = hmac_sha512(salt(sender).as_bytes(), &padded_data)?;
    let message_key = message_key_hash
        .get(..MESSAGE_KEY_BYTES)
        .ok_or(EncryptedCommentError::InvalidBody)?;
    let x = hmac_sha512(shared_secret.as_ref(), message_key)?;
    let key: [u8; 32] = x
        .get(..32)
        .ok_or(EncryptedCommentError::InvalidBody)?
        .try_into()
        .map_err(|_| EncryptedCommentError::InvalidBody)?;
    let iv: [u8; 16] = x
        .get(32..48)
        .ok_or(EncryptedCommentError::InvalidBody)?
        .try_into()
        .map_err(|_| EncryptedCommentError::InvalidBody)?;
    let mut ciphertext = padded_data.to_vec();
    encrypt_aes_256_cbc(&key, &iv, &mut ciphertext)?;

    let payload_length = PUBLIC_KEY_BYTES
        .checked_add(MESSAGE_KEY_BYTES)
        .and_then(|length| length.checked_add(ciphertext.len()))
        .ok_or(EncryptedCommentError::InvalidBody)?;
    let mut payload = Vec::with_capacity(payload_length);
    payload.extend(
        sender_public_key
            .iter()
            .zip(recipient_public_key)
            .map(|(sender, recipient)| sender ^ recipient),
    );
    payload.extend_from_slice(message_key);
    payload.extend_from_slice(&ciphertext);
    encrypted_payload_boc(&payload)
}

fn encrypted_payload(body: &Boc) -> Result<Vec<u8>, EncryptedCommentError> {
    let body = TonCell::from_boc(body.as_bytes().to_vec())
        .map_err(|_| EncryptedCommentError::InvalidBody)?;
    let mut parser = body.parser();
    if parser
        .read_num::<u32>(32)
        .map_err(|_| EncryptedCommentError::InvalidBody)?
        != ENCRYPTED_COMMENT_OPCODE
    {
        return Err(EncryptedCommentError::InvalidBody);
    }
    let payload = SnakeData::read(&mut parser)
        .map_err(|_| EncryptedCommentError::InvalidBody)?
        .as_slice()
        .to_vec();
    if payload.len() < PUBLIC_KEY_BYTES + MESSAGE_KEY_BYTES + AES_BLOCK_BYTES
        || payload.len() > MAX_ENCRYPTED_PAYLOAD_BYTES
    {
        return Err(EncryptedCommentError::InvalidBody);
    }
    Ok(payload)
}

fn encrypted_payload_boc(payload: &[u8]) -> Result<Boc, EncryptedCommentError> {
    if payload.len() > MAX_ENCRYPTED_PAYLOAD_BYTES || payload.len() < ROOT_PAYLOAD_BYTES {
        return Err(EncryptedCommentError::InvalidBody);
    }

    let (root_chunk, remaining) = payload
        .split_at_checked(ROOT_PAYLOAD_BYTES)
        .ok_or(EncryptedCommentError::InvalidBody)?;
    let chunks = std::iter::once(root_chunk)
        .chain(remaining.chunks(REF_PAYLOAD_BYTES))
        .collect::<Vec<_>>();
    let mut reference = None;
    for chunk in chunks.iter().skip(1).rev() {
        let mut builder = TonCell::builder();
        let bit_length = chunk
            .len()
            .checked_mul(8)
            .ok_or(EncryptedCommentError::CellEncoding)?;
        builder
            .write_bits(chunk, bit_length)
            .map_err(|_| EncryptedCommentError::CellEncoding)?;
        if let Some(next) = reference {
            builder
                .write_ref(next)
                .map_err(|_| EncryptedCommentError::CellEncoding)?;
        }
        reference = Some(
            builder
                .build()
                .map_err(|_| EncryptedCommentError::CellEncoding)?,
        );
    }

    let mut root = TonCell::builder();
    root.write_bits(ENCRYPTED_COMMENT_OPCODE.to_be_bytes(), 32)
        .map_err(|_| EncryptedCommentError::CellEncoding)?;
    let root_chunk = chunks
        .first()
        .copied()
        .ok_or(EncryptedCommentError::InvalidBody)?;
    let root_bit_length = root_chunk
        .len()
        .checked_mul(8)
        .ok_or(EncryptedCommentError::CellEncoding)?;
    root.write_bits(root_chunk, root_bit_length)
        .map_err(|_| EncryptedCommentError::CellEncoding)?;
    if let Some(next) = reference {
        root.write_ref(next)
            .map_err(|_| EncryptedCommentError::CellEncoding)?;
    }
    let root = root
        .build()
        .map_err(|_| EncryptedCommentError::CellEncoding)?;
    let bytes = root
        .to_boc()
        .map_err(|_| EncryptedCommentError::CellEncoding)?;
    Boc::try_from(bytes).map_err(|_| EncryptedCommentError::CellEncoding)
}

fn padded_prefix_length(comment_length: usize) -> Result<usize, EncryptedCommentError> {
    comment_length
        .checked_add(31)
        .map(|length| length & !15)
        .and_then(|length| length.checked_sub(comment_length))
        .filter(|length| (16..=31).contains(length))
        .ok_or(EncryptedCommentError::InvalidBody)
}

fn salt(address: &TonAddressString) -> String {
    address.as_address().to_base64(true, true, true)
}

fn hmac_sha512(key: &[u8], data: &[u8]) -> Result<Zeroizing<[u8; 64]>, EncryptedCommentError> {
    let mut hmac =
        <HmacSha512 as Mac>::new_from_slice(key).map_err(|_| EncryptedCommentError::InvalidBody)?;
    hmac.update(data);
    Ok(Zeroizing::new(hmac.finalize().into_bytes().into()))
}

fn ed25519_shared_secret(
    private_seed: &[u8; 32],
    peer_public_key: &[u8; 32],
) -> Result<Zeroizing<[u8; 32]>, EncryptedCommentError> {
    let point = CompressedEdwardsY(*peer_public_key)
        .decompress()
        .filter(|point| !point.is_small_order())
        .ok_or(EncryptedCommentError::InvalidPeerPublicKey)?
        .to_montgomery();
    let digest = Sha512::digest(private_seed);
    let scalar: [u8; 32] = digest
        .get(..32)
        .ok_or(EncryptedCommentError::InvalidPeerPublicKey)?
        .try_into()
        .map_err(|_| EncryptedCommentError::InvalidPeerPublicKey)?;
    let shared = Zeroizing::new(point.mul_clamped(scalar).to_bytes());
    if shared.ct_eq(&[0_u8; 32]).unwrap_u8() == 1 {
        return Err(EncryptedCommentError::InvalidPeerPublicKey);
    }
    Ok(shared)
}

fn encrypt_aes_256_cbc(
    key: &[u8; 32],
    iv: &[u8; 16],
    data: &mut [u8],
) -> Result<(), EncryptedCommentError> {
    if data.is_empty() || !data.len().is_multiple_of(AES_BLOCK_BYTES) {
        return Err(EncryptedCommentError::InvalidBody);
    }
    let cipher = Aes256::new(key.into());
    let mut previous = *iv;
    for block in data.chunks_exact_mut(AES_BLOCK_BYTES) {
        for (byte, previous_byte) in block.iter_mut().zip(previous) {
            *byte ^= previous_byte;
        }
        cipher.encrypt_block(block.into());
        previous.copy_from_slice(block);
    }
    Ok(())
}

fn decrypt_aes_256_cbc(
    key: &[u8; 32],
    iv: &[u8; 16],
    data: &mut [u8],
) -> Result<(), EncryptedCommentError> {
    if data.is_empty() || !data.len().is_multiple_of(AES_BLOCK_BYTES) {
        return Err(EncryptedCommentError::InvalidBody);
    }
    let cipher = Aes256::new(key.into());
    let mut previous = *iv;
    for block in data.chunks_exact_mut(AES_BLOCK_BYTES) {
        let encrypted: [u8; AES_BLOCK_BYTES] = block
            .try_into()
            .map_err(|_| EncryptedCommentError::InvalidBody)?;
        cipher.decrypt_block(block.into());
        for (byte, previous_byte) in block.iter_mut().zip(previous) {
            *byte ^= previous_byte;
        }
        previous = encrypted;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use ed25519_dalek::SigningKey;
    use ton::ton_core::cell::TonCell;

    use super::*;

    const SENDER_SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    const RECIPIENT_SEED: [u8; 32] = [
        0x4c, 0xcd, 0x08, 0x9b, 0x28, 0xff, 0x96, 0xda, 0x9d, 0xb6, 0xc3, 0x46, 0xec, 0x11, 0x4e,
        0x0f, 0x5b, 0x8a, 0x31, 0x9f, 0x35, 0xab, 0xa6, 0x24, 0xda, 0x8c, 0xf6, 0xed, 0x4f, 0xb8,
        0xa6, 0xfb,
    ];
    const ADDRESS: &str = "0:1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn deterministic_vector_uses_the_standard_opcode_and_snake_shape() {
        let sender = TonAddressString::try_from(ADDRESS).expect("valid sender");
        let sender_public = SigningKey::from_bytes(&SENDER_SEED)
            .verifying_key()
            .to_bytes();
        let recipient_public = SigningKey::from_bytes(&RECIPIENT_SEED)
            .verifying_key()
            .to_bytes();
        let mut prefix = (0_u8..23).collect::<Vec<_>>();
        prefix[0] = 23;

        let body = encrypt_with_prefix(
            &SENDER_SEED,
            sender_public,
            recipient_public,
            &sender,
            b"hello TON",
            &prefix,
        )
        .expect("vector encrypts");
        let cell = TonCell::from_boc(body.as_bytes().to_vec()).expect("body parses");
        let mut parser = cell.parser();
        assert_eq!(
            parser.read_num::<u32>(32).expect("opcode"),
            ENCRYPTED_COMMENT_OPCODE
        );
        assert_eq!(cell.refs().len(), 1, "64 encrypted bytes require one ref");
        assert_eq!(
            salt(&sender),
            "EQAREREREREREREREREREREREREREREREREREREREREREeYT"
        );
        assert_eq!(
            STANDARD.encode(encrypted_payload(&body).expect("payload parses")),
            "6hqPwmryg+1H/PR0hH95hpJ5Xjz0YrWpb89Pmd3zNxYw7gaE8q0p1XqArX6+K4EhYrBs3tZH0Bj2DwAjLTLIf/BjUDsC0Z/6g4uwB59HEWA="
        );
    }

    #[test]
    fn shared_secret_and_ciphertext_round_trip_between_ed25519_keys() {
        let sender = TonAddressString::try_from(ADDRESS).expect("valid sender");
        let sender_public = SigningKey::from_bytes(&SENDER_SEED)
            .verifying_key()
            .to_bytes();
        let recipient_public = SigningKey::from_bytes(&RECIPIENT_SEED)
            .verifying_key()
            .to_bytes();
        assert_eq!(
            ed25519_shared_secret(&SENDER_SEED, &recipient_public).expect("sender secret"),
            ed25519_shared_secret(&RECIPIENT_SEED, &sender_public).expect("recipient secret")
        );

        let mut prefix = (0_u8..22).collect::<Vec<_>>();
        prefix[0] = 22;
        let body = encrypt_with_prefix(
            &SENDER_SEED,
            sender_public,
            recipient_public,
            &sender,
            b"encrypted!",
            &prefix,
        )
        .expect("comment encrypts");
        assert_eq!(
            decrypt_with_key(&RECIPIENT_SEED, recipient_public, &sender, &body)
                .expect("recipient decrypts"),
            "encrypted!"
        );
        let payload = encrypted_payload(&body).expect("payload parses");
        assert_eq!(payload.len(), 80);

        let public_xor: [u8; 32] = payload[..32].try_into().expect("xor length");
        let recovered_sender =
            std::array::from_fn(|index| public_xor[index] ^ recipient_public[index]);
        assert_eq!(recovered_sender, sender_public);
    }

    #[test]
    fn rejects_oversized_and_tampered_payloads() {
        assert_eq!(padded_prefix_length(0).expect("prefix"), 16);
        assert_eq!(padded_prefix_length(1).expect("prefix"), 31);
        assert_eq!(padded_prefix_length(960).expect("prefix"), 16);

        let sender = TonAddressString::try_from(ADDRESS).expect("valid sender");
        let sender_public = SigningKey::from_bytes(&SENDER_SEED)
            .verifying_key()
            .to_bytes();
        let recipient_public = SigningKey::from_bytes(&RECIPIENT_SEED)
            .verifying_key()
            .to_bytes();
        let prefix = [16_u8; 16];
        assert!(matches!(
            encrypt_with_prefix(
                &SENDER_SEED,
                sender_public,
                recipient_public,
                &sender,
                &[0_u8; 961],
                &prefix,
            ),
            Err(EncryptedCommentError::InvalidBody)
        ));

        let maximum = vec![b'a'; MAX_ENCRYPTED_COMMENT_BYTES];
        let maximum_body = encrypt_with_prefix(
            &SENDER_SEED,
            sender_public,
            recipient_public,
            &sender,
            &maximum,
            &prefix,
        )
        .expect("maximum-size comment encrypts");
        assert_eq!(
            encrypted_payload(&maximum_body)
                .expect("maximum payload parses")
                .len(),
            MAX_ENCRYPTED_PAYLOAD_BYTES
        );
        assert_eq!(
            decrypt_with_key(&RECIPIENT_SEED, recipient_public, &sender, &maximum_body,)
                .expect("maximum-size comment decrypts")
                .as_bytes(),
            maximum
        );

        let mut tampered = encrypted_payload(&maximum_body).expect("payload parses");
        let last = tampered.last_mut().expect("ciphertext is nonempty");
        *last ^= 1;
        let tampered = encrypted_payload_boc(&tampered).expect("tampered BOC builds");
        assert!(matches!(
            decrypt_with_key(&RECIPIENT_SEED, recipient_public, &sender, &tampered),
            Err(EncryptedCommentError::AuthenticationFailed)
        ));
    }
}
