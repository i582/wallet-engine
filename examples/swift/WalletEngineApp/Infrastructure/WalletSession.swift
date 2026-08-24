import Foundation
import Observation
import WalletEngineFFI

/// Composition root for the Apple host callbacks.
nonisolated struct AppleWalletEnvironment: Sendable {
    let platformHost: AppleWalletPlatformHost
    private let httpPolicy: AppleWalletHTTPPolicy
    private let runtime: AppleRuntimeConfiguration

    @MainActor
    init(
        bundle: Bundle = .main,
        runtime: AppleRuntimeConfiguration = .current
    ) {
        let credential = Self.loadToncenterCredential(bundle: bundle)
        var allowedOrigins = [
            "https://toncenter.com:443",
            "https://testnet.toncenter.com:443",
        ]
        if let toncenterBaseURL = runtime.toncenterBaseURL {
            allowedOrigins.append(toncenterBaseURL)
        }
        let policy = AppleWalletHTTPPolicy(
            allowedOrigins: allowedOrigins,
            allowInsecureLoopback: runtime.allowsInsecureLoopback,
            toncenterAPIKey: credential
        )

        httpPolicy = policy
        self.runtime = runtime
        platformHost = AppleWalletPlatformHost()
    }

    @MainActor
    func makeClient(
        wallet: StoredWallet,
        network: Network? = nil
    ) throws -> WalletClient {
        guard wallet.publicKey.count == 32 else {
            throw WalletSessionError.missingPublicKey
        }

        // Rust call identifiers are unique only within one WalletClient.
        // Keep the host cancellation registry client-scoped so a retiring
        // client's late cancel cannot cancel an identically numbered call in
        // its replacement.
        let httpHost = AppleWalletHTTPHost(policy: httpPolicy)
        return try WalletClient(
            config: config(
                wallet: wallet,
                publicKey: wallet.publicKey,
                network: network ?? wallet.network.engineValue
            ),
            httpHost: httpHost,
            platformHost: platformHost
        )
    }

    @MainActor
    func config(
        wallet: StoredWallet,
        publicKey: Data,
        network: Network
    ) -> WalletClientConfig {
        let isMainnet = network == .mainnet
        return WalletClientConfig(
            recordId: wallet.recordId,
            address: wallet.address,
            publicKey: publicKey,
            localSecretRef: ProtectedSecretRef(value: wallet.secretRef),
            network: network,
            sendValiditySeconds: 300,
            resolutionMarginSeconds: 60,
            providers: ProviderConfig(
                toncenterBaseUrl: runtime.toncenterBaseURL
                    ?? (isMainnet ? "https://toncenter.com" : "https://testnet.toncenter.com"),
                dnsRootAddress: nil,
                requestTimeoutMs: 15_000
            )
        )
    }

    @MainActor
    private static func loadToncenterCredential(bundle: Bundle) -> String? {
        guard let url = bundle.url(
            forResource: "toncenter-api-key",
            withExtension: nil
        ),
        let value = try? String(contentsOf: url, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines),
        !value.isEmpty else {
            return nil
        }
        return value
    }
}

/// Observable Apple-facing wrapper around one  Rust client.
///
/// Snapshot revisions are applied monotonically so an older command result
/// cannot overwrite a newer publication from `waitForChange`.
@MainActor
@Observable
final class WalletSession {
    private(set) var snapshot: WalletSnapshot
    private(set) var lastUpdate: WalletUpdate?
    private(set) var diagnostic: String?
    private(set) var isShutDown = false

    @ObservationIgnored private var client: WalletClient
    @ObservationIgnored private var observationTask: Task<Void, Never>?
    @ObservationIgnored private var lifecycleGeneration: UInt64 = 0
    @ObservationIgnored private var isReplacingClient = false

    init(client: WalletClient) throws {
        self.client = client
        snapshot = try client.snapshot()
        beginObservingChanges()
    }

