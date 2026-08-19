import Foundation

struct ScenarioDefinition: Decodable, Sendable {
    let name: String
    let steps: [ScenarioStep]
}

struct ScenarioStep: Decodable, Sendable {
    let action: ScenarioAction
    let phase: ScenarioPhase
}

enum ScenarioPhase: String, Decodable, Sendable {
    case given
    case then
    case when
}

struct ScenarioAction: Decodable, Sendable {
    enum Kind: String, Decodable, Sendable {
        case networkLocalnet = "network.localnet"
        case walletOpen = "wallet.open"
        case walletCreate = "wallet.create"
        case walletAcceptRecovery = "wallet.acceptRecovery"
        case walletReloadDashboard = "wallet.reloadDashboard"
        case walletRefresh = "wallet.refresh"
        case walletOpenTonConnect = "wallet.openTonConnect"
        case walletCloseDialog = "wallet.closeDialog"
        case walletHandleConnectLink = "wallet.handleConnectLink"
        case walletApproveConnect = "wallet.approveConnect"
        case walletApproveRequest = "wallet.approveRequest"
        case walletRejectConnect = "wallet.rejectConnect"
        case walletRejectRequest = "wallet.rejectRequest"
        case dappStart = "dapp.start"
        case dappCreateConnectLink = "dapp.createConnectLink"
        case dappRequestTransaction = "dapp.requestTransaction"
        case expectWelcome = "expect.ui.welcome"
        case expectRecovery = "expect.ui.recovery"
        case expectDashboard = "expect.ui.dashboard"
        case expectActivity = "expect.ui.activity"
        case expectConnectApproval = "expect.ui.connectApproval"
        case expectConnectedDapp = "expect.ui.connectedDapp"
        case expectTonConnectEntry = "expect.ui.tonConnectEntry"
        case expectTransaction = "expect.ui.transaction"
        case expectDappConnected = "expect.dapp.connected"
        case expectDappConnectionRejected = "expect.dapp.connectionRejected"
        case expectTransactionApproved = "expect.dapp.transactionApproved"
        case expectTransactionRejected = "expect.dapp.transactionRejected"
        case expectScreenshot = "expect.screenshot"
    }

    let kind: Kind
    let activityExpectation: ActivityExpectation?
    let dappConfig: DappActorConfig?
    let transactionConfig: TransactionConfig?
    let dappName: String?
    let messages: [TransactionMessageConfig]?
    let network: String?
    let name: String?
    let target: ScreenshotTarget?

    private enum CodingKeys: String, CodingKey {
        case config
        case dappName
        case expectation
        case kind
        case messages
        case name
        case network
        case target
    }

    /// Decodes the action-specific configuration selected by its stable kind value.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        kind = try container.decode(Kind.self, forKey: .kind)
        dappName = try container.decodeIfPresent(String.self, forKey: .dappName)
        activityExpectation = try container.decodeIfPresent(
            ActivityExpectation.self,
            forKey: .expectation
        )
        messages = try container.decodeIfPresent([TransactionMessageConfig].self, forKey: .messages)
        network = try container.decodeIfPresent(String.self, forKey: .network)
        name = try container.decodeIfPresent(String.self, forKey: .name)
        target = try container.decodeIfPresent(ScreenshotTarget.self, forKey: .target)
        switch kind {
        case .dappStart:
            dappConfig = try container.decode(DappActorConfig.self, forKey: .config)
            transactionConfig = nil
        case .dappRequestTransaction:
            dappConfig = nil
            transactionConfig = try container.decode(TransactionConfig.self, forKey: .config)
        default:
            dappConfig = nil
            transactionConfig = nil
        }
    }
}

struct ActivityExpectation: Decodable, Sendable {
    let amounts: [String]?
    let count: Int
    let directions: [String]?
    let extends: String?
    let rememberAs: String?
    let sameAs: String?
}

enum ScreenshotTarget: String, Decodable, Sendable {
    case dialog
    case page
    case recovery
}

struct DappActorConfig: Codable, Equatable, Sendable {
    let inNetwork: String?
    let manifest: DappManifestConfig
    let manifestUrl: String
    let universalLink: String

