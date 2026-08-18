import Foundation
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
            publicKey: descriptor.publicKey,
            name: name,
            network: StoredWalletNetwork(engine: descriptor.network),
            secretRef: descriptor.secretRef.value
        )
    }

    var descriptor: WalletDescriptor? {
        guard !recordId.isEmpty,
              !secretRef.isEmpty,
              let publicKey,
              publicKey.count == 32 else {
            return nil
        }

        return WalletDescriptor(
            recordId: recordId,
            address: address,
            publicKey: publicKey,
            network: network.engineValue,
            secretRef: ProtectedSecretRef(value: secretRef)
        )
    }

    var descriptorForUpgrade: WalletDescriptor? {
        guard !recordId.isEmpty, !secretRef.isEmpty else { return nil }
        return WalletDescriptor(
            recordId: recordId,
            address: address,
            publicKey: publicKey ?? Data(),
            network: network.engineValue,
            secretRef: ProtectedSecretRef(value: secretRef)
        )
    }
}
