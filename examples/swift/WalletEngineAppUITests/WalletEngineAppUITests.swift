import XCTest

enum IOSScenarioTestError: LocalizedError {
    case missingEnvironment(String)
    case invalidURL(String)

    var errorDescription: String? {
        switch self {
        case .missingEnvironment(let name):
            "The iOS E2E harness did not provide \(name)"
        case .invalidURL(let value):
            "The iOS E2E harness provided an invalid URL: \(value)"
        }
    }
}

@MainActor
final class WalletEngineAppUITests: XCTestCase {
    /// Configures deterministic failure handling for every client scenario.
    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    /// Runs the shared wallet creation and restoration scenario on iOS.
    func testWalletLifecycleScenario() async throws {
        try await runScenario(named: "wallet-lifecycle")
    }

    /// Runs the shared TON Connect connection and rejection scenario on iOS.
    func testTonConnectScenario() async throws {
        try await runScenario(named: "ton-connect")
    }

    /// Runs the shared ten-message TON Connect review scenario on iOS.
    func testTenMessageTonConnectScenario() async throws {
        try await runScenario(named: "ton-connect-ten-messages")
    }

    /// Runs the shared TON Connect connection-rejection scenario on iOS.
    func testRejectedTonConnectScenario() async throws {
        try await runScenario(named: "ton-connect-rejected")
    }

    /// Runs the shared real-localnet activity refresh and restoration scenario on iOS.
    func testLocalnetActivityScenario() async throws {
        try await runScenario(named: "localnet-activity")
    }

    /// Loads one generated definition and executes it with the native iOS runner.
    private func runScenario(named name: String) async throws {
        let environment = ProcessInfo.processInfo.environment
        let bundle = Bundle(for: WalletEngineAppUITests.self)
        let bridgeURL = try requiredRuntimeValue(
            "TON_CONNECT_BRIDGE_URL",
            environment: environment,
            bundle: bundle
        )
        let providerURL = try requiredRuntimeValue(
            "TONCENTER_BASE_URL",
            environment: environment,
            bundle: bundle
        )
        let dappOriginValue = try requiredRuntimeValue(
            "IOS_DAPP_ORIGIN",
            environment: environment,
            bundle: bundle
        )
        guard let dappOrigin = URL(string: dappOriginValue) else {
            throw IOSScenarioTestError.invalidURL(dappOriginValue)
        }
        let definition = try ScenarioLoader.load(name: name, bundle: bundle)
        let runner = try IOSScenarioRunner(
            testCase: self,
            bundle: bundle,
            bridgeURL: bridgeURL,
            providerURL: providerURL,
            dappOrigin: dappOrigin
        )
        try await runner.run(definition)
    }

    /// Returns one required harness value from the test process or generated test bundle.
    private func requiredRuntimeValue(
        _ name: String,
        environment: [String: String],
        bundle: Bundle
    ) throws -> String {
        let value = environment[name] ?? bundle.object(forInfoDictionaryKey: name) as? String
        guard let value, !value.isEmpty, !value.hasPrefix("$(") else {
            throw IOSScenarioTestError.missingEnvironment(name)
        }
        return value
    }
}
