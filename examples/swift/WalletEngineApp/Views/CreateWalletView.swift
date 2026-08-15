import SwiftUI
import WalletEngineFFI

struct CreateWalletView: View {
    @Environment(\.dismiss) private var dismiss

    let lifecycle: WalletLifecycleModel
    let onUseWallet: @MainActor (WalletDescriptor, String) async throws -> Void

    @State private var pendingDescriptor: WalletDescriptor?
    @State private var walletName = "My Wallet"
    @State private var hasSavedRecoveryPhrase = false
    @State private var operation = Operation.idle
    @State private var errorMessage: String?
    @State private var isShowingError = false
    @State private var isPresentationActive = true
    @State private var didFinish = false
#if os(iOS)
    @State private var selectedDetent: PresentationDetent = .medium
#endif

    init(
        lifecycle: WalletLifecycleModel,
        onUseWallet: @escaping @MainActor (WalletDescriptor, String) async throws -> Void
    ) {
        self.lifecycle = lifecycle
        self.onUseWallet = onUseWallet
    }

    var body: some View {
        NavigationStack {
            content
                .navigationTitle(pendingDescriptor == nil ? "Create wallet" : "Recovery phrase")
#if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
#endif
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel", action: beginCancellation)
                            .disabled(isBusy)
                            .keyboardShortcut(.cancelAction)
                    }

                    ToolbarItem(placement: .confirmationAction) {
                        confirmationAction
                    }
                }
        }
#if os(iOS)
        .presentationDetents([.medium, .large], selection: $selectedDetent)
        .presentationDragIndicator(.visible)
#else
        .desktopSheetSize(width: 620, minHeight: 520)
