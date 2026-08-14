import CryptoKit
import Darwin
import Foundation
import LocalAuthentication
import Security
import WalletEngineFFI

/// Wall clock used only for protocol timestamps such as `valid_until`.
actor AppleWalletClock {
    func now() -> UInt64 {
        let seconds = Date.now.timeIntervalSince1970.rounded(.down)
        guard seconds.isFinite, seconds >= 0 else { return 0 }
        return UInt64(seconds)
    }
}

/// Resolves opaque  secret references through the system Keychain.
/// A reference identifies an item; Rust never receives the Keychain service
/// name or any persistence details.
actor AppleWalletProtectedSecretStore {
    private let service: String

    init(
        service: String = "\(Bundle.main.bundleIdentifier ?? "WalletEngine").protected-secret"
    ) {
        self.service = service
    }

    func read(_ request: ProtectedSecretRead) async throws -> Data {
        let service = service
        do {
            return try await Task.detached(priority: .userInitiated) {
                try Self.readFromKeychain(
                    reference: request.secretRef.value,
                    service: service,
                    reason: Self.localizedReason(for: request.reason)
                )
            }.value
        } catch is CancellationError {
            throw Self.failure(.cancelled, "Secret access was cancelled")
        } catch let error as ProtectedSecretHostError {
            throw error
        } catch {
            throw Self.failure(.other, "Protected storage failed")
        }
    }

    func store(_ request: ProtectedSecretStore) async throws {
        guard request.requireUserPresence else {
            throw Self.failure(
                .policyViolation,
                "Wallet secrets must require device authentication"
            )
        }
        let service = service
        do {
            try await Task.detached(priority: .userInitiated) {
                try Self.storeInKeychain(
                    request.bytes,
                    reference: request.secretRef.value,
                    service: service
                )
            }.value
        } catch is CancellationError {
            throw Self.failure(.cancelled, "Secret storage was cancelled")
        } catch let error as ProtectedSecretHostError {
            throw error
        } catch {
            throw Self.failure(.other, "Protected storage failed")
        }
    }

    func delete(_ reference: ProtectedSecretRef) async throws {
        let service = service
        do {
            try await Self.authenticateForSecretDeletion()
            try await Task.detached(priority: .userInitiated) {
                var query: [String: Any] = [
                    kSecClass as String: kSecClassGenericPassword,
                    kSecAttrService as String: service,
                    kSecAttrAccount as String: reference.value,
                ]
                Self.addPlatformKeychainAttributes(to: &query)
                let status = SecItemDelete(query as CFDictionary)
                guard status == errSecSuccess || status == errSecItemNotFound else {
                    throw Self.keychainFailure(status)
                }
            }.value
        } catch is CancellationError {
            throw Self.failure(.cancelled, "Secret deletion was cancelled")
        } catch let error as ProtectedSecretHostError {
            throw error
        } catch {
            throw Self.failure(.other, "Protected storage failed")
        }
    }

    private nonisolated static func readFromKeychain(
        reference: String,
        service: String,
        reason: String
    ) throws -> Data {
        let context = LAContext()
        context.localizedReason = reason.isEmpty
            ? "Authenticate to access this wallet."
            : reason
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: reference,
            kSecMatchLimit as String: kSecMatchLimitOne,
            kSecReturnData as String: true,
            kSecUseAuthenticationContext as String: context,
        ]
        addPlatformKeychainAttributes(to: &query)

        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess else {
            throw keychainFailure(status)
        }
        guard let data = item as? Data, !data.isEmpty else {
            throw failure(.policyViolation, "Protected secret is empty")
        }
        return data
    }

    private nonisolated static func localizedReason(
        for reason: SecretAccessReason
    ) -> String {
        switch reason {
        case .createWallet:
            "Authenticate to protect this wallet."
        case .signTransfer:
            "Authenticate to sign this GRAM transfer."
        case .revealRecoveryPhrase:
            "Authenticate to reveal this wallet's recovery phrase."
        }
    }

    private nonisolated static func authenticateForSecretDeletion() async throws {
        let context = LAContext()
        context.localizedCancelTitle = "Cancel"
        do {
            guard try await context.evaluatePolicy(
                .deviceOwnerAuthentication,
                localizedReason: "Authenticate to delete this wallet."
            ) else {
                throw failure(.authenticationFailed, "Device authentication failed")
            }
        } catch let error as ProtectedSecretHostError {
            throw error
        } catch let error as LAError {
            switch error.code {
            case .userCancel, .systemCancel, .appCancel:
                throw failure(.cancelled, "Wallet deletion was cancelled")
            case .authenticationFailed:
                throw failure(.authenticationFailed, "Device authentication failed")
            case .biometryNotAvailable,
                 .biometryNotEnrolled,
                 .passcodeNotSet,
                 .notInteractive:
                throw failure(.unavailable, "Device authentication is unavailable")
            default:
                throw failure(.other, "Device authentication failed")
            }
        } catch {
            throw failure(.other, "Device authentication failed")
        }
    }

    private nonisolated static func storeInKeychain(
        _ bytes: Data,
        reference: String,
        service: String
    ) throws {
        guard !reference.isEmpty, !bytes.isEmpty else {
            throw failure(.policyViolation, "Protected secret is empty")
        }

        var accessError: Unmanaged<CFError>?
        let access = SecAccessControlCreateWithFlags(
            nil,
            accessibility,
            .userPresence,
            &accessError
        )
        guard let access else {
            if let error = accessError?.takeRetainedValue() {
                throw failure(.unavailable, error.localizedDescription)
            }
            throw failure(.unavailable, "Device authentication is unavailable")
        }

        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: reference,
            kSecValueData as String: bytes,
            kSecAttrAccessControl as String: access,
        ]
        addPlatformKeychainAttributes(to: &query)

        let status = SecItemAdd(query as CFDictionary, nil)
        guard status == errSecSuccess else {
            if status == errSecDuplicateItem {
                throw failure(
                    .policyViolation,
                    "A protected secret already exists for this reference"
                )
            }
            throw keychainFailure(status)
        }
    }

    private nonisolated static func keychainFailure(
        _ status: OSStatus
    ) -> ProtectedSecretHostError {
        switch status {
        case errSecItemNotFound:
            failure(.notFound, "Protected secret was not found")
        case errSecAuthFailed:
            failure(.authenticationFailed, "Device authentication failed")
        case errSecUserCanceled:
            failure(.cancelled, "Secret access was cancelled")
        case errSecInteractionNotAllowed:
            failure(.unavailable, "Protected storage interaction is unavailable")
        default:
            failure(.other, "Protected storage failed (\(status))")
        }
    }

    private nonisolated static var accessibility: CFString {
#if os(iOS)
        kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly
#else
        kSecAttrAccessibleWhenUnlockedThisDeviceOnly
#endif
    }

    private nonisolated static func addPlatformKeychainAttributes(
        to query: inout [String: Any]
    ) {
#if targetEnvironment(simulator)
        query[kSecUseDataProtectionKeychain as String] = false
#elseif os(iOS)
        query[kSecAttrSynchronizable as String] = false
#elseif DEBUG
        // A local macOS Debug build may only have Xcode's ad-hoc
        // "Sign to Run Locally" signature. Such a signature has no stable
        // application identifier and cannot access the data-protection
        // keychain (errSecMissingEntitlement). Keep user-presence access
        // control, but use the file-based login keychain for this local build.
        query[kSecUseDataProtectionKeychain as String] = false
        query[kSecAttrSynchronizable as String] = false
#else
        query[kSecUseDataProtectionKeychain as String] = true
        query[kSecAttrSynchronizable as String] = false
#endif
    }

    private nonisolated static func failure(
        _ kind: ProtectedSecretHostErrorKind,
        _ message: String
    ) -> ProtectedSecretHostError {
        .Failed(kind: kind, diagnostic: sanitized(message))
    }
}

