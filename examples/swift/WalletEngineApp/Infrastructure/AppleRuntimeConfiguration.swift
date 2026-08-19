import Foundation

/// Supplies endpoint and storage overrides used by an isolated client test run.
nonisolated struct AppleRuntimeConfiguration: Sendable {
    static let current = AppleRuntimeConfiguration()

    let allowsInsecureLoopback: Bool
    let tonConnectBridgeURL: String
    let toncenterBaseURL: String?
    let storageNamespace: String?

    /// Reads supported launch variables and ignores unsafe or malformed overrides.
    init(environment: [String: String] = ProcessInfo.processInfo.environment) {
        let isClientTest = environment["WALLET_ENGINE_CLIENT_E2E"] == "1"
        allowsInsecureLoopback = isClientTest
        tonConnectBridgeURL = Self.loopbackOverride(
            environment["TON_CONNECT_BRIDGE_URL"],
            enabled: isClientTest
        ) ?? "https://connect.ton.org/bridge"
        toncenterBaseURL = Self.loopbackOverride(
            environment["TONCENTER_BASE_URL"],
            enabled: isClientTest
        )
        storageNamespace = Self.storageNamespace(
            environment["WALLET_ENGINE_CLIENT_E2E_STORAGE"],
            enabled: isClientTest
        )
    }

    /// Returns a loopback HTTP or HTTPS endpoint only for an enabled client test.
    private static func loopbackOverride(
        _ value: String?,
        enabled: Bool
    ) -> String? {
        guard enabled,
              let value,
              let url = URL(string: value),
              let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              isLoopback(url.host),
              url.user == nil,
              url.password == nil,
              url.fragment == nil else {
            return nil
        }
        return value
    }

    /// Returns a filesystem-safe namespace for one isolated simulator scenario.
    private static func storageNamespace(
        _ value: String?,
        enabled: Bool
    ) -> String? {
        guard enabled, let value, !value.isEmpty, value.count <= 64 else {
            return nil
        }
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-_"))
        guard value.unicodeScalars.allSatisfy(allowed.contains) else {
            return nil
        }
        return value
    }

    /// Reports whether a URL host resolves only to the local test machine.
    static func isLoopback(_ host: String?) -> Bool {
        guard let host = host?.lowercased() else { return false }
        return host == "127.0.0.1" || host == "localhost" || host == "::1"
    }
}
