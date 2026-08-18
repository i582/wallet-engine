import Foundation
import WalletEngineFFI

nonisolated final class TonConnectRedirectDelegate: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        completionHandler(nil)
    }
}

/// Native HTTP bridge and manifest transport. Protocol bytes stay opaque.
actor TonConnectTransport {
    private static let maximumManifestBytes = 256 * 1024

    private let session: URLSession

    init(configuration: URLSessionConfiguration = .ephemeral) {
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.urlCache = nil
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.timeoutIntervalForRequest = 30
        // The bridge event endpoint is a long-polling stream. Its lifetime must not be
        // capped by the short request budget used for manifests and bridge messages.
        configuration.timeoutIntervalForResource = 24 * 60 * 60
        session = URLSession(
            configuration: configuration,
            delegate: TonConnectRedirectDelegate(),
            delegateQueue: nil
        )
    }

    func loadManifest(from value: String) async throws -> String {
        let request = try request(url: value, method: "GET")
        let (data, response) = try await session.data(for: request)
        try validate(response)
        guard data.count <= Self.maximumManifestBytes else {
            throw TonConnectTransportError.manifestTooLarge
        }
        guard let manifest = String(data: data, encoding: .utf8) else {
            throw TonConnectTransportError.invalidUtf8
        }
        return manifest
    }

    func post(_ prepared: TonConnectPreparedPost) async throws {
        var request = try request(url: prepared.url, method: "POST")
        request.setValue("text/plain; charset=utf-8", forHTTPHeaderField: "Content-Type")
        request.httpBody = Data(prepared.body.utf8)
        let (_, response) = try await session.data(for: request)
        try validate(response)
    }

    func stream(
        from value: String,
        onChunk: @escaping @Sendable (Data) async throws -> Void
    ) async throws {
        var request = try request(url: value, method: "GET")
        request.timeoutInterval = 5 * 60
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        let (bytes, response) = try await session.bytes(for: request)
        try validate(response)

        var chunk = Data()
        chunk.reserveCapacity(4 * 1024)
        for try await byte in bytes {
            try Task.checkCancellation()
            chunk.append(byte)
            if byte == 0x0A || chunk.count >= 4 * 1024 {
                try await onChunk(chunk)
                chunk.removeAll(keepingCapacity: true)
            }
        }
        if !chunk.isEmpty {
            try await onChunk(chunk)
        }
    }

    private func request(url value: String, method: String) throws -> URLRequest {
        guard let url = URL(string: value),
              url.scheme?.lowercased() == "https",
              url.host != nil,
              url.user == nil,
              url.password == nil,
              url.fragment == nil else {
            throw TonConnectTransportError.invalidUrl
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.cachePolicy = .reloadIgnoringLocalCacheData
        return request
    }

    private func validate(_ response: URLResponse) throws {
        guard let response = response as? HTTPURLResponse,
              (200..<300).contains(response.statusCode) else {
            throw TonConnectTransportError.invalidResponse
        }
    }
}

nonisolated enum TonConnectTransportError: LocalizedError, Sendable {
    case invalidUrl
    case invalidResponse
    case invalidUtf8
    case manifestTooLarge

    var errorDescription: String? {
        switch self {
        case .invalidUrl:
            "TON Connect URL is invalid."
        case .invalidResponse:
            "TON Connect server returned an unsuccessful response."
        case .invalidUtf8:
            "TON Connect manifest is not UTF-8."
        case .manifestTooLarge:
            "TON Connect manifest exceeds 256 KiB."
        }
    }
}