/// Durable, actor-serialized compare-and-swap journal for  send operations.
///
/// The journal never stores a mnemonic. Its opaque payload is produced by
/// Rust and is written atomically before a prepared transfer may be submitted.
actor AppleWalletJournalStore {
    static let shared = AppleWalletJournalStore()

    private struct DiskRecord: Codable {
        let version: UInt64
        let payload: Data
    }

    private let directoryURL: URL
    private let fileManager: FileManager
    private let encoder = PropertyListEncoder()
    private let decoder = PropertyListDecoder()

    init(
        directoryURL: URL? = nil,
        fileManager: FileManager = .default
    ) {
        self.fileManager = fileManager
        self.directoryURL = directoryURL ?? Self.defaultDirectoryURL(
            fileManager: fileManager
        )
        encoder.outputFormat = .binary
    }

    func load(_ key: JournalKey) throws -> JournalRecord? {
        try ensureDirectory()
        return try withExclusiveFileLock {
            try loadUnlocked(key)
        }
    }

    private func loadUnlocked(_ key: JournalKey) throws -> JournalRecord? {
        let url = recordURL(for: key)
        guard fileManager.fileExists(atPath: url.path) else { return nil }

        do {
            let data = try Data(contentsOf: url, options: .mappedIfSafe)
            let record = try decoder.decode(DiskRecord.self, from: data)
            return JournalRecord(
                version: record.version,
                payload: record.payload
            )
        } catch let error as JournalHostError {
            throw error
        } catch let error as DecodingError {
            throw Self.failure(.corruptData, String(describing: error))
        } catch {
            throw Self.failure(.unavailable, String(describing: error))
        }
    }

    func compareExchange(
        _ mutation: JournalCompareExchange
    ) throws -> JournalCompareExchangeResult {
        try ensureDirectory()
        return try withExclusiveFileLock {
            let current = try loadUnlocked(mutation.key)
            guard current?.version == mutation.expectedVersion else {
                return JournalCompareExchangeResult(
                    applied: false,
                    current: current
                )
            }

            let diskRecord = DiskRecord(
                version: mutation.replacement.version,
                payload: mutation.replacement.payload
            )
            do {
                let encoded = try encoder.encode(diskRecord)
                try writeDurably(
                    encoded,
                    to: recordURL(for: mutation.key)
                )
                return JournalCompareExchangeResult(
                    applied: true,
                    current: mutation.replacement
                )
            } catch let error as JournalHostError {
                throw error
            } catch {
                throw Self.failure(.unavailable, String(describing: error))
            }
        }
    }

    /// Publishes one complete record without ever exposing a partially-written
    /// destination. The temporary file is flushed before the atomic rename;
    /// the containing directory is flushed afterwards when the filesystem
    /// supports directory `fsync`.
    private func writeDurably(_ data: Data, to destinationURL: URL) throws {
        let temporaryURL = directoryURL.appendingPathComponent(
            ".\(destinationURL.lastPathComponent).\(UUID().uuidString).tmp",
            isDirectory: false
        )
        var descriptor = Darwin.open(
            temporaryURL.path,
            O_CREAT | O_EXCL | O_WRONLY | O_CLOEXEC,
            S_IRUSR | S_IWUSR
        )
        guard descriptor >= 0 else {
            throw Self.posixFailure("Could not create a journal transaction")
        }

        var shouldRemoveTemporaryFile = true
        defer {
            if descriptor >= 0 {
                Darwin.close(descriptor)
            }
            if shouldRemoveTemporaryFile {
                Darwin.unlink(temporaryURL.path)
            }
        }

        guard fchmod(descriptor, S_IRUSR | S_IWUSR) == 0 else {
            throw Self.posixFailure("Could not protect the journal transaction")
        }

        try data.withUnsafeBytes { rawBuffer in
            guard let baseAddress = rawBuffer.baseAddress else { return }
            var offset = 0
            while offset < rawBuffer.count {
                let written = Darwin.write(
                    descriptor,
                    baseAddress.advanced(by: offset),
                    rawBuffer.count - offset
                )
                if written < 0, errno == EINTR {
                    continue
                }
                guard written > 0 else {
                    throw Self.posixFailure("Could not write the send journal")
                }
                offset += written
            }
        }

        guard Darwin.fsync(descriptor) == 0 else {
            throw Self.posixFailure("Could not flush the send journal")
        }
        guard Darwin.close(descriptor) == 0 else {
            let code = errno
            descriptor = -1
            throw Self.posixFailure(
                "Could not close the send journal transaction",
                code: code
            )
        }
        descriptor = -1

        guard Darwin.rename(temporaryURL.path, destinationURL.path) == 0 else {
            throw Self.posixFailure("Could not publish the send journal")
        }
        shouldRemoveTemporaryFile = false
        try flushDirectoryIfSupported()
    }

    private func flushDirectoryIfSupported() throws {
        let descriptor = Darwin.open(directoryURL.path, O_RDONLY | O_CLOEXEC)
        guard descriptor >= 0 else {
            throw Self.posixFailure("Could not open the send journal directory")
        }
        defer { Darwin.close(descriptor) }

        guard Darwin.fsync(descriptor) == 0 else {
            let code = errno
            if code == EINVAL || code == ENOTSUP {
                return
            }
            throw Self.posixFailure(
                "Could not flush the send journal directory",
                code: code
            )
        }
    }

    private func withExclusiveFileLock<T>(
        _ operation: () throws -> T
    ) throws -> T {
        let lockURL = directoryURL.appendingPathComponent(
            ".journal.lock",
            isDirectory: false
        )
        let descriptor = Darwin.open(
            lockURL.path,
            O_CREAT | O_RDWR | O_CLOEXEC,
            S_IRUSR | S_IWUSR
        )
        guard descriptor >= 0 else {
            throw Self.posixFailure("Could not open the journal lock")
        }
        defer { Darwin.close(descriptor) }

        guard fchmod(descriptor, S_IRUSR | S_IWUSR) == 0 else {
            throw Self.posixFailure("Could not protect the journal lock")
        }

        guard flock(descriptor, LOCK_EX) == 0 else {
            throw Self.posixFailure("Could not lock the send journal")
        }
        defer { flock(descriptor, LOCK_UN) }

        return try operation()
    }

    private func ensureDirectory() throws {
        do {
            try fileManager.createDirectory(
                at: directoryURL,
                withIntermediateDirectories: true
            )
            guard chmod(directoryURL.path, S_IRWXU) == 0 else {
                throw Self.posixFailure("Could not protect the send journal directory")
            }
        } catch let error as JournalHostError {
            throw error
        } catch {
            throw Self.failure(.unavailable, String(describing: error))
        }
    }

    private func recordURL(for key: JournalKey) -> URL {
        let identity = "\(key.recordId)\u{0}\(key.slot)"
        let digest = SHA256.hash(data: Data(identity.utf8))
            .map { String(format: "%02x", $0) }
            .joined()
        return directoryURL
            .appendingPathComponent(digest, isDirectory: false)
            .appendingPathExtension("plist")
    }

    private nonisolated static func defaultDirectoryURL(
        fileManager: FileManager
    ) -> URL {
        let base = fileManager.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? URL(
            fileURLWithPath: NSHomeDirectory(),
            isDirectory: true
        )
        .appendingPathComponent("Library", isDirectory: true)
        .appendingPathComponent("Application Support", isDirectory: true)
        let bundleComponent = Bundle.main.bundleIdentifier ?? "TON-Wallet"
        return base
            .appendingPathComponent(bundleComponent, isDirectory: true)
            .appendingPathComponent("WalletEngine", isDirectory: true)
            .appendingPathComponent("SendJournal", isDirectory: true)
    }

    private nonisolated static func failure(
        _ kind: JournalHostErrorKind,
        _ message: String
    ) -> JournalHostError {
        .Failed(kind: kind, diagnostic: sanitized(message))
    }

    private nonisolated static func posixFailure(
        _ context: String,
        code: Int32 = errno
    ) -> JournalHostError {
        let detail = String(cString: strerror(code))
        return failure(.unavailable, "\(context): \(detail)")
    }
}

