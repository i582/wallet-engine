import Foundation

enum DappClientError: LocalizedError {
    case invalidResponse
    case requestFailed(status: Int, body: String)
    case timeout(String)

    var errorDescription: String? {
        switch self {
        case .invalidResponse:
            "The dApp actor returned an invalid HTTP response"
        case .requestFailed(let status, let body):
            "The dApp actor returned HTTP \(status): \(body)"
        case .timeout(let description):
            "Timed out waiting for dApp \(description)"
        }
    }
}

nonisolated private final class DappLoopbackTrustDelegate: NSObject, URLSessionDelegate, @unchecked Sendable {
    /// Trusts the harness certificate only for loopback hosts used by this test bundle.
    func urlSession(
        _ session: URLSession,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        let host = challenge.protectionSpace.host.lowercased()
        let isLoopback = host == "127.0.0.1" || host == "localhost" || host == "::1"
        guard isLoopback,
              challenge.protectionSpace.authenticationMethod
                == NSURLAuthenticationMethodServerTrust,
              let serverTrust = challenge.protectionSpace.serverTrust else {
            completionHandler(.performDefaultHandling, nil)
            return
        }
        completionHandler(.useCredential, URLCredential(trust: serverTrust))
    }
}

actor DappClient {
    let origin: URL

    private let session: URLSession

    /// Creates a client for the loopback actor started by the iOS harness.
    init(origin: URL) {
        self.origin = origin
        let configuration = URLSessionConfiguration.ephemeral
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.urlCache = nil
        session = URLSession(
            configuration: configuration,
            delegate: DappLoopbackTrustDelegate(),
            delegateQueue: nil
        )
    }

    /// Clears the SDK connection and observations retained by an earlier scenario.
    func reset() async throws {
        _ = try await command(body: ["type": "reset"])
    }

    /// Asks the official SDK actor to create the TON Connect link used by the wallet.
    func createConnectLink() async throws -> String {
        struct Response: Decodable {
            let link: String
        }
        let data = try await command(body: ["type": "connect"])
        let response = try JSONDecoder().decode(Response.self, from: data)
        return response.link
    }

    /// Sends the exact rendered transaction request through the official SDK actor.
    func requestTransaction(_ transaction: RenderedTransactionRequest) async throws {
        let transactionData = try JSONEncoder().encode(transaction)
        let transactionObject = try JSONSerialization.jsonObject(with: transactionData)
        let body: [String: Any] = [
            "transaction": transactionObject,
            "type": "send_transaction",
        ]
        _ = try await command(body: body)
    }

    /// Reads the latest account, request, response, and journal observed by the dApp.
    func state() async throws -> DappActorState {
        let url = origin.appendingPathComponent("state")
        let (data, response) = try await session.data(from: url)
        try validate(response: response, data: data)
        return try JSONDecoder().decode(DappActorState.self, from: data)
    }

    /// Waits until the dApp state satisfies one protocol-level expectation.
    func wait(
        description: String,
        timeout: Duration = .seconds(10),
        predicate: @Sendable (DappActorState) -> Bool
    ) async throws -> DappActorState {
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: timeout)
        while clock.now < deadline {
            let candidate = try await state()
            if predicate(candidate) {
                return candidate
            }
            try await Task.sleep(for: .milliseconds(50))
        }
        throw DappClientError.timeout(description)
    }

    /// Posts one JSON command and returns its successful actor response body.
    private func command(body: [String: Any]) async throws -> Data {
        var request = URLRequest(url: origin.appendingPathComponent("command"))
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: body)
        let (data, rawResponse) = try await session.data(for: request)
        try validate(response: rawResponse, data: data)
        return data
    }

    /// Rejects non-success responses while preserving the actor's diagnostic body.
    private func validate(response: URLResponse, data: Data) throws {
        guard let response = response as? HTTPURLResponse else {
            throw DappClientError.invalidResponse
        }
        guard (200..<300).contains(response.statusCode) else {
            throw DappClientError.requestFailed(
                status: response.statusCode,
                body: String(data: data, encoding: .utf8) ?? ""
            )
        }
    }
}
