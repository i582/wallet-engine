import Foundation
import WalletEngineFFI

/// Security policy for callback-driven  HTTP calls.
///
/// Rust owns request construction and response parsing. The host owns transport,
/// credentials, redirect rejection, and byte limits.
nonisolated struct AppleWalletHTTPPolicy: Sendable {
    fileprivate enum Limits {
        static let maximumRequestBodyBytes = 256 * 1024
        static let maximumRequestHeaders = 32
        static let maximumResponseHeaders = 64
        static let maximumResponseHeaderBytes = 64 * 1024
        static let maximumResponseBodyBytes = 4 * 1024 * 1024
        static let maximumRequestTimeoutMs: UInt64 = 5 * 60 * 1000
    }

    private static let forbiddenRequestHeaderNames: Set<String> = [
        "authorization",
        "proxy-authorization",
        "cookie",
        "host",
        "content-length",
        "transfer-encoding",
        "connection",
        "proxy-connection",
        "keep-alive",
        "te",
        "trailer",
        "upgrade",
        "x-api-key",
    ]

    private let allowedOrigins: Set<String>
    private let allowInsecureLoopback: Bool
    private let toncenterAPIKey: String?

    init(
        allowedOrigins: [String],
        allowInsecureLoopback: Bool = false,
        toncenterAPIKey: String? = nil
    ) {
        self.allowInsecureLoopback = allowInsecureLoopback
        self.allowedOrigins = Set(
            allowedOrigins.compactMap {
                Self.normalizedOrigin(
                    forOrigin: $0,
                    allowInsecureLoopback: allowInsecureLoopback
                )
            }
        )
        self.toncenterAPIKey = toncenterAPIKey
    }

    func prepare(_ request: HttpRequest) throws -> URLRequest {
        guard request.body.count <= Limits.maximumRequestBodyBytes else {
            throw Self.failure(.policyViolation, "Request body exceeds 256 KiB")
        }
        guard request.headers.count <= Limits.maximumRequestHeaders else {
            throw Self.failure(.policyViolation, "Too many request headers")
        }
        guard request.timeoutMs > 0,
              request.timeoutMs <= Limits.maximumRequestTimeoutMs else {
            throw Self.failure(.policyViolation, "Invalid request timeout")
        }
        guard let url = URL(string: request.url),
              let origin = Self.normalizedOrigin(
                  for: url,
                  allowInsecureLoopback: allowInsecureLoopback
              ),
              allowedOrigins.contains(origin),
              url.user == nil,
              url.password == nil,
              url.fragment == nil else {
            throw Self.failure(.policyViolation, "Request URL is not an allowed provider URL")
        }

        var urlRequest = URLRequest(url: url)
        switch request.method {
        case .get:
            urlRequest.httpMethod = "GET"
        case .post:
            urlRequest.httpMethod = "POST"
        }
        urlRequest.httpBody = request.body.isEmpty ? nil : request.body
        urlRequest.timeoutInterval = TimeInterval(request.timeoutMs) / 1_000
        urlRequest.cachePolicy = .reloadIgnoringLocalCacheData

        var seenHeaders = Set<String>()
        for header in request.headers {
            try Self.validateHeader(name: header.name, value: header.value)
            let normalizedName = header.name.lowercased()
            guard !Self.forbiddenRequestHeaderNames.contains(normalizedName) else {
                throw Self.failure(
                    .policyViolation,
                    "Request contains a host-owned HTTP header"
                )
            }
            guard seenHeaders.insert(normalizedName).inserted else {
                throw Self.failure(.policyViolation, "Duplicate request header")
            }
            urlRequest.setValue(header.value, forHTTPHeaderField: header.name)
        }

        if let toncenterAPIKey, !toncenterAPIKey.isEmpty {
            try Self.validateHeader(name: "X-API-Key", value: toncenterAPIKey)
            urlRequest.setValue(
                toncenterAPIKey,
                forHTTPHeaderField: "X-API-Key"
            )
        }

        return urlRequest
    }

    func responseHeaders(_ response: HTTPURLResponse) throws -> [HttpHeader] {
        guard response.allHeaderFields.count <= Limits.maximumResponseHeaders else {
            throw Self.failure(.responseTooLarge, "Too many response headers")
        }

        var totalBytes = 0
        var headers = [HttpHeader]()
        headers.reserveCapacity(response.allHeaderFields.count)

        for (rawName, rawValue) in response.allHeaderFields {
            let name = String(describing: rawName)
            let value = String(describing: rawValue)
            totalBytes += name.utf8.count + value.utf8.count
            guard totalBytes <= Limits.maximumResponseHeaderBytes else {
                throw Self.failure(
                    .responseTooLarge,
                    "Response headers exceed the host limit"
                )
            }
            try Self.validateHeader(name: name, value: value)
            guard mayExposeResponseHeader(name: name, value: value) else {
                continue
            }
            headers.append(
                HttpHeader(name: name.lowercased(), value: value)
            )
        }

        return headers.sorted {
            ($0.name.lowercased(), $0.value) < ($1.name.lowercased(), $1.value)
        }
    }

    private func mayExposeResponseHeader(name: String, value: String) -> Bool {
        let sensitiveNames: Set<String> = [
            "authorization",
            "proxy-authorization",
            "cookie",
            "set-cookie",
            "x-api-key",
        ]
        guard !sensitiveNames.contains(name.lowercased()) else { return false }

        return toncenterAPIKey.map { !value.contains($0) } ?? true
    }

    private static func validateHeader(name: String, value: String) throws {
        let separators = CharacterSet(
            charactersIn: "()<>@,;:\\\"/[]?={} \t"
        )
        guard !name.isEmpty,
              name.unicodeScalars.allSatisfy({ scalar in
                  scalar.value > 31
                      && scalar.value < 127
                      && !separators.contains(scalar)
              }),
              !value.unicodeScalars.contains(where: {
                  $0.value == 10 || $0.value == 13
              }) else {
            throw failure(.policyViolation, "Invalid HTTP header")
        }
    }

    /// Normalizes an HTTPS origin and optionally permits HTTP on loopback.
    static func normalizedOrigin(
        for url: URL,
        allowInsecureLoopback: Bool
    ) -> String? {
        guard let scheme = url.scheme?.lowercased(),
              let host = url.host?.lowercased(),
              !host.isEmpty,
              scheme == "https"
                || (allowInsecureLoopback
                    && scheme == "http"
                    && AppleRuntimeConfiguration.isLoopback(host)) else {
            return nil
        }
        let defaultPort = scheme == "https" ? 443 : 80
        return "\(scheme)://\(host):\(url.port ?? defaultPort)"
    }

    /// Normalizes one configured origin without accepting paths or credentials.
    static func normalizedOrigin(
        forOrigin value: String,
        allowInsecureLoopback: Bool
    ) -> String? {
        guard let url = URL(string: value),
              url.path.isEmpty || url.path == "/",
              url.query == nil,
              url.fragment == nil,
              url.user == nil,
              url.password == nil else {
            return nil
        }
        return normalizedOrigin(
            for: url,
            allowInsecureLoopback: allowInsecureLoopback
        )
    }

    private static func failure(
        _ kind: HttpHostErrorKind,
        _ message: String
    ) -> HttpHostError {
        .Failed(kind: kind, diagnostic: sanitized(message))
    }

    private static func sanitized(_ message: String) -> String {
        String(
            message.unicodeScalars
                .map {
                    CharacterSet.controlCharacters.contains($0) ? " " : String($0)
                }
                .joined()
                .prefix(256)
        )
    }
}