/// Aggregate implementation exposed to Rust through one UniFFI callback
/// object. The individual capabilities stay independently replaceable.
actor AppleWalletPlatformHost: WalletPlatformHost {
    private let clock: AppleWalletClock
    private let secrets: AppleWalletProtectedSecretStore
    private let journal: AppleWalletJournalStore

    init(
        clock: AppleWalletClock = AppleWalletClock(),
        secrets: AppleWalletProtectedSecretStore = AppleWalletProtectedSecretStore(),
        journal: AppleWalletJournalStore = .shared
    ) {
        self.clock = clock
        self.secrets = secrets
        self.journal = journal
    }

    func now() async -> UInt64 {
        await clock.now()
    }

    func readProtectedSecret(
        request: ProtectedSecretRead
    ) async throws -> Data {
        try await secrets.read(request)
    }

    func storeProtectedSecret(
        request: ProtectedSecretStore
    ) async throws {
        try await secrets.store(request)
    }

    func deleteProtectedSecret(
        secretRef: ProtectedSecretRef
    ) async throws {
        try await secrets.delete(secretRef)
    }

    func loadJournal(
        key: JournalKey
    ) async throws -> JournalRecord? {
        try await journal.load(key)
    }

    func compareExchangeJournal(
        mutation: JournalCompareExchange
    ) async throws -> JournalCompareExchangeResult {
        try await journal.compareExchange(mutation)
    }
}

private nonisolated func sanitized(_ message: String) -> String {
    String(
        message.unicodeScalars
            .map {
                CharacterSet.controlCharacters.contains($0) ? " " : String($0)
            }
            .joined()
            .prefix(256)
    )
}
