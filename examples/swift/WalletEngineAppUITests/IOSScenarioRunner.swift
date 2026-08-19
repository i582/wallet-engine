import Foundation
import XCTest

enum IOSScenarioRunnerError: LocalizedError {
    case invalidAction(String)
    case protocolMismatch(String)
    case stepFailed(scenario: String, phase: ScenarioPhase, index: Int, kind: String, cause: Error)

    var errorDescription: String? {
        switch self {
        case .invalidAction(let message):
            message
        case .protocolMismatch(let message):
            message
        case .stepFailed(let scenario, let phase, let index, let kind, let cause):
            "Scenario \"\(scenario)\", \(phase.rawValue) step \(index) (\(kind)) failed: \(cause.localizedDescription)"
        }
    }
}

@MainActor
final class IOSScenarioRunner {
    private let bridgeURL: String
    private let dapp: DappClient
    private let dappOrigin: URL
    private let driver: IOSWalletDriver
    private let provider: ProviderClient
    private let snapshots: SnapshotVerifier

    private var connectLink: String?
    private var expectedDappConfig: DappActorConfig?
    private var lastTransaction: RenderedTransactionRequest?
    private var rememberedActivity: [String: IOSActivityObservation] = [:]

    /// Creates a native runner for one generated client scenario.
    init(
        testCase: XCTestCase,
        bundle: Bundle,
        bridgeURL: String,
        providerURL: String,
        dappOrigin: URL
    ) throws {
        guard let providerOrigin = URL(string: providerURL) else {
            throw IOSScenarioRunnerError.invalidAction("the provider URL is invalid")
        }
        self.bridgeURL = bridgeURL
        self.dappOrigin = dappOrigin
        dapp = DappClient(origin: dappOrigin)
        provider = ProviderClient(origin: providerOrigin)
        driver = IOSWalletDriver(
            bridgeURL: bridgeURL,
            providerURL: providerURL,
            storageNamespace: UUID().uuidString
        )
        snapshots = SnapshotVerifier(testCase: testCase, bundle: bundle)
    }

    /// Executes every scenario step in declaration order and identifies a failure.
    func run(_ definition: ScenarioDefinition) async throws {
        try await provider.reset()
        for (offset, step) in definition.steps.enumerated() {
            do {
                try await execute(step.action)
            } catch {
                throw IOSScenarioRunnerError.stepFailed(
                    scenario: definition.name,
                    phase: step.phase,
                    index: offset + 1,
                    kind: step.action.kind.rawValue,
                    cause: error
                )
            }
        }
    }

    /// Executes one serializable action with state retained from preceding steps.
    private func execute(_ action: ScenarioAction) async throws {
        switch action.kind {
        case .networkLocalnet:
            try await provider.useLocalnet()
        case .walletOpen:
            try driver.open()
        case .walletCreate:
            try driver.createWallet()
        case .walletAcceptRecovery:
            try driver.acceptRecovery()
        case .walletReloadDashboard:
            try driver.reloadDashboard()
        case .walletRefresh:
            try driver.refresh()
        case .walletOpenTonConnect:
            try driver.openTonConnect()
        case .walletCloseDialog:
            try driver.closeDialog()
        case .walletHandleConnectLink:
            try driver.handleConnectLink(requiredConnectLink())
        case .walletApproveConnect:
            try driver.approveConnection()
        case .walletApproveRequest:
            try driver.approveRequest()
        case .walletRejectConnect:
            try driver.rejectConnection()
        case .walletRejectRequest:
            try driver.rejectRequest()
        case .dappStart:
            let config = try required(action.dappConfig, field: "dApp config")
            expectedDappConfig = config
            try await dapp.reset()
            try await assertActorConfiguration(config)
        case .dappCreateConnectLink:
            connectLink = try await dapp.createConnectLink()
        case .dappRequestTransaction:
            let config = try required(action.transactionConfig, field: "transaction config")
            lastTransaction = try await renderTransaction(config)
            try await dapp.requestTransaction(requiredLastTransaction())
        case .expectWelcome:
            try driver.assertWelcome()
        case .expectRecovery:
            try driver.assertRecovery()
        case .expectDashboard:
            try driver.assertDashboard()
        case .expectActivity:
            try assertActivity(
                try required(action.activityExpectation, field: "activity expectation")
            )
        case .expectConnectApproval:
            try driver.assertConnectApproval(
                dappName: try required(action.dappName, field: "dApp name")
            )
        case .expectConnectedDapp:
            try driver.assertConnectedDapp(
                dappName: try required(action.dappName, field: "dApp name")
            )
        case .expectTonConnectEntry:
            try driver.assertTonConnectEntry()
        case .expectTransaction:
            try driver.assertTransaction(
                messages: try required(action.messages, field: "transaction messages")
            )
        case .expectDappConnected:
            try await assertDappConnected(
                network: try required(action.network, field: "network")
            )
        case .expectDappConnectionRejected:
            try await assertConnectionRejected()
        case .expectTransactionApproved:
            try await assertTransactionApproved()
        case .expectTransactionRejected:
            try await assertTransactionRejected()
        case .expectScreenshot:
            let name = try required(action.name, field: "screenshot name")
            let target = try required(action.target, field: "screenshot target")
            try snapshots.verify(name: name, capture: driver.capture(target: target))
        }
    }

