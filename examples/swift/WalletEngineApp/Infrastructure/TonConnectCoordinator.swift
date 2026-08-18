import Foundation
import Observation
import WalletEngineFFI

#if os(iOS)
import UIKit
#endif

struct TonConnectConnection: Equatable, Sendable {
    let manifest: TonConnectManifest
}

enum TonConnectApproval: Identifiable, Equatable, Sendable {
    case connect(manifest: TonConnectManifest, prompt: TonConnectConnectPrompt)
    case transaction(
        manifest: TonConnectManifest,
        request: TonConnectIncomingRequest,
        preview: SendPreview
    )

    var id: String {
        switch self {
        case .connect(let manifest, _):
            "connect:\(manifest.url)"
        case .transaction(_, let request, _):
            "transaction:\(request.id)"
        }
    }
}

/// Apple product integration around the Rust TON Connect session state machine.
@MainActor
@Observable
final class TonConnectCoordinator {
    private(set) var connection: TonConnectConnection?
    private(set) var approval: TonConnectApproval?
    private(set) var diagnostic: String?
    private(set) var isWorking = false

    /// Reports whether a new transaction can replace an unresolved signed send after confirmation.
    var canForceRetry: Bool {
        walletSession.snapshot.send.resolution?.canForceRetry == true
    }

    @ObservationIgnored private let wallet: StoredWallet
    @ObservationIgnored private let descriptor: WalletDescriptor
    @ObservationIgnored private let walletSession: WalletSession
    @ObservationIgnored private let lifecycle: WalletLifecycleModel
    @ObservationIgnored private let transport: TonConnectTransport
    @ObservationIgnored private let store: TonConnectSessionStore
    @ObservationIgnored private var rustSession: TonConnectSession?
    @ObservationIgnored private var manifest: TonConnectManifest?
    @ObservationIgnored private var listenerTask: Task<Void, Never>?
    @ObservationIgnored private var generation: UInt64 = 0

    init(
        wallet: StoredWallet,
        walletSession: WalletSession,
        lifecycle: WalletLifecycleModel,
        transport: TonConnectTransport = TonConnectTransport(),
        store: TonConnectSessionStore = .shared
    ) throws {
        guard let descriptor = wallet.descriptor else {
            throw TonConnectCoordinatorError.invalidWallet
        }
        self.wallet = wallet
        self.descriptor = descriptor
        self.walletSession = walletSession
        self.lifecycle = lifecycle
        self.transport = transport
        self.store = store
    }

    func restore() async {
        guard rustSession == nil else { return }
        do {
            guard let stored = try await store.load(recordId: wallet.recordId) else {
                return
            }
            let restoredManifest = TonConnectManifest(
                url: stored.manifestURL,
                name: stored.manifestName,
                iconUrl: stored.manifestIconURL,
                domain: stored.manifestDomain
            )
            let session = try tonConnectSessionRestore(
                persisted: stored.rustSession,
                config: Self.config
            )
            let phase = try session.phase()
            rustSession = session
            manifest = restoredManifest
            if phase == .connected {
                connection = TonConnectConnection(manifest: restoredManifest)
            }
            try await deliverPendingPostIfNeeded()

            switch phase {
            case .pendingConnect:
                guard let prompt = try session.connectPrompt() else {
                    throw TonConnectCoordinatorError.missingConnectPrompt
                }
                approval = .connect(manifest: restoredManifest, prompt: prompt)
            case .connected:
                connection = TonConnectConnection(manifest: restoredManifest)
                for request in try session.pendingRequests(now: Self.now) {
                    await handle(request)
                }
                startListening()
            case .disconnected:
                guard try session.pendingPost() == nil else { return }
                try await clearPersistedSession()
                clearMemory()
            }
        } catch {
            diagnostic = Self.sanitized(error)
        }
    }

    func start(link: String) async throws {
        guard rustSession == nil else {
            throw TonConnectCoordinatorError.sessionAlreadyActive
        }
        isWorking = true
        diagnostic = nil
        defer { isWorking = false }

        let session = try tonConnectSessionFromLink(link: link, config: Self.config)
        guard let prompt = try session.connectPrompt() else {
            throw TonConnectCoordinatorError.missingConnectPrompt
        }
        let manifestJSON = try await transport.loadManifest(from: prompt.manifestUrl)
        let parsedManifest = try parseTonConnectManifest(json: manifestJSON)
        rustSession = session
        manifest = parsedManifest
        try await persist()
        approval = .connect(manifest: parsedManifest, prompt: prompt)
    }

    func approveConnection() async {
        guard case .connect(_, let prompt) = approval,
              let session = rustSession,
              let manifest else { return }
        isWorking = true
        diagnostic = nil
        do {
            let account = try lifecycle.tonConnectAccount(for: descriptor)
            let proof: TonConnectProofReply?
            if let payload = prompt.proofPayload {
                let timestamp = Self.now
                let signed = try await lifecycle.signTonConnectProof(
                    descriptor: descriptor,
                    domain: manifest.domain,
                    timestamp: timestamp,
                    payload: payload
                )
                proof = TonConnectProofReply(
                    timestamp: timestamp,
                    domain: manifest.domain,
                    payload: payload,
                    signature: signed.signature
                )
            } else {
                proof = nil
            }
            let post = try session.approveConnect(
                account: account,
                proof: proof,
                device: Self.device
            )
            connection = TonConnectConnection(manifest: manifest)
            approval = nil
            try await deliver(post)
            startListening()
        } catch {
            diagnostic = Self.sanitized(error)
        }
        isWorking = false
    }

