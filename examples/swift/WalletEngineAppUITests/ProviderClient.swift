import Foundation

enum ProviderClientError: LocalizedError {
    case invalidResponse
    case requestFailed(status: Int, body: String)

    var errorDescription: String? {
        switch self {
        case .invalidResponse:
            "The provider actor returned an invalid HTTP response"
        case .requestFailed(let status, let body):
            "The provider actor returned HTTP \(status): \(body)"
        }
    }
}

actor ProviderClient {
    private let origin: URL
    private let session: URLSession

    /// Creates a control client for the loopback provider started by the iOS harness.
    init(origin: URL) {
        self.origin = origin
        let configuration = URLSessionConfiguration.ephemeral
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.urlCache = nil
        session = URLSession(configuration: configuration)
    }

    /// Restores the deterministic provider before each isolated scenario starts.
    func reset() async throws {
        try await select(mode: "scripted")
    }

    /// Starts a fresh Acton localnet for all subsequent wallet provider requests.
    func useLocalnet() async throws {
        try await select(mode: "localnet")
    }

    /// Sends one provider-mode command and rejects an unsuccessful harness response.
    private func select(mode: String) async throws {
        var request = URLRequest(url: origin.appendingPathComponent("e2e/provider"))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: ["mode": mode])
        let (data, rawResponse) = try await session.data(for: request)
        guard let response = rawResponse as? HTTPURLResponse else {
            throw ProviderClientError.invalidResponse
        }
        guard (200..<300).contains(response.statusCode) else {
            throw ProviderClientError.requestFailed(
                status: response.statusCode,
                body: String(data: data, encoding: .utf8) ?? ""
            )
        }
    }
}