#endif
        .interactiveDismissDisabled(isBusy || pendingDescriptor != nil)
        .alert("Could not complete the action", isPresented: $isShowingError) {
            Button("OK", role: .cancel) {}
        } message: {
            if let errorMessage {
                Text(errorMessage)
            }
        }
        .onChange(of: errorMessage) { _, newValue in
            isShowingError = newValue != nil
        }
        .onAppear {
            isPresentationActive = true
        }
        .onDisappear(perform: presentationDidDisappear)
    }

    @ViewBuilder
    private var content: some View {
        if let pendingDescriptor {
            RecoveryPhraseView(
                descriptor: pendingDescriptor,
                words: recoveryWords,
                hasSavedRecoveryPhrase: $hasSavedRecoveryPhrase,
                errorMessage: errorMessage
            )
        } else {
            CreateWalletForm(
                walletName: $walletName,
                errorMessage: errorMessage,
                onSubmit: beginGeneration
            )
        }
    }

    @ViewBuilder
    private var confirmationAction: some View {
        if isBusy {
            ProgressView()
                .controlSize(.small)
                .accessibilityLabel(operation.accessibilityLabel)
        } else if pendingDescriptor != nil {
            Button("Use wallet", action: beginCompletion)
                .disabled(!canUseWallet)
                .keyboardShortcut(.defaultAction)
        } else {
            Button("Generate", action: beginGeneration)
                .disabled(normalizedWalletName.isEmpty)
                .keyboardShortcut(.defaultAction)
        }
    }

    private var normalizedWalletName: String {
        walletName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var recoveryWords: [String] {
        lifecycle.recoveryPhrase?.phrase
            .split(separator: " ")
            .map(String.init) ?? []
    }

    private var isBusy: Bool {
        operation != .idle || lifecycle.isWorking
    }

    private var canUseWallet: Bool {
        hasSavedRecoveryPhrase && !normalizedWalletName.isEmpty
    }

    private func beginGeneration() {
        guard operation == .idle, pendingDescriptor == nil, !normalizedWalletName.isEmpty else {
            return
        }

        operation = .creating
        errorMessage = nil
        Task { await generateWallet() }
    }

    private func generateWallet() async {
        defer { operation = .idle }

        do {
            let descriptor = try await lifecycle.createWallet(network: .testnet)

            guard isPresentationActive else {
                try? await lifecycle.deleteWallet(descriptor)
                return
            }

            pendingDescriptor = descriptor
            guard recoveryWords.count == 24 else {
                await removeInvalidWallet(descriptor)
                return
            }

            hasSavedRecoveryPhrase = false
#if os(iOS)
            selectedDetent = .large
#endif
        } catch {
            guard isPresentationActive else { return }
            errorMessage = displayMessage(
                for: error,
                fallback: "The wallet could not be created. Please try again."
            )
        }
    }

    private func removeInvalidWallet(_ descriptor: WalletDescriptor) async {
        do {
            try await lifecycle.deleteWallet(descriptor)
            pendingDescriptor = nil
            errorMessage = "The wallet could not be created safely. Please try again."
        } catch {
            errorMessage = "The recovery phrase was unavailable, and the protected wallet could not be removed. Choose Cancel to try removing it again."
        }
    }

    private func beginCompletion() {
        guard operation == .idle, canUseWallet, let descriptor = pendingDescriptor else {
            return
        }

        operation = .saving
        errorMessage = nil
        Task { await completeWallet(descriptor) }
    }

    private func completeWallet(_ descriptor: WalletDescriptor) async {
        do {
            try await onUseWallet(descriptor, normalizedWalletName)
            didFinish = true
            pendingDescriptor = nil
            lifecycle.discardRecoveryPhrase()
            operation = .idle
            dismiss()
        } catch {
            let persistenceError = error
            await removeWalletAfterPersistenceFailure(descriptor, persistenceError: persistenceError)
            operation = .idle
        }
    }

    private func removeWalletAfterPersistenceFailure(
        _ descriptor: WalletDescriptor,
        persistenceError: Error
    ) async {
        do {
            try await lifecycle.deleteWallet(descriptor)
            pendingDescriptor = nil
            hasSavedRecoveryPhrase = false
            let reason = displayMessage(
                for: persistenceError,
                fallback: "Please try again."
            )
            errorMessage = "The wallet could not be saved, so its protected data was removed. \(reason)"
        } catch {
            errorMessage = "The wallet could not be saved, and its protected data could not be removed. Choose Cancel to try removing it again."
        }
    }

    private func beginCancellation() {
        guard operation == .idle else { return }

        guard let descriptor = pendingDescriptor else {
            lifecycle.discardRecoveryPhrase()
            dismiss()
            return
        }

        operation = .deleting
        errorMessage = nil
        Task { await cancelAndDeleteWallet(descriptor) }
    }

    private func cancelAndDeleteWallet(_ descriptor: WalletDescriptor) async {
        do {
            try await lifecycle.deleteWallet(descriptor)
            pendingDescriptor = nil
            operation = .idle
            dismiss()
        } catch {
            operation = .idle
            errorMessage = displayMessage(
                for: error,
                fallback: "The protected wallet could not be removed. Please choose Cancel to try again."
            )
        }
    }

    private func presentationDidDisappear() {
        isPresentationActive = false

        guard !didFinish else { return }
        guard let descriptor = pendingDescriptor else {
            lifecycle.discardRecoveryPhrase()
            return
        }

        // Interactive dismissal is disabled while a descriptor is pending.
        // This is a final safeguard for a parent-driven dismissal.
        guard operation == .idle else { return }
        lifecycle.discardRecoveryPhrase()
        Task {
            try? await lifecycle.deleteWallet(descriptor)
        }
    }

    private func displayMessage(for error: Error, fallback: String) -> String {
        guard let localizedError = error as? LocalizedError,
              let description = localizedError.errorDescription,
              !description.isEmpty else {
            return fallback
        }
        return description
    }
}

private extension CreateWalletView {
    enum Operation: Equatable {
        case idle
        case creating
        case saving
        case deleting

        var accessibilityLabel: String {
            switch self {
            case .idle:
                "Working"
            case .creating:
                "Creating wallet"
            case .saving:
                "Saving wallet"
            case .deleting:
                "Removing wallet"
            }
        }
    }
}

