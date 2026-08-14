import WalletEngineFFI

nonisolated extension StoredWalletNetwork {
    init(engine value: Network) {
        self = switch value {
        case .mainnet: .mainnet
        case .testnet: .testnet
        }
    }

    var engineValue: Network {
        switch self {
        case .mainnet: .mainnet
        case .testnet: .testnet
        }
    }
}

nonisolated extension StoredWallet {
    /// Creates persisted metadata only after Rust returned a real protected
    /// secret reference for the wallet.
    init(descriptor: WalletDescriptor, name: String) {
        self.init(
            recordId: descriptor.recordId,
            address: descriptor.address,
            name: name,
            network: StoredWalletNetwork(engine: descriptor.network),
            secretRef: descriptor.secretRef.value
        )
    }

    var descriptor: WalletDescriptor? {
        guard !recordId.isEmpty,
              !secretRef.isEmpty else {
            return nil
        }

        return WalletDescriptor(
            recordId: recordId,
            address: address,
            network: network.engineValue,
            secretRef: ProtectedSecretRef(value: secretRef)
        )
    }
}
