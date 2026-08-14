import Foundation
import WalletEngineFFI

/// A credential that is owned by the Apple host and is never copied into Rust.
nonisolated struct WalletCredential: Sendable {
    let reference: String
    let headerName: String
    let secret: String
}

/// Security policy for callback-driven  HTTP calls.
///
/// Rust owns request construction and response parsing. The host owns transport,
/// credentials, redirect rejection, and byte limits.
nonisolated struct AppleWalletHTTPPolicy: Sendable {
    private enum Limits {
        static let maximumRequestBodyBytes = 256 * 1024
        static let maximumRequestHeaders = 32
        static let maximumResponseHeaders = 64
        static let maximumResponseHeaderBytes = 64 * 1024
        static let maximumResponseBodyBytes = 4 * 1024 * 1024
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
    private let credentials: [String: WalletCredential]

    init(
        allowedOrigins: [String],
        credentials: [WalletCredential] = []
    ) {
        self.allowedOrigins = Set(
            allowedOrigins.compactMap(Self.normalizedHTTPSOrigin(forOrigin:))
        )
        self.credentials = Dictionary(
            credentials.map { ($0.reference, $0) },
            uniquingKeysWith: { _, replacement in replacement }
        )
    }

    func prepare(_ request: HttpRequest) throws -> URLRequest {
        guard request.body.count <= Limits.maximumRequestBodyBytes else {
            throw Self.failure(.policyViolation, "Request body exceeds 256 KiB")
        }
        guard request.headers.count <= Limits.maximumRequestHeaders else {
            throw Self.failure(.policyViolation, "Too many request headers")
        }
        guard request.maxResponseBodyBytes > 0,
              request.maxResponseBodyBytes <= Limits.maximumResponseBodyBytes,
              request.maxResponseHeaderBytes > 0,
              request.maxResponseHeaderBytes <= Limits.maximumResponseHeaderBytes else {
            throw Self.failure(.policyViolation, "Invalid response size limit")
        }
        guard let url = URL(string: request.url),
              let origin = Self.normalizedHTTPSOrigin(for: url),
              allowedOrigins.contains(origin),
              url.user == nil,
              url.password == nil,
              url.fragment == nil else {
            throw Self.failure(.policyViolation, "Request URL is not an allowed HTTPS URL")
        }

        var urlRequest = URLRequest(url: url)
        switch request.method {
        case .get:
            urlRequest.httpMethod = "GET"
        case .post:
            urlRequest.httpMethod = "POST"
        }
        urlRequest.httpBody = request.body.isEmpty ? nil : request.body
        urlRequest.timeoutInterval = 30
        urlRequest.cachePolicy = .reloadIgnoringLocalCacheData

        var seenHeaders = Set<String>()
        for header in request.headers {
            try Self.validateHeader(name: header.name, value: header.value)
            let normalizedName = header.name.lowercased()
            guard !Self.forbiddenRequestHeaderNames.contains(normalizedName),
                  !credentials.values.contains(where: {
                      $0.headerName.caseInsensitiveCompare(header.name) == .orderedSame
                  }) else {
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

        switch (request.credential, request.credentialOrigin) {
        case (nil, nil):
            break
        case let (.some(reference), .some(declaredOrigin)):
            guard let credential = credentials[reference.value],
                  !credential.secret.isEmpty,
                  let rustOrigin = Self.normalizedHTTPSOrigin(
                      forOrigin: declaredOrigin
                  ),
                  rustOrigin == origin else {
                throw Self.failure(
                    .policyViolation,
                    "Credential is unavailable or forbidden for this origin"
                )
            }
            try Self.validateHeader(
                name: credential.headerName,
                value: credential.secret
            )
            guard seenHeaders.insert(credential.headerName.lowercased()).inserted else {
                throw Self.failure(
                    .policyViolation,
                    "Credential header must be owned by the host"
                )
            }
            urlRequest.setValue(
                credential.secret,
                forHTTPHeaderField: credential.headerName
            )
        default:
            throw Self.failure(.policyViolation, "Incomplete credential policy")
        }

        return urlRequest
    }

    func responseHeaders(
        _ response: HTTPURLResponse,
        limit: UInt64
    ) throws -> [HttpHeader] {
        guard limit > 0, limit <= Limits.maximumResponseHeaderBytes else {
            throw Self.failure(.policyViolation, "Invalid response header limit")
        }
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
            guard totalBytes <= Int(limit) else {
                throw Self.failure(
                    .responseTooLarge,
                    "Response headers exceed the declared limit"
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

        return !credentials.values.contains { credential in
            credential.headerName.caseInsensitiveCompare(name) == .orderedSame
                || (!credential.secret.isEmpty && value.contains(credential.secret))
        }
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

    static func normalizedHTTPSOrigin(for url: URL) -> String? {
        guard url.scheme?.lowercased() == "https",
              let host = url.host?.lowercased(),
              !host.isEmpty else {
            return nil
        }
        return "https://\(host):\(url.port ?? 443)"
    }

    static func normalizedHTTPSOrigin(forOrigin value: String) -> String? {
        guard let url = URL(string: value),
              url.path.isEmpty || url.path == "/",
              url.query == nil,
              url.fragment == nil,
              url.user == nil,
              url.password == nil else {
            return nil
        }
        return normalizedHTTPSOrigin(for: url)
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
        configuration.timeoutIntervalForRequest = 30
        configuration.timeoutIntervalForResource = 30
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
        let task = Task<HttpResponse, Error> {
            try await Self.perform(request, policy: policy, session: session)
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
        policy: AppleWalletHTTPPolicy,
        session: URLSession
    ) async throws -> HttpResponse {
        do {
            let urlRequest = try policy.prepare(request)
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

            let headers = try policy.responseHeaders(
                response,
                limit: request.maxResponseHeaderBytes
            )
            let body = try await collect(
                bytes,
                limit: request.maxResponseBodyBytes
            )
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

    private nonisolated static func collect(
        _ bytes: URLSession.AsyncBytes,
        limit: UInt64
    ) async throws -> Data {
        guard let integerLimit = Int(exactly: limit) else {
            throw failure(.responseTooLarge, "Response body limit is invalid")
        }
        var body = Data()
        body.reserveCapacity(min(integerLimit, 64 * 1024))

        for try await byte in bytes {
            try Task.checkCancellation()
            guard body.count < integerLimit else {
                throw failure(
                    .responseTooLarge,
                    "Response body exceeds the declared limit"
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