    convenience init(
        wallet: StoredWallet,
        network: Network? = nil,
        environment: AppleWalletEnvironment
    ) throws {
        try self.init(
            client: environment.makeClient(wallet: wallet, network: network)
        )
    }

    func refresh() async {
        await performUpdate { client in
            try await client.refresh()
        }
    }

    func loadMoreActivity() async {
        await performUpdate { client in
            try await client.loadMoreActivity()
        }
    }

    func cancelRefresh() async {
        await performControl { client in
            try await client.cancelRefresh()
        }
    }

    func cancelLoadMoreActivity() async {
        await performControl { client in
            try await client.cancelLoadMoreActivity()
        }
    }

    func send(_ request: SendRequest) async throws -> SendResult {
        guard !isShutDown else { throw WalletSessionError.shutDown }
        guard !isReplacingClient else { throw WalletSessionError.superseded }
        let activeClient = client
        let generation = lifecycleGeneration
        do {
            let result = try await activeClient.send(request: request)
            guard isCurrent(activeClient, generation: generation) else {
                throw WalletSessionError.superseded
            }
            apply(try activeClient.snapshot())
            diagnostic = nil
            return result
        } catch {
            if isCurrent(activeClient, generation: generation) {
                applyCurrentSnapshotIfAvailable(from: activeClient)
                diagnostic = Self.sanitized(error)
            }
            throw error
        }
    }

    func previewTonConnect(_ request: SendRequest) async throws -> SendPreview {
        guard !isShutDown else { throw WalletSessionError.shutDown }
        guard !isReplacingClient else { throw WalletSessionError.superseded }
        let activeClient = client
        let generation = lifecycleGeneration
        do {
            let preview = try await activeClient.previewTonConnect(request: request)
            guard isCurrent(activeClient, generation: generation) else {
                throw WalletSessionError.superseded
            }
            diagnostic = nil
            return preview
        } catch {
            if isCurrent(activeClient, generation: generation) {
                diagnostic = Self.sanitized(error)
            }
            throw error
        }
    }

    /// Signs a Wallet V5 internal message and refreshes durable send state.
    func signMessage(_ request: SignMessageRequest) async throws -> SignMessageResult {
        guard !isShutDown else { throw WalletSessionError.shutDown }
        guard !isReplacingClient else { throw WalletSessionError.superseded }
        let activeClient = client
        let generation = lifecycleGeneration
        do {
            let result = try await activeClient.signMessage(request: request)
            guard isCurrent(activeClient, generation: generation) else {
                throw WalletSessionError.superseded
            }
            apply(try activeClient.snapshot())
            diagnostic = nil
            return result
        } catch {
            if isCurrent(activeClient, generation: generation) {
                applyCurrentSnapshotIfAvailable(from: activeClient)
                diagnostic = Self.sanitized(error)
            }
            throw error
        }
    }

    /// Validates a relayed-message request without claiming wallet-paid fees.
    func previewSignMessage(_ request: SignMessageRequest) async throws -> SignMessagePreview {
        guard !isShutDown else { throw WalletSessionError.shutDown }
        guard !isReplacingClient else { throw WalletSessionError.superseded }
        let activeClient = client
        let generation = lifecycleGeneration
        do {
            let preview = try await activeClient.previewSignMessage(
                request: SendPreviewRequest(intent: request.intent)
            )
            guard isCurrent(activeClient, generation: generation) else {
                throw WalletSessionError.superseded
            }
            diagnostic = nil
            return preview
        } catch {
            if isCurrent(activeClient, generation: generation) {
                diagnostic = Self.sanitized(error)
            }
            throw error
        }
    }

    func cancelTonConnectPreview() async {
        await performControl { client in
            try await client.cancelSendPreview()
        }
    }

    func cancelSend() async {
        await performControl { client in
            try await client.cancelSend()
        }
    }

    /// Clears only the presentation diagnostic and preserves the wallet state.
    func dismissDiagnostic() {
        diagnostic = nil
    }

