//! SLIP-0010 Ed25519 key derivation.
//!
//! <https://github.com/satoshilabs/slips/blob/master/slip-0010.md>
//!
//! Each half of a rotation mnemonic becomes a BIP-39 seed
//! ([`super::mnemonic::Bip39Half::to_seed`]) and then an Ed25519 key on the
//! BIP-44 path [`TON_ACCOUNT_PATH`], as
//! [TEP-0003 section 3.3](https://github.com/ton-blockchain/TEPs/blob/master/text/0003-wallets.md#33-rotation-mnemonic)
//! requires.
//!
//! Ed25519 has no public-key derivation and no non-hardened children, so every
//! step here is hardened and every operation is infallible: HMAC-SHA512 accepts
//! any key length and every 32-byte string is a valid Ed25519 private key. That
//! is why nothing in this module returns a `Result`.
use std::fmt;

use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac as _};
use sha2::Sha512;
use zeroize::{Zeroize as _, Zeroizing};

/// Bytes in an Ed25519 private key.
pub(crate) const PRIVATE_KEY_LEN: usize = 32;

/// Bytes in a SLIP-0010 chain code.
pub(crate) const CHAIN_CODE_LEN: usize = 32;

/// HMAC key that seeds Ed25519 master generation: `"ed25519 seed"`.
const MASTER_KEY: &[u8] = b"ed25519 seed";

/// Bit that marks a child index as hardened.
const HARDENED_OFFSET: u32 = 0x8000_0000;

/// The BIP-44 path a TON account derives on: `m/44'/607'/0'`.
///
/// Indices are unhardened here; [`derive_path`] hardens every step.
pub(crate) const TON_ACCOUNT_PATH: [u32; 3] = [44, 607, 0];

/// A derived SLIP-0010 node: an Ed25519 private key and its chain code.
///
/// Both fields wipe themselves on drop and neither reaches [`fmt::Debug`].
pub(crate) struct ExtendedKey {
    /// `I_L` of the derivation step: the Ed25519 private key.
    pub(crate) private_key: Zeroizing<[u8; PRIVATE_KEY_LEN]>,
    /// `I_R` of the derivation step: the chain code the next step keys on.
    pub(crate) chain_code: Zeroizing<[u8; CHAIN_CODE_LEN]>,
}

impl fmt::Debug for ExtendedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtendedKey")
            .field("private_key", &"***REDACTED***")
            .field("chain_code", &"***REDACTED***")
            .finish()
    }
}

/// HMACs `parts` under `key` and splits the result into a node.
///
/// SLIP-0010 names the halves `I_L`, which becomes the key, and `I_R`, which
/// becomes the chain code.
#[allow(
    clippy::expect_used,
    reason = "HMAC hashes over-long keys and pads short ones, so no key length is invalid"
)]
fn hmac_node(key: &[u8], parts: &[&[u8]]) -> ExtendedKey {
    let mut mac =
        Hmac::<Sha512>::new_from_slice(key).expect("HMAC-SHA512 accepts a key of any length");
    for part in parts {
        mac.update(part);
    }

    let mut output = mac.finalize().into_bytes();
    let mut private_key = Zeroizing::new([0_u8; PRIVATE_KEY_LEN]);
    let mut chain_code = Zeroizing::new([0_u8; CHAIN_CODE_LEN]);

    let (left, right) = output.split_at(PRIVATE_KEY_LEN);
    private_key.copy_from_slice(left);
    chain_code.copy_from_slice(right);
    output.as_mut_slice().zeroize();

    ExtendedKey {
        private_key,
        chain_code,
    }
}

/// Derives the master node from a BIP-39 seed.
///
/// `I = HMAC-SHA512(key = MASTER_KEY, data = seed)`, then
/// `private_key = I[0..32]` and `chain_code = I[32..64]`.
pub(crate) fn master_key(seed: &[u8]) -> ExtendedKey {
    hmac_node(MASTER_KEY, &[seed])
}

/// Derives one hardened child of `parent`.
///
/// `index` is the unhardened number; the hardened bit is applied here. The
/// step is
/// `I = HMAC-SHA512(key = parent.chain_code, data = 0x00 || parent.private_key || ser32(index + HARDENED_OFFSET))`,
/// with `ser32` big-endian, and splits into the child key and chain code the
/// same way as [`master_key`].
pub(crate) fn derive_hardened_child(parent: &ExtendedKey, index: u32) -> ExtendedKey {
    let hardened = (index | HARDENED_OFFSET).to_be_bytes();

    hmac_node(
        parent.chain_code.as_slice(),
        &[&[0_u8], parent.private_key.as_slice(), &hardened],
    )
}

