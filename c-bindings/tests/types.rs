#![allow(clippy::expect_used)]

use std::mem::size_of;

use wallet_engine::Network;
use wallet_engine_c::{
    WALLET_ENGINE_NETWORK_MAINNET, WALLET_ENGINE_NETWORK_TESTNET, WalletEngineAbiStatus,
    WalletEngineNetwork, network_from_abi, network_to_abi,
};

#[test]
fn network_values_and_layout_are_stable() {
    assert_eq!(WALLET_ENGINE_NETWORK_MAINNET, 0);
    assert_eq!(WALLET_ENGINE_NETWORK_TESTNET, 1);
    assert_eq!(size_of::<WalletEngineNetwork>(), size_of::<u32>());
}

#[test]
fn network_converts_to_and_from_the_core_type() {
    for core in [Network::Mainnet, Network::Testnet] {
        let abi = network_to_abi(core);
        assert_eq!(network_from_abi(abi), Ok(core));
    }
}

#[test]
fn unknown_network_values_are_rejected() {
    for value in [2, u32::MAX] {
        assert_eq!(
            network_from_abi(value),
            Err(WalletEngineAbiStatus::InvalidArgument)
        );
    }
}