    /// Checks that the already-running actor uses the scenario's rendered configuration.
    private func assertActorConfiguration(_ config: DappActorConfig) async throws {
        let state = try await dapp.state()
        let expected = config.rendered(origin: dappOrigin, bridgeURL: bridgeURL)
        guard state.config == expected else {
            throw IOSScenarioRunnerError.protocolMismatch(
                "dApp config mismatch: expected \(expected), received \(state.config)"
            )
        }
        guard state.error == nil else {
            throw IOSScenarioRunnerError.protocolMismatch(
                "dApp started with error \(String(describing: state.error))"
            )
        }
    }

    /// Checks account, chain, device identity, capabilities, and dApp observations.
    private func assertDappConnected(network: String) async throws {
        let state = try await dapp.wait(description: "connection") { candidate in
            candidate.status == "connected"
        }
        let expectedConfig = try required(expectedDappConfig, field: "started dApp config")
            .rendered(origin: dappOrigin, bridgeURL: bridgeURL)
        guard state.config == expectedConfig else {
            throw IOSScenarioRunnerError.protocolMismatch("the dApp observed another config")
        }
        guard state.error == nil else {
            throw IOSScenarioRunnerError.protocolMismatch("the dApp observed a connection error")
        }
        guard let account = state.account,
              account.chain == network,
              !account.address.isEmpty,
              account.publicKey?.isEmpty == false,
              !account.walletStateInit.isEmpty else {
            throw IOSScenarioRunnerError.protocolMismatch("the connected account fields are incomplete")
        }
        let expectedFeatures = [
            DappFeature(extraCurrencySupported: false, maxMessages: 255, name: "SendTransaction"),
            DappFeature(extraCurrencySupported: false, maxMessages: 255, name: "SignMessage"),
        ]
        guard state.device == DappDevice(
            appName: "tonkeeper",
            appVersion: "1.0",
            features: expectedFeatures,
            maxProtocolVersion: 2,
            platform: "iphone"
        ) else {
            throw IOSScenarioRunnerError.protocolMismatch(
                "the dApp observed unexpected iOS device information"
            )
        }
        let journal = Set(state.journal.map(\.type))
        guard journal.isSuperset(of: ["connect_link_created", "wallet_connected"]) else {
            throw IOSScenarioRunnerError.protocolMismatch("the dApp connection journal is incomplete")
        }
    }

    /// Checks the terminal SDK error and empty account state after connection rejection.
    private func assertConnectionRejected() async throws {
        let state = try await dapp.wait(description: "connection rejection") { candidate in
            candidate.status == "error"
        }
        let expectedConfig = try required(expectedDappConfig, field: "started dApp config")
            .rendered(origin: dappOrigin, bridgeURL: bridgeURL)
        guard state.config == expectedConfig else {
            throw IOSScenarioRunnerError.protocolMismatch("the dApp observed another config")
        }
        guard state.account == nil, state.device == nil else {
            throw IOSScenarioRunnerError.protocolMismatch(
                "a rejected connection exposed wallet account data"
            )
        }
        guard state.error?.name == "UserRejectsError",
              state.error?.message.contains("User rejects the action in the wallet.") == true,
              state.error?.message.contains("User declined the connection") == true else {
            throw IOSScenarioRunnerError.protocolMismatch(
                "the dApp observed an unexpected connection rejection"
            )
        }
        let journal = Set(state.journal.map(\.type))
        guard journal.isSuperset(of: ["connect_link_created", "connector_error"]) else {
            throw IOSScenarioRunnerError.protocolMismatch(
                "the connection rejection journal is incomplete"
            )
        }
    }

    /// Checks the exact request and protocol error returned after wallet rejection.
    private func assertTransactionRejected() async throws {
        let transaction = try required(lastTransaction, field: "requested transaction")
        let state = try await dapp.wait(description: "transaction rejection") { candidate in
            candidate.transaction.status == "error"
        }
        guard state.transaction.request == transaction else {
            throw IOSScenarioRunnerError.protocolMismatch("the dApp observed another transaction")
        }
        guard state.transaction.resultIsNull else {
            throw IOSScenarioRunnerError.protocolMismatch("a rejected transaction returned a result")
        }
        guard state.transaction.error?.name == "UserRejectsError",
              state.transaction.error?.message.contains(
                "User rejects the action in the wallet."
              ) == true,
              state.transaction.error?.message.contains(
                "User declined the TON Connect request"
              ) == true else {
            throw IOSScenarioRunnerError.protocolMismatch(
                "the dApp observed an unexpected rejection error"
            )
        }
        let journal = Set(state.journal.map(\.type))
        guard journal.isSuperset(
            of: ["transaction_requested", "transaction_sent", "transaction_failed"]
        ) else {
            throw IOSScenarioRunnerError.protocolMismatch("the transaction journal is incomplete")
        }
    }