    func rejectConnection() async {
        guard let session = rustSession else { return }
        isWorking = true
        close()
        do {
            let phase = try session.phase()
            if let pending = try session.pendingPost() {
                try await deliver(pending, terminal: phase == .disconnected)
            }

            switch phase {
            case .pendingConnect:
                let post = try session.rejectConnect(message: "User declined the connection")
                try await deliver(post, terminal: true)
            case .connected:
                throw TonConnectCoordinatorError.sessionNotConnected
            case .disconnected:
                try await clearPersistedSession()
            }
            clearMemory()
        } catch {
            diagnostic = Self.sanitized(error)
        }
        isWorking = false
    }

    func approveTransaction(force: Bool = false) async {
        guard case .transaction(_, let request, _) = approval,
              let sendRequest = request.sendRequest,
              let session = rustSession else { return }
        isWorking = true
        diagnostic = nil
        let result: SendResult
        do {
            result = try await walletSession.send(
                SendRequest(
                    operationId: sendRequest.operationId,
                    force: force,
                    intent: sendRequest.intent
                )
            )
            guard result.phase == .submitted
                    || result.phase == .submissionUnknown
                    || result.phase == .confirmed else {
                throw TonConnectCoordinatorError.unsuccessfulSend(result.phase)
            }
        } catch {
            await failTransaction(request: request, error: error)
            isWorking = false
            return
        }

        approval = nil
        do {
            let post = try session.prepareSendSuccess(
                requestId: request.id,
                signedBoc: result.signedBoc
            )
            try await deliver(post)
        } catch {
            diagnostic = "Transaction submitted; TON Connect response is waiting for bridge delivery."
        }
        isWorking = false
    }

    func rejectTransaction() async {
        guard case .transaction(_, let request, _) = approval else { return }
        isWorking = true
        do {
            try await sendError(
                requestId: request.id,
                code: .userDeclined,
                message: "User declined the transaction"
            )
            approval = nil
        } catch {
            diagnostic = Self.sanitized(error)
        }
        isWorking = false
    }

    func disconnect() async {
        guard let session = rustSession else { return }
        isWorking = true
        diagnostic = nil
        close()
        do {
            let phase = try session.phase()
            if let pending = try session.pendingPost() {
                try await deliver(pending, terminal: phase == .disconnected)
            }

            switch phase {
            case .pendingConnect:
                throw TonConnectCoordinatorError.sessionNotConnected
            case .connected:
                let post = try session.disconnect()
                try await deliver(post, terminal: true)
            case .disconnected:
                try await clearPersistedSession()
            }
            clearMemory()
        } catch {
            diagnostic = Self.sanitized(error)
        }
        isWorking = false
    }

    func close() {
        generation &+= 1
        listenerTask?.cancel()
        listenerTask = nil
    }

    /// Removes only the UI diagnostic. The durable TON Connect session keeps
    /// running so dismissing a provider or bridge error does not disconnect the
    /// dApp behind the user's back.
    func dismissDiagnostic() {
        diagnostic = nil
    }