/// Native HTTP implementation for the  Rust callback interface.
///
/// The registry provides explicit cancellation even when Rust cancels before
/// URLSession has created its underlying task.
actor AppleWalletHTTPHost: WalletHttpHost {
    private static let maximumEarlyCancellations = 256
    private static let maximumRequestTimeout: TimeInterval = 5 * 60

    private let policy: AppleWalletHTTPPolicy
    private let session: URLSession
    private var tasks: [UInt64: Task<HttpResponse, Error>] = [:]
    private var cancelledBeforeStart = Set<UInt64>()

    init(
        policy: AppleWalletHTTPPolicy,
        configuration: URLSessionConfiguration = .ephemeral
    ) {
        self.policy = policy
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.urlCache = nil
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.timeoutIntervalForRequest = Self.maximumRequestTimeout
        configuration.timeoutIntervalForResource = Self.maximumRequestTimeout
        session = URLSession(configuration: configuration)
    }

    func executeHttp(request: HttpRequest) async throws -> HttpResponse {
        let id = request.id.value
        guard tasks[id] == nil else {
            throw Self.failure(.policyViolation, "Duplicate HTTP request identifier")
        }
        guard cancelledBeforeStart.remove(id) == nil else {
            throw Self.failure(.cancelled, "HTTP request was cancelled")
        }

        let policy = policy
        let session = session
        let urlRequest = try policy.prepare(request)
        let task = Task<HttpResponse, Error> {
            try await Self.performWithDeadline(
                request,
                urlRequest: urlRequest,
                policy: policy,
                session: session
            )
        }
        tasks[id] = task
        defer { tasks[id] = nil }

        do {
            return try await task.value
        } catch is CancellationError {
            throw Self.failure(.cancelled, "HTTP request was cancelled")
        } catch let error as HttpHostError {
            throw error
        } catch {
            throw Self.failure(.other, String(describing: error))
        }
    }

    func cancelHttp(requestId: HttpRequestId) async {
        if let task = tasks[requestId.value] {
            task.cancel()
        } else {
            cancelledBeforeStart.insert(requestId.value)
            while cancelledBeforeStart.count > Self.maximumEarlyCancellations,
                  let oldest = cancelledBeforeStart.min() {
                cancelledBeforeStart.remove(oldest)
            }
        }
    }

    private nonisolated static func perform(
        _ request: HttpRequest,
        urlRequest: URLRequest,
        policy: AppleWalletHTTPPolicy,
        session: URLSession
    ) async throws -> HttpResponse {
        do {
            let redirectDelegate = WalletHTTPRedirectDelegate()
            let (bytes, rawResponse) = try await session.bytes(
                for: urlRequest,
                delegate: redirectDelegate
            )
            guard let response = rawResponse as? HTTPURLResponse else {
                throw failure(.policyViolation, "HTTP response metadata is missing")
            }
            guard response.url?.absoluteString == request.url else {
                throw failure(.policyViolation, "HTTP redirect was not allowed")
            }
            guard let status = UInt16(exactly: response.statusCode) else {
                throw failure(.policyViolation, "HTTP status is outside UInt16")
            }

            let headers = try policy.responseHeaders(response)
            let body = try await collect(bytes)
            return HttpResponse(
                status: status,
                headers: headers,
                body: body,
                finalUrl: request.url
            )
        } catch is CancellationError {
            throw failure(.cancelled, "HTTP request was cancelled")
        } catch let error as HttpHostError {
            throw error
        } catch let error as URLError {
            if error.code == .cancelled {
                throw failure(.cancelled, "HTTP request was cancelled")
            }
            throw failure(
                transportKind(for: error.code),
                error.localizedDescription
            )
        } catch {
            throw failure(.other, String(describing: error))
        }
    }

    private nonisolated static func performWithDeadline(
        _ request: HttpRequest,
        urlRequest: URLRequest,
        policy: AppleWalletHTTPPolicy,
        session: URLSession
    ) async throws -> HttpResponse {
        try await withThrowingTaskGroup(of: HttpResponse.self) { group in
            group.addTask {
                try await perform(
                    request,
                    urlRequest: urlRequest,
                    policy: policy,
                    session: session
                )
            }
            group.addTask {
                try await Task.sleep(
                    nanoseconds: request.timeoutMs * 1_000_000
                )
                throw failure(.timeout, "Provider request timed out")
            }
            defer { group.cancelAll() }

            guard let response = try await group.next() else {
                throw failure(.other, "HTTP request task did not produce a result")
            }
            return response
        }
    }

    private nonisolated static func collect(
        _ bytes: URLSession.AsyncBytes
    ) async throws -> Data {
        let integerLimit = AppleWalletHTTPPolicy.Limits.maximumResponseBodyBytes
        var body = Data()
        body.reserveCapacity(min(integerLimit, 64 * 1024))

        for try await byte in bytes {
            try Task.checkCancellation()
            guard body.count < integerLimit else {
                throw failure(
                    .responseTooLarge,
                    "Response body exceeds the host limit"
                )
            }
            body.append(byte)
        }
        return body
    }

    private nonisolated static func transportKind(
        for code: URLError.Code
    ) -> HttpHostErrorKind {
        switch code {
        case .notConnectedToInternet,
             .internationalRoamingOff,
             .dataNotAllowed,
             .callIsActive:
            .offline
        case .timedOut:
            .timeout
        case .networkConnectionLost, .cannotConnectToHost:
            .connectionLost
        case .cannotFindHost, .dnsLookupFailed:
            .dns
        case .secureConnectionFailed,
             .serverCertificateHasBadDate,
             .serverCertificateUntrusted,
             .serverCertificateHasUnknownRoot,
             .serverCertificateNotYetValid,
             .clientCertificateRejected,
             .clientCertificateRequired:
            .tls
        default:
            .other
        }
    }

    private nonisolated static func failure(
        _ kind: HttpHostErrorKind,
        _ message: String
    ) -> HttpHostError {
        .Failed(
            kind: kind,
            diagnostic: String(
                message.unicodeScalars
                    .map {
                        CharacterSet.controlCharacters.contains($0) ? " " : String($0)
                    }
                    .joined()
                    .prefix(256)
            )
        )
    }
}

private nonisolated final class WalletHTTPRedirectDelegate: NSObject,
    URLSessionTaskDelegate,
    @unchecked Sendable
{
    func urlSession(
        _: URLSession,
        task _: URLSessionTask,
        willPerformHTTPRedirection _: HTTPURLResponse,
        newRequest _: URLRequest,
        completionHandler: @escaping @Sendable (URLRequest?) -> Void
    ) {
        completionHandler(nil)
    }
}