    /// Checks the exact request and non-empty protocol result returned after approval.
    private func assertTransactionApproved() async throws {
        let transaction = try required(lastTransaction, field: "requested transaction")
        let state = try await dapp.wait(description: "transaction approval") { candidate in
            candidate.transaction.status == "success"
        }
        guard state.transaction.request == transaction else {
            throw IOSScenarioRunnerError.protocolMismatch("the dApp observed another transaction")
        }
        guard !state.transaction.resultIsNull, state.transaction.error == nil else {
            throw IOSScenarioRunnerError.protocolMismatch(
                "an approved transaction did not return a successful result"
            )
        }
        let journal = Set(state.journal.map(\.type))
        guard journal.isSuperset(
            of: ["transaction_requested", "transaction_sent", "transaction_succeeded"]
        ) else {
            throw IOSScenarioRunnerError.protocolMismatch(
                "the transaction approval journal is incomplete"
            )
        }
    }

    /// Checks row values, uniqueness, order, and relationships to remembered history.
    private func assertActivity(_ expectation: ActivityExpectation) throws {
        let observation = try driver.observeActivity(count: expectation.count)
        guard Set(observation.ids).count == observation.ids.count,
              observation.ids.allSatisfy({ !$0.isEmpty }) else {
            throw IOSScenarioRunnerError.protocolMismatch(
                "the activity list contains empty or duplicate identifiers"
            )
        }
        if let directions = expectation.directions,
           observation.directions != directions {
            throw IOSScenarioRunnerError.protocolMismatch(
                "activity directions differ: expected \(directions), received \(observation.directions)"
            )
        }
        if let amounts = expectation.amounts {
            let directions = try required(
                expectation.directions,
                field: "directions for expected activity amounts"
            )
            guard amounts.count == directions.count else {
                throw IOSScenarioRunnerError.invalidAction(
                    "activity amounts and directions must have the same length"
                )
            }
            let rendered = zip(amounts, directions).map { amount, direction in
                driver.activityAmount(nanograms: amount, direction: direction)
            }
            guard observation.amounts == rendered else {
                throw IOSScenarioRunnerError.protocolMismatch(
                    "activity amounts differ: expected \(rendered), received \(observation.amounts)"
                )
            }
        }
        if let name = expectation.sameAs {
            let previous = try requiredActivity(name).ids
            guard observation.ids == previous else {
                throw IOSScenarioRunnerError.protocolMismatch(
                    "activity changed since observation \(name)"
                )
            }
        }
        if let name = expectation.extends {
            let previous = try requiredActivity(name).ids
            guard observation.ids.count > previous.count,
                  Array(observation.ids.suffix(previous.count)) == previous else {
                throw IOSScenarioRunnerError.protocolMismatch(
                    "activity does not extend observation \(name) in newest-first order"
                )
            }
        }
        if let name = expectation.rememberAs {
            rememberedActivity[name] = observation
        }
    }

    /// Resolves sender fields that are available only after the dApp connects.
    private func renderTransaction(
        _ config: TransactionConfig
    ) async throws -> RenderedTransactionRequest {
        let state = try await dapp.state()
        let account = state.account
        if config.fromConnectedWallet == true, account == nil {
            throw IOSScenarioRunnerError.invalidAction("the dApp has no connected wallet account")
        }
        return RenderedTransactionRequest(
            from: config.fromConnectedWallet == true ? account?.address : nil,
            messages: config.messages,
            network: config.network,
            validUntil: config.validUntil
        )
    }

    /// Returns the connect link produced by a preceding dApp step.
    private func requiredConnectLink() throws -> String {
        try required(connectLink, field: "created connect link")
    }

    /// Returns the transaction retained for the dApp-side equality assertion.
    private func requiredLastTransaction() throws -> RenderedTransactionRequest {
        try required(lastTransaction, field: "rendered transaction")
    }

    /// Returns a preceding activity observation used by an ordering or stability assertion.
    private func requiredActivity(_ name: String) throws -> IOSActivityObservation {
        try required(rememberedActivity[name], field: "activity observation \(name)")
    }

    /// Unwraps one action field or reports a malformed generated scenario.
    private func required<Value>(_ value: Value?, field: String) throws -> Value {
        guard let value else {
            throw IOSScenarioRunnerError.invalidAction("scenario action is missing \(field)")
        }
        return value
    }
}
