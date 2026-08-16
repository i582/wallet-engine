import Foundation

enum StoredWalletNetwork: String, Codable, Sendable {
    case mainnet
    case testnet
}

/// Public wallet metadata persisted by the example application.
/// Recovery words are stored only by `AppleWalletPlatformHost`.
struct StoredWallet: Codable, Identifiable, Sendable {
    let recordId: String
    let address: String
    /// Public derivation data. Older archives can omit it and require reimport.
    let publicKey: Data?
    var name: String
    let network: StoredWalletNetwork
    let secretRef: String

    var id: String { recordId }
}

struct WalletAccountSnapshot: Sendable {
    let balanceNanograms: String
    let status: String
    let syncUtime: UInt64

    var balanceGrams: String {
        GramAmount.format(nanograms: balanceNanograms)
    }
}

struct WalletTransaction: Identifiable, Sendable {
    let id: String
    let transactionHash: String
    let logicalTime: String
    let timestamp: UInt64
    let direction: String
    let amountNanograms: String
    let counterparty: String?

    var isReceived: Bool { direction == "received" }

    var amountGrams: String {
        GramAmount.format(nanograms: amountNanograms)
    }
}