/// Walks `path` from the master node, hardening every step.
///
/// `derive_path(seed, &TON_ACCOUNT_PATH)` is the account key of one rotation
/// half.
pub(crate) fn derive_path(seed: &[u8], path: &[u32]) -> ExtendedKey {
    path.iter().fold(master_key(seed), |node, index| {
        derive_hardened_child(&node, *index)
    })
}

/// The Ed25519 signing key of a derived node.
///
/// SLIP-0010 stops at the 32-byte private key; turning it into a key pair is
/// plain ed25519, so this needs no derivation logic. `SigningKey` wipes itself
/// on drop.
pub(crate) fn signing_key(extended: &ExtendedKey) -> SigningKey {
    SigningKey::from_bytes(&extended.private_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet::mnemonic::Bip39Half;

    /// A path from the master node, then the chain code, private key, and
    /// Ed25519 public key the spec expects there.
    ///
    /// Path indices are unhardened; every step is hardened during derivation.
    /// The spec prints public keys with a leading `00` (`ser_P` for ed25519),
    /// which is dropped here.
    type Node = (&'static [u32], &'static str, &'static str, &'static str);

    /// SLIP-0010 test vector 1 for ed25519.
    const VECTOR_1_SEED: &str = "000102030405060708090a0b0c0d0e0f";
    const VECTOR_1: &[Node] = &[
        // m
        (
            &[],
            "90046a93de5380a72b5e45010748567d5ea02bbf6522f979e05c0d8d8ca9fffb",
            "2b4be7f19ee27bbf30c667b642d5f4aa69fd169872f8fc3059c08ebae2eb19e7",
            "a4b2856bfec510abab89753fac1ac0e1112364e7d250545963f135f2a33188ed",
        ),
        // m/0'
        (
            &[0],
            "8b59aa11380b624e81507a27fedda59fea6d0b779a778918a2fd3590e16e9c69",
            "68e0fe46dfb67e368c75379acec591dad19df3cde26e63b93a8e704f1dade7a3",
            "8c8a13df77a28f3445213a0f432fde644acaa215fc72dcdf300d5efaa85d350c",
        ),
        // m/0'/1'
        (
            &[0, 1],
            "a320425f77d1b5c2505a6b1b27382b37368ee640e3557c315416801243552f14",
            "b1d0bad404bf35da785a64ca1ac54b2617211d2777696fbffaf208f746ae84f2",
            "1932a5270f335bed617d5b935c80aedb1a35bd9fc1e31acafd5372c30f5c1187",
        ),
        // m/0'/1'/2'
        (
            &[0, 1, 2],
            "2e69929e00b5ab250f49c3fb1c12f252de4fed2c1db88387094a0f8c4c9ccd6c",
            "92a5b23c0b8a99e37d07df3fb9966917f5d06e02ddbd909c7e184371463e9fc9",
            "ae98736566d30ed0e9d2f4486a64bc95740d89c7db33f52121f8ea8f76ff0fc1",
        ),
        // m/0'/1'/2'/2'
        (
            &[0, 1, 2, 2],
            "8f6d87f93d750e0efccda017d662a1b31a266e4a6f5993b15f5c1f07f74dd5cc",
            "30d1dc7e5fc04c31219ab25a27ae00b50f6fd66622f6e9c913253d6511d1e662",
            "8abae2d66361c879b900d204ad2cc4984fa2aa344dd7ddc46007329ac76c429c",
        ),
        // m/0'/1'/2'/2'/1000000000'
        (
            &[0, 1, 2, 2, 1000000000],
            "68789923a0cac2cd5a29172a475fe9e0fb14cd6adb5ad98a3fa70333e7afa230",
            "8f94d394a8e8fd6b1bc2f3f49f5c47e385281d5c17e65324b0f62483e37e8793",
            "3c24da049451555d51a7014a37337aa4e12d41e485abccfa46b47dfb2af54b7a",
        ),
    ];

    /// SLIP-0010 test vector 2 for ed25519.
    const VECTOR_2_SEED: &str = "fffcf9f6f3f0edeae7e4e1dedbd8d5d2cfccc9c6c3c0bdbab7b4b1aeaba8a5a29f9c999693908d8a8784817e7b7875726f6c696663605d5a5754514e4b484542";
    const VECTOR_2: &[Node] = &[
        // m
        (
            &[],
            "ef70a74db9c3a5af931b5fe73ed8e1a53464133654fd55e7a66f8570b8e33c3b",
            "171cb88b1b3c1db25add599712e36245d75bc65a1a5c9e18d76f9f2b1eab4012",
            "8fe9693f8fa62a4305a140b9764c5ee01e455963744fe18204b4fb948249308a",
        ),
        // m/0'
        (
            &[0],
            "0b78a3226f915c082bf118f83618a618ab6dec793752624cbeb622acb562862d",
            "1559eb2bbec5790b0c65d8693e4d0875b1747f4970ae8b650486ed7470845635",
            "86fab68dcb57aa196c77c5f264f215a112c22a912c10d123b0d03c3c28ef1037",
        ),
        // m/0'/2147483647'
        (
            &[0, 2147483647],
            "138f0b2551bcafeca6ff2aa88ba8ed0ed8de070841f0c4ef0165df8181eaad7f",
            "ea4f5bfe8694d8bb74b7b59404632fd5968b774ed545e810de9c32a4fb4192f4",
            "5ba3b9ac6e90e83effcd25ac4e58a1365a9e35a3d3ae5eb07b9e4d90bcf7506d",
        ),
        // m/0'/2147483647'/1'
        (
            &[0, 2147483647, 1],
            "73bd9fff1cfbde33a1b846c27085f711c0fe2d66fd32e139d3ebc28e5a4a6b90",
            "3757c7577170179c7868353ada796c839135b3d30554bbb74a4b1e4a5a58505c",
            "2e66aa57069c86cc18249aecf5cb5a9cebbfd6fadeab056254763874a9352b45",
        ),
        // m/0'/2147483647'/1'/2147483646'
        (
            &[0, 2147483647, 1, 2147483646],
            "0902fe8a29f9140480a00ef244bd183e8a13288e4412d8389d140aac1794825a",
            "5837736c89570de861ebc173b1086da4f505d4adb387c6a1b1342d5e4ac9ec72",
            "e33c0f7d81d843c572275f287498e8d408654fdf0d1e065b84e2e6f157aab09b",
        ),
        // m/0'/2147483647'/1'/2147483646'/2'
        (
            &[0, 2147483647, 1, 2147483646, 2],
            "5d70af781f3a37b829f0d060924d5e960bdc02e85423494afc0b1a41bbe196d4",
            "551d333177df541ad876a60ea71f00447931c0a9da16f227c11ea080d7391b8d",
            "47150c75db263559a70d5778bf36abbab30fb061ad69f69ece61a72b0cfa4fc0",
        ),
    ];

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn unhex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks(2)
            .map(|pair| {
                let digits = std::str::from_utf8(pair).expect("hex fixture is ASCII");
                u8::from_str_radix(digits, 16).expect("hex fixture is valid")
            })
            .collect()
    }

    fn check(seed_hex: &str, nodes: &[Node]) {
        let seed = unhex(seed_hex);

        for (path, chain_code, private_key, public_key) in nodes {
            let derived = derive_path(&seed, path);

            assert_eq!(
                hex(derived.chain_code.as_slice()),
                *chain_code,
                "chain code at {path:?}"
            );
            assert_eq!(
                hex(derived.private_key.as_slice()),
                *private_key,
                "private key at {path:?}"
            );
            assert_eq!(
                hex(&signing_key(&derived).verifying_key().to_bytes()),
                *public_key,
                "public key at {path:?}"
            );
        }
    }

    #[test]
    fn matches_ed25519_vector_1() {
        check(VECTOR_1_SEED, VECTOR_1);
    }

    #[test]
    fn matches_ed25519_vector_2() {
        check(VECTOR_2_SEED, VECTOR_2);
    }

    /// Walking the path in one call and step by step must agree.
    #[test]
    fn derive_path_folds_hardened_children() {
        let seed = unhex(VECTOR_1_SEED);
        let path = [0, 1, 2];

        let mut stepwise = master_key(&seed);
        for index in path {
            stepwise = derive_hardened_child(&stepwise, index);
        }
        let at_once = derive_path(&seed, &path);

        assert_eq!(
            stepwise.private_key.as_slice(),
            at_once.private_key.as_slice()
        );
        assert_eq!(
            stepwise.chain_code.as_slice(),
            at_once.chain_code.as_slice()
        );
    }

    /// The empty path is the master node itself.
    #[test]
    fn empty_path_is_the_master_node() {
        let seed = unhex(VECTOR_2_SEED);
        let master = master_key(&seed);
        let derived = derive_path(&seed, &[]);

        assert_eq!(
            master.private_key.as_slice(),
            derived.private_key.as_slice()
        );
        assert_eq!(master.chain_code.as_slice(), derived.chain_code.as_slice());
    }

    /// Every step of a hardened path must change both halves of the node.
    #[test]
    fn each_step_changes_the_node() {
        let seed = unhex(VECTOR_1_SEED);
        let parent = master_key(&seed);
        let child = derive_hardened_child(&parent, 0);
        let sibling = derive_hardened_child(&parent, 1);

        assert_ne!(parent.private_key.as_slice(), child.private_key.as_slice());
        assert_ne!(parent.chain_code.as_slice(), child.chain_code.as_slice());
        assert_ne!(child.private_key.as_slice(), sibling.private_key.as_slice());
        assert_ne!(child.chain_code.as_slice(), sibling.chain_code.as_slice());
    }

    /// Trust Wallet Core derives TON with `curve: ed25519` on
    /// `m/44'/607'/0'` (`registry.json`), the same scheme as TEP-0003 section
    /// 3.1. It publishes no TON mnemonic vector, but it publishes keys for
    /// other ed25519 coins on the same derivation, which pin this code against
    /// an independent implementation of the same algorithm.
    ///
    /// NEAR, `m/44'/397'/0'`: three hardened levels, structurally identical to
    /// the TON path, and exercised here from the mnemonic all the way to the
    /// public key.
    ///
    /// <https://github.com/trustwallet/wallet-core/blob/master/tests/common/HDWallet/HDWalletTests.cpp>
    #[test]
    fn matches_trust_wallet_core_near_derivation() {
        const MNEMONIC: &str =
            "owner erupt swamp room swift final allow unaware hint identify figure cotton";
        const PRIVATE_KEY: &str =
            "35e0d9631bd538d5569266abf6be7a9a403ebfda92ddd49b3268e35360a6c2dd";
        const PUBLIC_KEY: &str = "b8d5df25047841365008f30fb6b30dd820e9a84d869f05623d114e96831f2fbf";

        let half = Bip39Half::parse(MNEMONIC).expect("valid 12-word phrase");
        let derived = derive_path(half.to_seed("").as_slice(), &[44, 397, 0]);

        assert_eq!(hex(derived.private_key.as_slice()), PRIVATE_KEY);
        assert_eq!(
            hex(&signing_key(&derived).verifying_key().to_bytes()),
            PUBLIC_KEY
        );
    }

    /// Aptos, `m/44'/637'/0'/0'/0'`: five hardened levels from the same
    /// implementation, which exercises a deeper path than any other case here.
    ///
    /// Trust Wallet Core states the mnemonic, not the seed. Its phrase is 15
    /// words, which [`Bip39Half`] deliberately rejects, so the BIP-39 seed is
    /// inlined and only the SLIP-0010 half of the chain is under test.
    ///
    /// <https://github.com/trustwallet/wallet-core/blob/master/tests/common/HDWallet/HDWalletTests.cpp>
    #[test]
    fn matches_trust_wallet_core_aptos_derivation() {
        // BIP-39 seed of "ripple scissors kick mammal hire column oak again
        // sun offer wealth tomorrow wagon turn fatal", no passphrase.
        const SEED: &str = "354c22aedb9a37407adc61f657a6f00d10ed125efa360215f36c6919abd94d6d\
                            bc193a5f9c495e21ee74118661e327e84a5f5f11fa373ec33b80897d4697557d";
        const PRIVATE_KEY: &str =
            "7f2634c0e2414a621e96e39c41d09021700cee12ee43328ed094c5580cd0bd6f";
        const PUBLIC_KEY: &str = "633e5c7e355bdd484706436ce1f06fdf280bd7c2229a7f9b6489684412c6967c";

        let derived = derive_path(&unhex(&SEED.replace(' ', "")), &[44, 637, 0, 0, 0]);

        assert_eq!(hex(derived.private_key.as_slice()), PRIVATE_KEY);
        assert_eq!(
            hex(&signing_key(&derived).verifying_key().to_bytes()),
            PUBLIC_KEY
        );
    }

    /// The TEP-0003 account path applied to the first official BIP-39 seed.
    ///
    /// Not from a published vector set: TEP-0003 ships none, so this is
    /// computed from the SLIP-0010 reference algorithm and pinned as a
    /// regression guard. The two Trust Wallet Core cases above show that the
    /// same algorithm reproduces a published implementation, so the risk here
    /// is a wrong path, not wrong arithmetic. Replace it if a cross-wallet TON
    /// vector appears.
    #[test]
    fn derives_the_ton_account_path() {
        const SEED: &str = "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc1\
                            9a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4";
        const PRIVATE_KEY: &str =
            "b477ef5ed17fb8a2b8faddd7a9835a227243a82c70b190c7af4896155aa7df9f";
        const CHAIN_CODE: &str = "a780c5d9ce9328d66eb0b9a53192489ae742c5622ea98f39a488edbdc0135387";

        assert_eq!(TON_ACCOUNT_PATH, [44, 607, 0]);

        let derived = derive_path(&unhex(&SEED.replace(' ', "")), &TON_ACCOUNT_PATH);

        assert_eq!(hex(derived.private_key.as_slice()), PRIVATE_KEY);
        assert_eq!(hex(derived.chain_code.as_slice()), CHAIN_CODE);
    }
}
