import Foundation
import Observation
import WalletEngineFFI

/// Apple-facing owner of the  wallet lifecycle API.
///
/// Recovery phrases are transient: this model keeps one only while its
/// presentation is active and provides an explicit discard operation.
@MainActor
@Observable
final class WalletLifecycleModel {
    private(set) var recoveryPhrase: RecoveryPhrase?
    private(set) var diagnostic: String?
    private(set) var isWorking = false

    @ObservationIgnored private let lifecycle: WalletEngineFFI.WalletLifecycle
    @ObservationIgnored private var operationGeneration: UInt64 = 0

    init(platformHost: AppleWalletPlatformHost) {
        lifecycle = WalletEngineFFI.WalletLifecycle(platformHost: platformHost)
    }

    convenience init(environment: AppleWalletEnvironment) {
        self.init(platformHost: environment.platformHost)
    }

    func createWallet(
        recordId: String = UUID().uuidString.lowercased(),
        network: Network = .testnet
    ) async throws -> WalletDescriptor {
        let created = try await perform {
            try await lifecycle.createWallet(
                request: CreateWalletRequest(
                    recordId: recordId,
                    network: network
                )
            )
        }
        recoveryPhrase = created.recoveryPhrase
        return created.descriptor
    }

    func importWallet(
        words: [String],
        recordId: String = UUID().uuidString.lowercased(),
        network: Network = .testnet
    ) async throws -> WalletDescriptor {
        try await perform {
            try await lifecycle.importWallet(
                request: ImportWalletRequest(
                    recordId: recordId,
                    network: network,
                    recoveryWords: words
                )
            )
        }
    }

    func revealRecoveryPhrase(
        for descriptor: WalletDescriptor
    ) async throws {
        let phrase = try await perform {
            try await lifecycle.revealRecoveryPhrase(
                descriptor: descriptor
            )
        }
        recoveryPhrase = phrase
    }

    func deleteWallet(_ descriptor: WalletDescriptor) async throws {
        try await perform {
            try await lifecycle.deleteWallet(descriptor: descriptor)
        }
        discardRecoveryPhrase()
    }

    func tonConnectAccount(
        for descriptor: WalletDescriptor
    ) throws -> TonConnectAccountInfo {
        try lifecycle.tonConnectAccount(descriptor: descriptor)
    }

    func upgradeLegacyDescriptor(
        _ descriptor: WalletDescriptor
    ) async throws -> WalletDescriptor {
        try await lifecycle.upgradeLegacyDescriptor(descriptor: descriptor)
    }

    func signTonConnectProof(
        descriptor: WalletDescriptor,
        domain: String,
        timestamp: UInt64,
        payload: String
    ) async throws -> TonConnectProofSignature {
        try await perform {
            try await lifecycle.signTonConnectProof(
                request: TonConnectProofSignRequest(
                    descriptor: descriptor,
                    domain: domain,
                    timestamp: timestamp,
                    payload: payload
                )
            )
        }
    }

    func discardRecoveryPhrase() {
        recoveryPhrase = nil
    }

    /// Invalidates publication from an in-flight lifecycle command and removes
    /// any phrase retained for presentation. Rust/Keychain work may finish in
    /// the background, but its result can no longer reach this model.
    func cancelPresentation() {
        operationGeneration &+= 1
        isWorking = false
        recoveryPhrase = nil
    }

    private func perform<T>(
        _ operation: () async throws -> T
    ) async throws -> T {
        guard !isWorking else { throw WalletLifecycleModelError.operationInProgress }
        recoveryPhrase = nil
        operationGeneration &+= 1
        let generation = operationGeneration
        isWorking = true
        defer {
            if generation == operationGeneration {
                isWorking = false
            }
        }

        do {
            let value = try await operation()
            guard generation == operationGeneration else {
                throw WalletLifecycleModelError.superseded
            }
            try Task.checkCancellation()
            diagnostic = nil
            return value
        } catch {
            if generation == operationGeneration {
                recoveryPhrase = nil
                diagnostic = Self.sanitized(error)
            }
            throw error
        }
    }

    private static func sanitized(_ error: Error) -> String {
        String(
            String(describing: error).unicodeScalars
                .map {
                    CharacterSet.controlCharacters.contains($0) ? " " : String($0)
                }
                .joined()
                .prefix(256)
        )
    }
}

enum WalletLifecycleModelError: Error, Sendable {
    case operationInProgress
    case superseded
}