    private func startListening() {
        listenerTask?.cancel()
        generation &+= 1
        let activeGeneration = generation
        listenerTask = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled, activeGeneration == self.generation {
                do {
                    guard let session = self.rustSession else { return }
                    try await self.deliverPendingPostIfNeeded()
                    guard try session.phase() == .connected else {
                        self.clearMemory()
                        return
                    }
                    let url = try session.beginEventsSubscription()
                    try await self.transport.stream(from: url) { [weak self] chunk in
                        guard let self else { return }
                        try await self.receive(chunk, generation: activeGeneration)
                    }
                } catch is CancellationError {
                    return
                } catch {
                    guard activeGeneration == self.generation else { return }
                    if !Self.isTransientBridgeError(error) {
                        self.diagnostic = Self.sanitized(error)
                    }
                    try? await Task.sleep(for: .seconds(2))
                }
            }
        }
    }

    private func receive(_ chunk: Data, generation: UInt64) async throws {
        guard generation == self.generation, let session = rustSession else { return }
        let requests = try session.ingestSseChunk(chunk: chunk, now: Self.now)
        try await persist()
        for request in requests {
            await handle(request)
        }
    }

    private func handle(_ request: TonConnectIncomingRequest) async {
        switch request.kind {
        case .sendTransaction:
            await preview(request)
        case .disconnect:
            do {
                guard let session = rustSession else { return }
                let post = try session.prepareDisconnectSuccess(requestId: request.id)
                // The authenticated disconnect already ended the Rust session.
                // Reflect that state before posting the acknowledgement so a
                // bridge failure cannot leave the UI showing a live dApp.
                connection = nil
                approval = nil
                try await deliver(post, terminal: true)
                clearMemory()
            } catch {
                diagnostic = Self.sanitized(error)
            }
        case .unsupported:
            do {
                try await sendError(
                    requestId: request.id,
                    code: request.errorCode ?? .unknown,
                    message: request.errorMessage ?? "Method is not supported"
                )
            } catch {
                diagnostic = Self.sanitized(error)
            }
        }
    }

    private func preview(_ request: TonConnectIncomingRequest) async {
        guard let sendRequest = request.sendRequest, let manifest else { return }
        do {
            let preview = try await walletSession.previewTonConnect(sendRequest)
            approval = .transaction(
                manifest: manifest,
                request: request,
                preview: preview
            )
        } catch {
            do {
                try await sendError(
                    requestId: request.id,
                    code: .unknown,
                    message: "Transaction preview failed: \(Self.sanitized(error))"
                )
            } catch {
                diagnostic = Self.sanitized(error)
            }
        }
    }

    private func failTransaction(
        request: TonConnectIncomingRequest,
        error: Error
    ) async {
        let message = "Transaction failed: \(Self.sanitized(error))"
        diagnostic = message
        do {
            try await sendError(requestId: request.id, code: .unknown, message: message)
            approval = nil
        } catch {
            diagnostic = Self.sanitized(error)
        }
    }

    private func sendError(
        requestId: String,
        code: TonConnectRpcErrorCode,
        message: String
    ) async throws {
        guard let session = rustSession else {
            throw TonConnectCoordinatorError.missingSession
        }
        let post = try session.prepareError(
            requestId: requestId,
            code: code,
            message: message
        )
        try await deliver(post)
    }

    private func deliverPendingPostIfNeeded() async throws {
        guard let session = rustSession,
              let post = try session.pendingPost() else { return }
        try await deliver(post, terminal: (try session.phase()) == .disconnected)
        diagnostic = nil
    }

    private func deliver(
        _ post: TonConnectPreparedPost,
        terminal: Bool = false
    ) async throws {
        guard let session = rustSession else {
            throw TonConnectCoordinatorError.missingSession
        }
        try await persist()
        do {
            try await transport.post(post)
        } catch {
            if (try? session.phase()) == .connected {
                startListening()
            }
            throw error
        }
        try session.completePendingPost()
        if terminal {
            try await clearPersistedSession()
        } else {
            try await persist()
        }
    }

    private func persist() async throws {
        guard let session = rustSession, let manifest else { return }
        try await store.save(
            StoredTonConnectSession(
                rustSession: try session.persisted(),
                manifestURL: manifest.url,
                manifestName: manifest.name,
                manifestIconURL: manifest.iconUrl,
                manifestDomain: manifest.domain
            ),
            recordId: wallet.recordId
        )
    }

    private func clearPersistedSession() async throws {
        try await store.remove(recordId: wallet.recordId)
    }

    private func clearMemory() {
        close()
        rustSession = nil
        manifest = nil
        connection = nil
        approval = nil
    }

    private static var config: TonConnectSessionConfig {
        TonConnectSessionConfig(
            bridgeUrl: "https://connect.ton.org/bridge",
            maxEventBytes: 1_048_576,
            messageTtlSeconds: 300
        )
    }

    private static var device: TonConnectDevice {
        let platform: TonConnectDevicePlatform
#if os(macOS)
        platform = .mac
#else
        platform = UIDevice.current.userInterfaceIdiom == .pad ? .ipad : .iphone
#endif
        return TonConnectDevice(
            platform: platform,
            appName: "tonkeeper",
            appVersion: Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.1.0"
        )
    }

    private static var now: UInt64 {
        UInt64(max(0, Date().timeIntervalSince1970))
    }

    private static func isTransientBridgeError(_ error: Error) -> Bool {
        guard let urlError = error as? URLError else { return false }
        switch urlError.code {
        case .cannotConnectToHost,
             .cannotFindHost,
             .dnsLookupFailed,
             .networkConnectionLost,
             .notConnectedToInternet,
             .timedOut:
            return true
        default:
            return false
        }
    }

    private static func sanitized(_ error: Error) -> String {
        let description = (error as? LocalizedError)?.errorDescription
            ?? error.localizedDescription
        return String(
            description.unicodeScalars
                .map { CharacterSet.controlCharacters.contains($0) ? " " : String($0) }
                .joined()
                .prefix(256)
        )
    }
}

nonisolated enum TonConnectCoordinatorError: LocalizedError, Sendable {
    case invalidWallet
    case sessionAlreadyActive
    case missingConnectPrompt
    case missingSession
    case sessionNotConnected
    case unsuccessfulSend(SendPhase)

    var errorDescription: String? {
        switch self {
        case .invalidWallet:
            "Wallet metadata is incomplete."
        case .sessionAlreadyActive:
            "A TON Connect session is already active."
        case .missingConnectPrompt:
            "TON Connect link does not contain a connect request."
        case .missingSession:
            "TON Connect session is unavailable."
        case .sessionNotConnected:
            "TON Connect session is not connected."
        case .unsuccessfulSend(let phase):
            "Transaction finished with \(phase)."
        }
    }
}
