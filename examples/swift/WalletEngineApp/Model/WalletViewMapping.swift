import Foundation
import WalletEngineFFI

/// Adapts  engine records to the DTOs consumed by the existing views.
nonisolated extension WalletAccountSnapshot {
    init(engine value: WalletEngineFFI.AccountSnapshot) {
        balanceNanograms = value.balanceNanograms
        status = switch value.status {
        case .nonexistent: "nonexistent"
        case .uninitialized: "uninitialized"
        case .active: "active"
        case .frozen: "frozen"
        case .unknown: "unknown"
        }
        syncUtime = value.syncUtime
    }
}

nonisolated extension WalletTransaction {
    init(engine value: WalletEngineFFI.ActivityItem) {
        id = value.id
        transactionHash = value.transactionHash
        logicalTime = value.logicalTime
        timestamp = value.timestamp
        direction = switch value.direction {
        case .sent: "sent"
        case .received: "received"
        }
        amountNanograms = value.amountNanograms
        counterparty = value.counterparty
    }
}

nonisolated extension WalletEngineFFI.WalletSnapshot {
    var viewAccount: WalletAccountSnapshot? {
        account.map(WalletAccountSnapshot.init(engine:))
    }

    var viewTransactions: [WalletTransaction] {
        activity.map(WalletTransaction.init(engine:))
    }
}