private struct CreateWalletForm: View {
    @Binding var walletName: String
    let errorMessage: String?
    let onSubmit: () -> Void

    var body: some View {
        Form {
            Section {
                TextField("Wallet name", text: $walletName)
#if os(iOS)
                    .textInputAutocapitalization(.words)
#endif
                    .onSubmit(onSubmit)
                    .onChange(of: walletName) { _, newValue in
                        if newValue.count > 40 {
                            walletName = String(newValue.prefix(40))
                        }
                    }
            } header: {
                Text("Wallet name")
            } footer: {
                Text("You can change this name later.")
            }

            Section {
                WalletCreationIntroduction()
            }

            if let errorMessage {
                Section {
                    WalletCreationError(message: errorMessage)
                }
            }
        }
        .formStyle(.grouped)
    }
}

private struct WalletCreationIntroduction: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Image(systemName: "key.viewfinder")
                .font(.largeTitle)
                .foregroundStyle(.tint)
                .accessibilityHidden(true)

            Text("A new testnet wallet and its recovery phrase will be created securely on this \(PlatformCopy.localDeviceName).")
                .fixedSize(horizontal: false, vertical: true)

            Label(
                "Anyone with the recovery phrase can control the wallet.",
                systemImage: "exclamationmark.shield.fill"
            )
            .font(.callout.weight(.medium))
            .foregroundStyle(.orange)

            Label(
                "The phrase is protected by Keychain and device authentication.",
                systemImage: "lock.shield.fill"
            )
            .font(.callout)
            .foregroundStyle(.green)
        }
        .padding(.vertical, 8)
    }
}

private struct RecoveryPhraseView: View {
    let descriptor: WalletDescriptor
    let words: [String]
    @Binding var hasSavedRecoveryPhrase: Bool
    let errorMessage: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                Label(compactAddress, systemImage: "wallet.pass.fill")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .accessibilityLabel("Wallet address \(descriptor.address)")

                if words.count == 24 {
                    RecoveryWordsGrid(words: words)
                        .privacySensitive()
                } else {
                    ContentUnavailableView(
                        "Recovery phrase unavailable",
                        systemImage: "lock.trianglebadge.exclamationmark",
                        description: Text("Cancel this wallet and create a new one.")
                    )
                }

                Toggle(
                    "I saved all 24 words in a safe place",
                    isOn: $hasSavedRecoveryPhrase
                )
                .platformConfirmationToggleStyle()
                .disabled(words.count != 24)

                if let errorMessage {
                    WalletCreationError(message: errorMessage)
                }

                Text("After you continue, the recovery phrase is removed from this screen. You can reveal it later with device authentication.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(20)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var compactAddress: String {
        "\(descriptor.address.prefix(8))…\(descriptor.address.suffix(6))"
    }
}

private struct RecoveryWordsGrid: View {
    private let numberedWords: [NumberedRecoveryWord]

    init(words: [String]) {
        numberedWords = words.enumerated().map { index, word in
            NumberedRecoveryWord(number: index + 1, word: word)
        }
    }

    var body: some View {
        LazyVGrid(
            columns: [GridItem(.adaptive(minimum: 120), spacing: 8)],
            spacing: 8
        ) {
            ForEach(numberedWords) { item in
                HStack(spacing: 8) {
                    Text(item.number, format: .number)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .frame(minWidth: 22, alignment: .trailing)

                    Text(item.word)
                        .font(.callout)
                        .lineLimit(1)
                        .minimumScaleFactor(0.8)

                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 8)
                .background(.background.secondary, in: .rect(cornerRadius: 8))
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("Word \(item.number), \(item.word)")
            }
        }
    }
}

private struct NumberedRecoveryWord: Identifiable {
    let number: Int
    let word: String

    var id: Int { number }
}

private struct WalletCreationError: View {
    let message: String

    var body: some View {
        Label(message, systemImage: "exclamationmark.triangle.fill")
            .font(.callout)
            .foregroundStyle(.red)
            .fixedSize(horizontal: false, vertical: true)
            .textSelection(.enabled)
    }
}