    func replaceClient(_ replacement: WalletClient) async throws {
        guard !isReplacingClient else {
            throw WalletSessionError.superseded
        }
        guard replacement !== client else { return }
        let replacementSnapshot = try replacement.snapshot()
        let previous = client
        lifecycleGeneration &+= 1
        let generation = lifecycleGeneration
        isShutDown = false
        isReplacingClient = true

        observationTask?.cancel()
        observationTask = nil
        try? await previous.shutdown()

        guard generation == lifecycleGeneration else {
            try? await replacement.shutdown()
            isReplacingClient = false
            throw WalletSessionError.superseded
        }

        client = replacement
        snapshot = replacementSnapshot
        lastUpdate = nil
        diagnostic = nil
        isReplacingClient = false
        beginObservingChanges()
    }

    func shutdown() async {
        guard !isShutDown else { return }
        lifecycleGeneration &+= 1
        let generation = lifecycleGeneration
        let shuttingDownClient = client
        isShutDown = true
        observationTask?.cancel()
        observationTask = nil
        do {
            try await shuttingDownClient.shutdown()
        } catch {
            guard generation == lifecycleGeneration,
                  client === shuttingDownClient,
                  isShutDown
            else { return }
            diagnostic = Self.sanitized(error)
        }
    }

    deinit {
        observationTask?.cancel()
    }

    private func beginObservingChanges() {
        observationTask?.cancel()
        let observedClient = client
        let generation = lifecycleGeneration
        var revision = snapshot.revision

        observationTask = Task { [weak self] in
            while !Task.isCancelled {
                do {
                    let next = try await observedClient.waitForChange(
                        afterRevision: revision
                    )
                    guard next.revision > revision else { continue }
                    revision = next.revision
                    guard let self,
                          self.isCurrent(observedClient, generation: generation)
                    else { return }
                    self.apply(next)
                } catch {
                    guard !Task.isCancelled else { return }
                    guard let self,
                          self.isCurrent(observedClient, generation: generation)
                    else { return }
                    self.diagnostic = Self.sanitized(error)
                    return
                }
            }
        }
    }

    private func performUpdate(
        _ operation: (WalletClient) async throws -> WalletUpdate
    ) async {
        guard !isShutDown, !isReplacingClient else { return }
        let activeClient = client
        let generation = lifecycleGeneration
        do {
            let update = try await operation(activeClient)
            guard isCurrent(activeClient, generation: generation) else { return }
            lastUpdate = update
            apply(update.snapshot)
            diagnostic = nil
        } catch {
            guard isCurrent(activeClient, generation: generation) else { return }
            applyCurrentSnapshotIfAvailable(from: activeClient)
            diagnostic = Self.sanitized(error)
        }
    }

    private func performControl(
        _ operation: (WalletClient) async throws -> Void
    ) async {
        guard !isShutDown, !isReplacingClient else { return }
        let activeClient = client
        let generation = lifecycleGeneration
        do {
            try await operation(activeClient)
            guard isCurrent(activeClient, generation: generation) else { return }
            apply(try activeClient.snapshot())
            diagnostic = nil
        } catch {
            guard isCurrent(activeClient, generation: generation) else { return }
            applyCurrentSnapshotIfAvailable(from: activeClient)
            diagnostic = Self.sanitized(error)
        }
    }

    private func applyCurrentSnapshotIfAvailable(from source: WalletClient) {
        guard client === source, let current = try? source.snapshot() else { return }
        apply(current)
    }

    private func isCurrent(
        _ source: WalletClient,
        generation: UInt64
    ) -> Bool {
        !isShutDown
            && !isReplacingClient
            && lifecycleGeneration == generation
            && client === source
    }

    private func apply(_ next: WalletSnapshot) {
        guard next.revision >= snapshot.revision, next != snapshot else { return }
        snapshot = next
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

enum WalletSessionError: Error, Sendable {
    case missingPublicKey
    case shutDown
    case superseded
}