    /// Resolves actor-origin placeholders and includes the bridge observed by the SDK.
    func rendered(origin: URL, bridgeURL: String) -> RenderedDappActorConfig {
        RenderedDappActorConfig(
            bridgeUrl: bridgeURL,
            inNetwork: inNetwork,
            manifest: manifest.rendered(origin: origin),
            manifestUrl: manifestUrl.replacingOccurrences(
                of: "{actor_origin}",
                with: origin.absoluteString
            ),
            universalLink: universalLink
        )
    }
}

struct DappManifestConfig: Codable, Equatable, Sendable {
    let iconUrl: String
    let name: String
    let url: String

    /// Resolves actor-origin placeholders in every manifest URL.
    func rendered(origin: URL) -> DappManifestConfig {
        DappManifestConfig(
            iconUrl: iconUrl.replacingOccurrences(
                of: "{actor_origin}",
                with: origin.absoluteString
            ),
            name: name,
            url: url.replacingOccurrences(
                of: "{actor_origin}",
                with: origin.absoluteString
            )
        )
    }
}

struct RenderedDappActorConfig: Codable, Equatable, Sendable {
    let bridgeUrl: String
    let inNetwork: String?
    let manifest: DappManifestConfig
    let manifestUrl: String
    let universalLink: String
}

struct TransactionConfig: Codable, Equatable, Sendable {
    let fromConnectedWallet: Bool?
    let messages: [TransactionMessageConfig]
    let network: String
    let validUntil: UInt64
}

struct TransactionMessageConfig: Codable, Equatable, Sendable {
    let address: String
    let amount: String
    let payload: String?
    let stateInit: String?
}

struct RenderedTransactionRequest: Codable, Equatable, Sendable {
    let from: String?
    let messages: [TransactionMessageConfig]
    let network: String
    let validUntil: UInt64
}

struct DappActorState: Decodable, Sendable {
    let account: DappAccount?
    let config: RenderedDappActorConfig
    let device: DappDevice?
    let error: DappError?
    let journal: [DappJournalEntry]
    let status: String
    let transaction: DappTransactionState
}

struct DappAccount: Decodable, Equatable, Sendable {
    let address: String
    let chain: String
    let publicKey: String?
    let walletStateInit: String
}

struct DappDevice: Decodable, Equatable, Sendable {
    let appName: String
    let appVersion: String
    let features: [DappFeature]
    let maxProtocolVersion: Int
    let platform: String
}

struct DappFeature: Decodable, Equatable, Sendable {
    let extraCurrencySupported: Bool?
    let maxMessages: Int?
    let name: String
}

struct DappError: Decodable, Equatable, Sendable {
    let message: String
    let name: String
}

struct DappJournalEntry: Decodable, Equatable, Sendable {
    let type: String
}

struct DappTransactionState: Decodable, Sendable {
    let error: DappError?
    let request: RenderedTransactionRequest?
    let resultIsNull: Bool
    let status: String

    private enum CodingKeys: String, CodingKey {
        case error
        case request
        case result
        case status
    }

    /// Records whether the SDK completed without returning a transaction result.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        error = try container.decodeIfPresent(DappError.self, forKey: .error)
        request = try container.decodeIfPresent(RenderedTransactionRequest.self, forKey: .request)
        if container.contains(.result) {
            resultIsNull = try container.decodeNil(forKey: .result)
        } else {
            resultIsNull = false
        }
        status = try container.decode(String.self, forKey: .status)
    }
}

enum ScenarioModelError: LocalizedError {
    case missingResource(String)

    var errorDescription: String? {
        switch self {
        case .missingResource(let name):
            "Scenario resource \(name).json is missing from the UI-test bundle"
        }
    }
}

enum ScenarioLoader {
    /// Loads one generated cross-platform scenario from the UI-test bundle.
    static func load(name: String, bundle: Bundle) throws -> ScenarioDefinition {
        guard let url = bundle.url(forResource: name, withExtension: "json") else {
            throw ScenarioModelError.missingResource(name)
        }
        return try JSONDecoder().decode(
            ScenarioDefinition.self,
            from: Data(contentsOf: url)
        )
    }
}
