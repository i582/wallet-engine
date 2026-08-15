use ton::ton_wallet::Mnemonic;

/// A stable mnemonic and the V5R1 testnet address derived from it.
///
/// Keep these values together. Changing one without the other would make
/// lifecycle, signing, and localnet scenarios test different wallets.
pub(crate) struct TestWalletFixture;

impl TestWalletFixture {
    const RECOVERY_PHRASE: &'static str = "section garden tomato dinner season dice renew length useful spin trade intact use universe what post spike keen mandate behind concert egg doll rug";

    const TESTNET_V5_ADDRESS: &'static str = "0QA_6fh0aRAkD7n1MNfAUx8TvyCUw2iTQfzVM-0isMze2anN";

    const OTHER_RECOVERY_PHRASE: &'static str = "fish uncle sort juice lunar salute peasant decorate flash cherry become treat obtain august diet safe describe area below nasty scale right armed rural";

    pub(crate) const fn recovery_phrase_bytes(&self) -> &'static [u8] {
        Self::RECOVERY_PHRASE.as_bytes()
    }

    pub(crate) fn recovery_words(&self) -> Vec<String> {
        Self::RECOVERY_PHRASE
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    pub(crate) const fn testnet_v5_address(&self) -> &'static str {
        Self::TESTNET_V5_ADDRESS
    }

    pub(crate) fn public_key(&self) -> Vec<u8> {
        Mnemonic::from_str(Self::RECOVERY_PHRASE, None)
            .expect("test recovery phrase must remain valid")
            .to_key_pair()
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
