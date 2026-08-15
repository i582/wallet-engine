import CoreImage.CIFilterBuiltins
import SwiftUI
import WalletEngineFFI

struct ContentView: View {
    @AppStorage("isBalanceVisible") private var isBalanceVisible = true
    @AppStorage("appAppearance") private var appAppearance = AppAppearance.system.rawValue

    private var preferredColorScheme: ColorScheme? {
        AppAppearance(rawValue: appAppearance)?.colorScheme
    }

    var body: some View {
#if os(macOS)
        NavigationStack {
            WalletDashboard(isBalanceVisible: $isBalanceVisible)
        }
        .desktopMinimumSize(width: 430, height: 520)
        .toolbarBackground(.hidden, for: .windowToolbar)
        .preferredColorScheme(preferredColorScheme)
#else
        NavigationStack {
            WalletDashboard(isBalanceVisible: $isBalanceVisible)
            .navigationDestination(for: WalletSection.self) { section in
                if section == .settings {
                    SettingsView()
                }
            }
        }
        .preferredColorScheme(preferredColorScheme)
#endif
    }
}

private enum WalletSection: Hashable {
    case settings
}

private enum AppAppearance: String, CaseIterable, Identifiable {
    case system = "System"
    case light = "Light"
    case dark = "Dark"

    var id: Self { self }

    var colorScheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }
}

struct SettingsView: View {
    @AppStorage("isBalanceVisible") private var isBalanceVisible = true
    @AppStorage("appAppearance") private var appAppearance = AppAppearance.system.rawValue

    private var preferredColorScheme: ColorScheme? {
        AppAppearance(rawValue: appAppearance)?.colorScheme
    }

    var body: some View {
        settingsForm
#if os(macOS)
            .formStyle(.grouped)
            .scenePadding()
            .frame(width: 520, height: 420, alignment: .top)
#else
            .formStyle(.grouped)
            .frame(maxWidth: 680)
            .frame(maxWidth: .infinity, alignment: .top)
            .navigationTitle("Settings")
            .platformCompactNavigationTitle()
#endif
            .preferredColorScheme(preferredColorScheme)
    }

    private var settingsForm: some View {
        Form {
            Section("Appearance") {
                Picker("Appearance", selection: $appAppearance) {
                    ForEach(AppAppearance.allCases) { appearance in
                        Text(appearance.rawValue)
                            .tag(appearance.rawValue)
                    }
                }
                .pickerStyle(.segmented)

                Toggle("Show wallet balance", isOn: $isBalanceVisible)
            }

            Section("Network") {
                LabeledContent("Network", value: "Testnet")
                LabeledContent("Wallet version", value: "V5R1")
            }

            Section("Wallet data") {
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: PlatformCopy.localStorageIcon)
                        .foregroundStyle(.secondary)
                        .frame(width: 24)

                    VStack(alignment: .leading, spacing: 3) {
                        Text("Stored on this \(PlatformCopy.localDeviceName)")
                        Text("Recovery words are protected by Keychain and device authentication.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .padding(.vertical, 2)
            }
        }
    }
}

private struct WalletDashboard: View {
    @Binding var isBalanceVisible: Bool

    @State private var environment: AppleWalletEnvironment
    @State private var lifecycle: WalletLifecycleModel
    @State private var wallets: [StoredWallet] = []
    @State private var selectedWalletAddress: String?
    @State private var session: WalletSession?
    @State private var sessionError: String?
    @State private var isConfirmingWalletDeletion = false
    @State private var presentedSheet: WalletSheet?
    @State private var didRestoreWallets = false
    @State private var persistenceError: String?
    @State private var activationGeneration: UInt64 = 0

    init(isBalanceVisible: Binding<Bool>) {
        _isBalanceVisible = isBalanceVisible
        let environment = AppleWalletEnvironment()
        _environment = State(initialValue: environment)
        _lifecycle = State(initialValue: WalletLifecycleModel(environment: environment))
    }

    private var activeWallet: StoredWallet? {
        guard let selectedWalletAddress else { return nil }
        return wallets.first { $0.address == selectedWalletAddress }
    }

    private var walletSnapshot: WalletSnapshot? {
        guard let activeWallet,
              let snapshot = session?.snapshot,
              snapshot.recordId == activeWallet.recordId else {
            return nil
        }
        return snapshot
    }

    private var account: WalletAccountSnapshot? {
        walletSnapshot?.viewAccount
    }

    private var transactions: [WalletTransaction] {
        walletSnapshot?.viewTransactions ?? []
    }

    private var isLoadingHistory: Bool {
        guard !hasTerminalWalletFailure, activeWallet != nil else { return false }
        return walletSnapshot == nil || walletSnapshot?.activityResource.phase == .loading
    }

    private var isRefreshing: Bool {
        guard !hasTerminalWalletFailure else { return false }
        guard let snapshot = walletSnapshot else {
            return activeWallet != nil
        }
        return snapshot.accountResource.phase == .loading
            || snapshot.activityResource.phase == .loading
    }

    private var isLoadingMoreHistory: Bool {
        !hasTerminalWalletFailure
            && walletSnapshot?.activityPaginationResource.phase == .loading
    }

    private var canLoadMoreHistory: Bool {
        !hasTerminalWalletFailure
            && walletSnapshot?.activityHasMore == true
            && walletSnapshot?.activityResource.phase != .loading
    }

    private var hasTerminalWalletFailure: Bool {
        sessionError != nil || session?.diagnostic != nil
    }

    private var historyError: String? {
        hasTerminalWalletFailure || walletSnapshot?.activityResource.phase == .failed
            ? "Could not load activity"
            : nil
    }

    private var hasRefreshNotice: Bool {
        hasTerminalWalletFailure
            || walletSnapshot?.accountResource.phase == .failed
            || walletSnapshot?.activityResource.phase == .failed
    }

    private var loadMoreHistoryError: String? {
        walletSnapshot?.activityPaginationResource.phase == .failed
            ? "Could not load more activity"
            : nil
    }

    private var horizontalPadding: CGFloat {
#if os(iOS)
        16
#else
        32
#endif
    }

    var body: some View {
        ScrollView {
            if let activeWallet {
                VStack(alignment: .leading, spacing: 28) {
                    VStack(spacing: 14) {
                        BalancePanel(
                            isBalanceVisible: $isBalanceVisible,
                            account: account,
                            isLoading: isRefreshing,
                            onRefresh: refreshAccount
                        )
                        WalletActions(
                            onSend: { presentedSheet = .send },
                            onReceive: { presentedSheet = .receive }
                        )
                        if hasRefreshNotice {
                            WalletDataNotice(onRetry: refreshAccount)
                        }
                    }
                    if let persistenceError {
                        Label(persistenceError, systemImage: "externaldrive.badge.exclamationmark")
                            .font(.callout)
                            .foregroundStyle(.red)
                            .textSelection(.enabled)
                    }
                    RecentActivity(
                        transactions: transactions,
                        isLoading: isLoadingHistory,
                        errorMessage: historyError,
                        canLoadMore: canLoadMoreHistory,
                        isLoadingMore: isLoadingMoreHistory,
                        loadMoreError: loadMoreHistoryError,
                        onRefresh: refreshAccount,
                        onLoadMore: loadMoreHistory
                    )
                }
                .frame(maxWidth: 980, alignment: .leading)
                .padding(.horizontal, horizontalPadding)
                .padding(.top, 8)
                .padding(.bottom, 32)
            } else {
                EmptyWalletState {
                    presentedSheet = .create
                }
                .containerRelativeFrame(.vertical)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.platformWindowBackground)
        .refreshable {
            await refreshWalletData()
        }
#if os(macOS)
        .navigationTitle("Wallet")
#else
        .navigationTitle(activeWallet?.name ?? "Wallet")
#endif
        .platformCompactNavigationTitle()
        .toolbar {
#if os(iOS)
            ToolbarItem(placement: .topBarLeading) {
                NavigationLink(value: WalletSection.settings) {
                    Label("Settings", systemImage: "gearshape")
                }
                .accessibilityLabel("Settings")
            }

            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    presentedSheet = .create
                } label: {
                    Label(activeWallet == nil ? "Create wallet" : "New wallet", systemImage: "plus.circle")
                }
                .help("Create a new testnet wallet")
            }
#else
            ToolbarItem(placement: .principal) {
                if let activeWallet {
                    Menu {
                        Section("Wallets") {
                            ForEach(wallets, id: \.address) { wallet in
                                Button {
                                    selectWallet(wallet.address)
                                } label: {
                                    if wallet.address == activeWallet.address {
                                        Label(wallet.name, systemImage: "checkmark")
                                    } else {
                                        Text(wallet.name)
                                    }
                                }
                            }
                        }

                        Divider()

                        Button {
                            presentedSheet = .rename
                        } label: {
                            Label("Rename wallet", systemImage: "pencil")
                        }
                        Button(role: .destructive) {
                            isConfirmingWalletDeletion = true
                        } label: {
                            Label("Delete wallet", systemImage: "trash")
                        }
                    } label: {
                        Text(activeWallet.name)
                    }
                    .fixedSize(horizontal: true, vertical: false)
                    .help("Choose wallet")
                }
            }

            ToolbarItem(placement: .primaryAction) {
                Button {
                    presentedSheet = .create
                } label: {
                    Label(activeWallet == nil ? "Create wallet" : "New wallet", systemImage: "plus.circle")
                }
                .help("Create a new testnet wallet")
            }
#endif
        }
        .toolbarTitleMenu {
            if let activeWallet {
                Section("Wallets") {
                    ForEach(wallets, id: \.address) { wallet in
                        Button {
                            selectWallet(wallet.address)
                        } label: {
                            if wallet.address == activeWallet.address {
                                Label(wallet.name, systemImage: "checkmark")
                            } else {
                                Text(wallet.name)
                            }
                        }
                    }
                }

                Section {
                    Button {
                        presentedSheet = .rename
                    } label: {
                        Label("Rename wallet", systemImage: "pencil")
                    }
                    Button(role: .destructive) {
                        isConfirmingWalletDeletion = true
                    } label: {
                        Label("Delete wallet", systemImage: "trash")
                    }
                }
            }
        }
        .sheet(item: $presentedSheet) { sheet in
            switch sheet {
            case .create:
                CreateWalletView(
                    lifecycle: lifecycle,
                    onUseWallet: handleCreatedWallet
                )
            case .rename:
                if let activeWallet {
                    RenameWalletView(currentName: activeWallet.name) { newName in
                        renameWallet(activeWallet, to: newName)
                    }
                }
            case .receive:
                if let activeWallet {
                    ReceiveWalletView(wallet: activeWallet)
                }
            case .send:
                if let activeWallet, let session {
                    SendWalletView(
                        wallet: activeWallet,
                        account: account,
                        session: session
                    ) {
                        refreshAccount()
                    }
                }
            }
        }
        .confirmationDialog(
            "Delete \(activeWallet?.name ?? "wallet")?",
            isPresented: $isConfirmingWalletDeletion,
            titleVisibility: .visible
        ) {
            Button("Delete wallet", role: .destructive) {
                Task {
                    await deleteActiveWallet()
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Its recovery phrase and saved data will be removed from this \(PlatformCopy.localDeviceName). This cannot be undone.")
        }
        .task {
            restoreWallets()
        }
        .task(id: activeWallet?.recordId) {
            await activateWalletData()
        }
        .onDisappear(perform: stopWalletData)
    }

    private func restoreWallets() {
        guard !didRestoreWallets else { return }
        didRestoreWallets = true
        do {
            let archive = try WalletStore.load()
            wallets = archive.wallets
            selectedWalletAddress = archive.selectedAddress.flatMap { selected in
                archive.wallets.contains { $0.address == selected } ? selected : nil
            } ?? archive.wallets.first?.address
        } catch {
            persistenceError = "Could not load wallets from disk: \(error.localizedDescription)"
        }
    }

    private func handleCreatedWallet(
        _ descriptor: WalletDescriptor,
        _ name: String
    ) async throws {
        let storedWallet = StoredWallet(descriptor: descriptor, name: name)
        var updatedWallets = wallets
        updatedWallets.append(storedWallet)
        try WalletStore.save(wallets: updatedWallets, selectedAddress: storedWallet.address)

        wallets = updatedWallets
        selectedWalletAddress = storedWallet.address
        sessionError = nil
        persistenceError = nil
    }

    private func renameWallet(_ wallet: StoredWallet, to newName: String) {
        guard let index = wallets.firstIndex(where: { $0.address == wallet.address }) else {
            return
        }
        wallets[index].name = newName
        persistWallets()
    }

    private func persistWallets() {
        do {
            try WalletStore.save(wallets: wallets, selectedAddress: selectedWalletAddress)
            persistenceError = nil
        } catch {
            persistenceError = "Could not save wallets to disk: \(error.localizedDescription)"
        }
    }

    private func selectWallet(_ address: String) {
        guard address != selectedWalletAddress else { return }

        selectedWalletAddress = address
        sessionError = nil
        do {
            try WalletStore.save(wallets: wallets, selectedAddress: address)
            persistenceError = nil
        } catch {
            persistenceError = "Could not save the selected wallet: \(error.localizedDescription)"
        }
    }

    private func deleteActiveWallet() async {
        guard let selectedWalletAddress,
              let deletedIndex = wallets.firstIndex(where: { $0.address == selectedWalletAddress }) else {
            return
        }

        let wallet = wallets[deletedIndex]
        guard let descriptor = wallet.descriptor else {
            persistenceError = "Could not read the wallet metadata."
            return
        }

        var remainingWallets = wallets
        remainingWallets.remove(at: deletedIndex)
        let nextAddress = remainingWallets.isEmpty
            ? nil
            : remainingWallets[min(deletedIndex, remainingWallets.count - 1)].address

        do {
            try await lifecycle.deleteWallet(descriptor)
            try WalletStore.save(wallets: remainingWallets, selectedAddress: nextAddress)
            wallets = remainingWallets
            self.selectedWalletAddress = nextAddress
            sessionError = nil
            persistenceError = nil
        } catch {
            persistenceError = "Could not delete the wallet: \(error.localizedDescription)"
        }
    }

    private func refreshAccount() {
        Task {
            await refreshWalletData()
        }
    }

    private func refreshWalletData() async {
        guard let wallet = activeWallet else { return }

        if let session,
           session.snapshot.recordId == wallet.recordId {
            await session.refresh()
        } else {
            await activateWalletData(expectedWalletAddress: wallet.address)
        }
    }

    private func loadMoreHistory() {
        guard canLoadMoreHistory, !isLoadingMoreHistory else { return }
        Task {
            await session?.loadMoreActivity()
        }
    }

    private func activateWalletData(expectedWalletAddress: String? = nil) async {
        guard expectedWalletAddress == nil
                || activeWallet?.address == expectedWalletAddress else {
            return
        }
        activationGeneration &+= 1
        let generation = activationGeneration
        guard let wallet = activeWallet else {
            let previous = session
            session = nil
            sessionError = nil
            await previous?.shutdown()
            return
        }

        sessionError = nil
        do {
            let installedWallet = wallet

            let replacement = try environment.makeClient(wallet: installedWallet)
            // Native construction is synchronous; explicitly complete durable
            // send recovery before the replacement client becomes observable.
            _ = try await replacement.start()
            guard !Task.isCancelled,
                  generation == activationGeneration,
                  activeWallet?.recordId == installedWallet.recordId else {
                try? await replacement.shutdown()
                return
            }

            let installedSession: WalletSession
            if let session {
                try await session.replaceClient(replacement)
                installedSession = session
            } else {
                installedSession = try WalletSession(client: replacement)
                session = installedSession
            }

            guard !Task.isCancelled,
                  generation == activationGeneration,
                  activeWallet?.recordId == installedWallet.recordId else {
                await installedSession.shutdown()
                return
            }

            await installedSession.refresh()
            sessionError = nil
        } catch is CancellationError {
            return
        } catch {
            guard generation == activationGeneration,
                  activeWallet?.address == wallet.address else { return }
            sessionError = "Couldn’t start wallet data: \(error.localizedDescription)"
        }
    }

    private func stopWalletData() {
        activationGeneration &+= 1
        let previous = session
        session = nil
        Task {
            await previous?.shutdown()
        }
    }
}

private enum WalletSheet: String, Identifiable {
    case create
    case rename
    case receive
    case send

    var id: String { rawValue }
}

private struct EmptyWalletState: View {
    let onCreate: () -> Void

    var body: some View {
        ContentUnavailableView {
            Label("No wallet yet", systemImage: "wallet.bifold")
        } description: {
            Text("Create a wallet to send, receive, and track GRAM.")
        } actions: {
            Button("Create testnet wallet", action: onCreate)
                .buttonStyle(.borderedProminent)
        }
    }
}

private struct RenameWalletView: View {
    @Environment(\.dismiss) private var dismiss

    let onSave: (String) -> Void
    @State private var walletName: String
    @FocusState private var isNameFocused: Bool

    init(currentName: String, onSave: @escaping (String) -> Void) {
        self.onSave = onSave
        _walletName = State(initialValue: currentName)
    }

    private var normalizedWalletName: String {
        walletName.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
#if os(iOS)
        NavigationStack {
            Form {
                Section {
                    TextField("Wallet name", text: $walletName)
                        .focused($isNameFocused)
                        .onChange(of: walletName) { _, newValue in
                            limitWalletName(newValue)
                        }
                } header: {
                    Text("Wallet name")
                } footer: {
                    Text("The name is stored locally and does not affect the wallet address.")
                }
            }
            .navigationTitle("Rename wallet")
            .navigationBarTitleDisplayMode(.inline)
            .defaultFocus($isNameFocused, true)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save", action: save)
                        .disabled(normalizedWalletName.isEmpty)
                }
            }
        }
        .platformResizableModalPresentation()
#else
        VStack(alignment: .leading, spacing: 20) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Rename wallet")
                    .font(.title2.weight(.semibold))
                Text("The name is stored locally and does not affect the wallet address.")
                    .foregroundStyle(.secondary)
            }

            VStack(alignment: .leading, spacing: 7) {
                Text("Wallet name")
                    .font(.callout.weight(.medium))
                TextField("Wallet name", text: $walletName)
                    .textFieldStyle(.roundedBorder)
                    .onChange(of: walletName) { _, newValue in
                        limitWalletName(newValue)
                    }
            }

            HStack {
                Button("Cancel") { dismiss() }
                Spacer()
                Button("Save", action: save)
                    .buttonStyle(.borderedProminent)
                    .disabled(normalizedWalletName.isEmpty)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(.horizontal, 24)
        .padding(.top, 12)
        .padding(.bottom, 20)
        .desktopSheetSize(width: 420)
#endif
    }

    private func limitWalletName(_ newValue: String) {
        if newValue.count > 40 {
            walletName = String(newValue.prefix(40))
        }
    }

    private func save() {
        onSave(normalizedWalletName)
        dismiss()
    }
}

private struct BalancePanel: View {
    @Binding var isBalanceVisible: Bool
    let account: WalletAccountSnapshot?
    let isLoading: Bool
    let onRefresh: () -> Void
    @ScaledMetric(relativeTo: .largeTitle) private var balanceFontSize = 42.0

    private var displayedBalance: String {
        guard isBalanceVisible else { return "••••••" }
        if let account { return account.balanceGrams }
        return isLoading ? "…" : "—"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("BALANCE")
                    .font(.caption.weight(.semibold))
                    .tracking(0.8)
                    .foregroundStyle(.white.opacity(0.7))

                Spacer()

                HStack(spacing: 0) {
                    Button(action: onRefresh) {
                        if isLoading {
                            ProgressView()
                                .controlSize(.small)
                        } else {
                            Image(systemName: "arrow.clockwise")
                        }
                    }
                    .platformCompactIconButtonFrame()
                    .buttonStyle(.plain)
                    .disabled(isLoading)
                    .accessibilityLabel(isLoading ? "Refreshing balance" : "Refresh balance")
                    .desktopHelp("Refresh account")

                    Button {
                        isBalanceVisible.toggle()
                    } label: {
                        Image(systemName: isBalanceVisible ? "eye" : "eye.slash")
                    }
                    .platformCompactIconButtonFrame()
                    .buttonStyle(.plain)
                    .accessibilityLabel(isBalanceVisible ? "Hide balance" : "Show balance")
                    .desktopHelp(isBalanceVisible ? "Hide balance" : "Show balance")
                }
                .foregroundStyle(.white.opacity(0.8))
            }

            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text(displayedBalance)
                    .font(.system(size: balanceFontSize, weight: .semibold, design: .rounded))
                    .foregroundStyle(.white)
                    .monospacedDigit()
                    .lineLimit(1)
                    .minimumScaleFactor(0.58)
                    .layoutPriority(1)
                Text("GRAM")
                    .font(.title3.weight(.medium))
                    .foregroundStyle(.white.opacity(0.72))
                    .fixedSize()
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Balance")
            .accessibilityValue(isBalanceVisible ? "\(displayedBalance) GRAM" : "Hidden")

        }
        .padding(.horizontal, 24)
        .padding(.top, 8)
        .padding(.bottom, 20)
        .background(
            LinearGradient(
                colors: [Color(red: 0.03, green: 0.38, blue: 0.68), Color(red: 0.02, green: 0.22, blue: 0.46)],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            ),
            in: RoundedRectangle(cornerRadius: 20, style: .continuous)
        )
        .shadow(color: .black.opacity(0.14), radius: 18, y: 8)
    }
}

private struct WalletDataNotice: View {
    let onRetry: () -> Void

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 10) {
                Label("Couldn’t refresh wallet data", systemImage: "exclamationmark.circle")
                    .font(.callout)
                Spacer()
                Button("Try again", action: onRetry)
                    .platformLinkButtonStyle()
            }

            VStack(alignment: .leading, spacing: 8) {
                Label("Couldn’t refresh wallet data", systemImage: "exclamationmark.circle")
                    .font(.callout)
                Button("Try again", action: onRetry)
                    .platformLinkButtonStyle()
            }
        }
        .foregroundStyle(.secondary)
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 12))
    }
}

private struct WalletActions: View {
    let onSend: () -> Void
    let onReceive: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Button(action: onSend) {
                Label("Send", systemImage: "arrow.up.right")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)

            Button(action: onReceive) {
                Label("Receive", systemImage: "arrow.down.left")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.bordered)
        }
        .controlSize(.large)
    }
}

private struct RecentActivity: View {
    let transactions: [WalletTransaction]
    let isLoading: Bool
    let errorMessage: String?
    let canLoadMore: Bool
    let isLoadingMore: Bool
    let loadMoreError: String?
    let onRefresh: () -> Void
    let onLoadMore: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Recent activity")
                    .font(.title3.weight(.semibold))
                Spacer()
                if isLoading, !transactions.isEmpty {
                    ProgressView()
                        .controlSize(.small)
                }
            }

            if transactions.isEmpty, isLoading {
                HStack {
                    Spacer()
                    ProgressView()
                        .controlSize(.small)
                    Spacer()
                }
                .frame(height: 96)
            } else if transactions.isEmpty, errorMessage != nil {
                ActivityPlaceholder(
                    icon: "arrow.clockwise.circle",
                    title: "Activity is unavailable",
                    actionTitle: "Try again",
                    action: onRefresh
                )
            } else if transactions.isEmpty {
                ActivityPlaceholder(
                    icon: "clock",
                    title: "No transactions yet",
                    actionTitle: nil,
                    action: nil
                )
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(transactions.enumerated()), id: \.element.id) { index, transaction in
                        VStack(spacing: 0) {
                            NavigationLink {
                                TransactionDetailView(transaction: transaction)
                            } label: {
                                ActivityRow(transaction: transaction)
                            }
                            .buttonStyle(.plain)
                            if index < transactions.count - 1 {
                                Divider()
                                    .padding(.leading, 54)
                            }
                        }
                    }

                    if canLoadMore {
                        Divider()
                            .padding(.leading, 54)
                        Group {
                            if isLoadingMore {
                                ProgressView()
                                    .controlSize(.small)
                            } else if loadMoreError != nil {
                                Button("Try again", action: onLoadMore)
                                    .buttonStyle(.borderless)
                            } else {
                                Button("Load more", action: onLoadMore)
                                    .buttonStyle(.borderless)
                            }
                        }
                        .frame(maxWidth: .infinity)
                        .frame(height: 44)
                    }
                }
                .padding(.horizontal, 16)
                .background(.background.secondary, in: RoundedRectangle(cornerRadius: 14))
                .overlay {
                    RoundedRectangle(cornerRadius: 14)
                        .stroke(.separator.opacity(0.45), lineWidth: 1)
                }
            }
        }
    }
}

private struct ActivityRow: View {
    let transaction: WalletTransaction

    private var endpoint: String? {
        guard let counterparty = transaction.counterparty, !counterparty.isEmpty else {
            return nil
        }
        if counterparty.count <= 18 { return counterparty }
        return "\(counterparty.prefix(8))…\(counterparty.suffix(6))"
    }

    private var date: Date {
        Date(timeIntervalSince1970: TimeInterval(transaction.timestamp))
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: transaction.isReceived ? "arrow.down.left" : "arrow.up.right")
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(transaction.isReceived ? .green : .primary)
                .frame(width: 38, height: 38)
                .background(.quaternary, in: Circle())

            VStack(alignment: .leading, spacing: 3) {
                Text(transaction.isReceived ? "Received" : "Sent")
                    .font(.body.weight(.medium))
                Text(
                    endpoint.map {
                        "\(date.formatted(date: .abbreviated, time: .shortened)) · \($0)"
                    } ?? date.formatted(date: .abbreviated, time: .shortened)
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Text("\(transaction.isReceived ? "+" : "−")\(transaction.amountGrams) GRAM")
                .font(.body.weight(.semibold))
                .monospacedDigit()
                .lineLimit(1)
            .fixedSize(horizontal: true, vertical: false)

            Image(systemName: "chevron.right")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.tertiary)
        }
        .padding(.vertical, 13)
        .contentShape(Rectangle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(transaction.isReceived ? "Received" : "Sent")
        .accessibilityValue("\(transaction.isReceived ? "+" : "minus")\(transaction.amountGrams) GRAM, \(date.formatted(date: .abbreviated, time: .shortened))")
        .accessibilityHint("Shows transaction details")
    }
}

private struct TransactionDetailView: View {
    let transaction: WalletTransaction
    @ScaledMetric(relativeTo: .largeTitle) private var amountFontSize = 38.0

    private var date: Date {
        Date(timeIntervalSince1970: TimeInterval(transaction.timestamp))
    }

    private var counterpartyLabel: String {
        transaction.isReceived ? "From" : "To"
    }

    private var counterparty: String {
        guard let counterparty = transaction.counterparty, !counterparty.isEmpty else {
            return "Unknown address"
        }
        return counterparty
    }

    private var horizontalPadding: CGFloat {
#if os(iOS)
        20
#else
        32
#endif
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 24) {
                VStack(spacing: 10) {
                    Image(systemName: transaction.isReceived ? "arrow.down.left" : "arrow.up.right")
                        .font(.system(size: 24, weight: .semibold))
                        .foregroundStyle(transaction.isReceived ? .green : .blue)
                        .frame(width: 58, height: 58)
                        .background(.quaternary, in: Circle())

                    Text(transaction.isReceived ? "Received" : "Sent")
                        .font(.title3.weight(.medium))
                        .foregroundStyle(.secondary)

                    Text("\(transaction.isReceived ? "+" : "−")\(transaction.amountGrams) GRAM")
                        .font(.system(size: amountFontSize, weight: .semibold, design: .rounded))
                        .monospacedDigit()

                    Label("Confirmed", systemImage: "checkmark.circle.fill")
                        .font(.callout.weight(.medium))
                        .foregroundStyle(.green)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(.green.opacity(0.12), in: Capsule())
                        .accessibilityLabel("Transaction confirmed on the blockchain")
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 20)

                VStack(spacing: 0) {
                    TransactionValueRow(
                        title: "Date",
                        value: date.formatted(date: .long, time: .shortened)
                    )
                    Divider()
                    TransactionValueRow(
                        title: counterpartyLabel,
                        value: counterparty,
                        monospaced: true,
                        compactAddressOnPhone: true,
                        copyable: transaction.counterparty?.isEmpty == false
                    )
                }
                .padding(.horizontal, 18)
                .background(.background.secondary, in: RoundedRectangle(cornerRadius: 14))
                .overlay {
                    RoundedRectangle(cornerRadius: 14)
                        .stroke(.separator.opacity(0.45), lineWidth: 1)
                }

                DisclosureGroup("Transaction details") {
                    VStack(spacing: 0) {
                        TransactionValueRow(
                            title: "Transaction ID",
                            value: transaction.transactionHash,
                            monospaced: true
                        )
                        Divider()
                        TransactionValueRow(
                            title: "Logical time",
                            value: transaction.logicalTime,
                            monospaced: true
                        )
                    }
                    .padding(.top, 8)
                }
                .padding(18)
                .background(.background.secondary, in: RoundedRectangle(cornerRadius: 14))
                .overlay {
                    RoundedRectangle(cornerRadius: 14)
                        .stroke(.separator.opacity(0.45), lineWidth: 1)
                }
            }
            .frame(maxWidth: 680)
            .padding(.horizontal, horizontalPadding)
            .padding(.top, 12)
            .padding(.bottom, 32)
            .frame(maxWidth: .infinity)
        }
        .background(Color.platformWindowBackground)
        .navigationTitle("Transaction")
        .platformCompactNavigationTitle()
    }
}

private struct TransactionValueRow: View {
    let title: String
    let value: String
    var monospaced = false
    var compactAddressOnPhone = false
    var copyable = false
    @State private var didCopy = false

    private var displayedValue: String {
#if os(iOS)
        guard compactAddressOnPhone, value.count > 15 else { return value }
        return "\(value.prefix(6))…\(value.suffix(6))"
#else
        return value
#endif
    }

    var body: some View {
#if os(macOS)
        HStack(alignment: .firstTextBaseline, spacing: 16) {
            Text(title)
                .foregroundStyle(.secondary)
                .frame(width: 88, alignment: .leading)

            valueContent
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .font(.body)
        .padding(.vertical, 14)
#else
        LabeledContent {
            valueContent
                .multilineTextAlignment(.trailing)
        } label: {
            Text(title)
                .foregroundStyle(.secondary)
        }
        .font(.body)
        .padding(.vertical, 14)
#endif
    }

    private var valueContent: some View {
        HStack(spacing: 8) {
            valueText
            if copyable {
                Button {
                    PlatformPasteboard.copy(value)
                    didCopy = true
                } label: {
                    Image(systemName: didCopy ? "checkmark" : "doc.on.doc")
                        .frame(width: 18, height: 18)
                        .contentTransition(.symbolEffect(.replace))
                }
                .buttonStyle(.plain)
                .foregroundStyle(didCopy ? .green : .secondary)
                .accessibilityLabel(didCopy ? "Address copied" : "Copy address")
                .desktopHelp(didCopy ? "Copied" : "Copy address")
            }
        }
        .task(id: didCopy) {
            guard didCopy else { return }
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            didCopy = false
        }
    }

    private var valueText: some View {
        Text(displayedValue)
            .fontDesign(monospaced ? .monospaced : .default)
            .lineLimit(1)
            .truncationMode(.middle)
            .textSelection(.enabled)
            .accessibilityValue(value)
            .desktopHelp(value)
    }
}

private struct ActivityPlaceholder: View {
    let icon: String
    let title: String
    let actionTitle: String?
    let action: (() -> Void)?

    var body: some View {
        VStack(spacing: 9) {
            Image(systemName: icon)
                .font(.title2)
                .foregroundStyle(.secondary)
            Text(title)
                .font(.callout)
                .foregroundStyle(.secondary)
            if let actionTitle, let action {
                Button(actionTitle, action: action)
                    .platformLinkButtonStyle()
            }
        }
        .frame(maxWidth: .infinity, minHeight: 112)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 14))
    }
}

private struct ReceiveWalletView: View {
    @Environment(\.dismiss) private var dismiss
    let wallet: StoredWallet
    @State private var didCopy = false

    var body: some View {
        Group {
#if os(iOS)
            NavigationStack {
                ScrollView {
                    receiveDetails
                        .padding(24)
                }
                .scrollBounceBehavior(.basedOnSize)
                .navigationTitle("Receive GRAM")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Close") { dismiss() }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button(action: copyAddress) {
                            Label(didCopy ? "Copied" : "Copy", systemImage: didCopy ? "checkmark" : "doc.on.doc")
                        }
                    }
                }
            }
            .platformResizableModalPresentation()
#else
            VStack(alignment: .leading, spacing: 22) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Receive GRAM")
                        .font(.title2.weight(.semibold))
                }

                receiveDetails

                HStack {
                    Button("Close") { dismiss() }
                    Spacer()
                    Button(action: copyAddress) {
                        Label(didCopy ? "Copied" : "Copy address", systemImage: didCopy ? "checkmark" : "doc.on.doc")
                    }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                }
            }
            .padding(24)
            .desktopSheetSize(width: 500)
#endif
        }
        .task(id: didCopy) {
            guard didCopy else { return }
            try? await Task.sleep(for: .seconds(1.5))
            guard !Task.isCancelled else { return }
            didCopy = false
        }
    }

    private var receiveDetails: some View {
        VStack(alignment: .leading, spacing: 22) {
            Text("Send testnet GRAM to \(wallet.name) using this address.")
                .foregroundStyle(.secondary)

            QRCodeView(value: wallet.address)
                .frame(width: 148, height: 148)
                .padding(14)
                .background(.white, in: RoundedRectangle(cornerRadius: 16))
                .frame(maxWidth: .infinity)
                .accessibilityLabel("QR code for receiving GRAM at \(wallet.address)")

            Text(wallet.address)
                .font(.body.monospaced())
                .textSelection(.enabled)
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.background.secondary, in: RoundedRectangle(cornerRadius: 12))
        }
    }

    private func copyAddress() {
        PlatformPasteboard.copy(wallet.address)
        didCopy = true
    }
}

private struct QRCodeView: View {
    let value: String
    @State private var image: PlatformImage?

    nonisolated private static func makeImage(for value: String) -> PlatformImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(value.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else { return nil }

        let scaled = output.transformed(by: CGAffineTransform(scaleX: 12, y: 12))
        let context = CIContext(options: [.useSoftwareRenderer: false])
        guard let cgImage = context.createCGImage(scaled, from: scaled.extent) else {
            return nil
        }
#if os(macOS)
        return NSImage(cgImage: cgImage, size: NSSize(width: 180, height: 180))
#else
        return UIImage(cgImage: cgImage)
#endif
    }

    private func imageView(_ image: PlatformImage) -> Image {
#if os(macOS)
        Image(nsImage: image)
#else
        Image(uiImage: image)
#endif
    }

    var body: some View {
        Group {
            if let image {
                imageView(image)
                    .resizable()
                    .interpolation(.none)
            } else {
                ProgressView()
                    .controlSize(.small)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task(id: value) {
            image = nil
            image = await Task.detached {
                Self.makeImage(for: value)
            }.value
        }
    }
}

private struct SendWalletView: View {
    @Environment(\.dismiss) private var dismiss

    let wallet: StoredWallet
    let account: WalletAccountSnapshot?
    let session: WalletSession
    let onSubmitted: () -> Void

    @State private var destination = ""
    @State private var amount = ""
    @State private var isSubmitting = false
    @State private var isConfirming = false
    @State private var errorMessage: String?
    @FocusState private var focusedField: Field?

    private enum Field: Hashable {
        case destination
        case amount
    }

    private var normalizedDestination: String {
        destination.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedAmount: String {
        amount.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        Group {
#if os(iOS)
        NavigationStack {
            Form {
                Section {
                    TextField("Wallet address", text: $destination)
                        .platformWalletAddressInput()
                        .focused($focusedField, equals: .destination)
                        .submitLabel(.next)
                        .onSubmit { focusedField = .amount }
                } header: {
                    Text("Recipient")
                }

                Section {
                    HStack {
                        TextField("0", text: $amount)
                            .monospacedDigit()
                            .platformDecimalInput()
                            .focused($focusedField, equals: .amount)
                        Text("GRAM")
                            .foregroundStyle(.secondary)
                    }
                } header: {
                    Text("Amount")
                } footer: {
                    Text("Available: \(account?.balanceGrams ?? "—") GRAM. Network fees are charged separately.")
                }

                if let errorMessage {
                    Section {
                        Label(errorMessage, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.red)
                            .textSelection(.enabled)
                    }
                }
            }
            .navigationTitle("Send GRAM")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .disabled(isSubmitting)
                }
                ToolbarItem(placement: .confirmationAction) {
                    sendButton
                }
            }
            .defaultFocus($focusedField, .destination)
        }
        .platformResizableModalPresentation()
        .interactiveDismissDisabled(isSubmitting)
#else
        VStack(alignment: .leading, spacing: 22) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Send GRAM")
                    .font(.title2.weight(.semibold))
                Text("From \(wallet.name)")
                    .foregroundStyle(.secondary)
            }

            VStack(alignment: .leading, spacing: 8) {
                Text("Recipient address")
                    .font(.callout.weight(.medium))
                TextField("Testnet wallet address", text: $destination)
                    .textFieldStyle(.roundedBorder)
                    .platformWalletAddressInput()
            }

            VStack(alignment: .leading, spacing: 8) {
                Text("Amount")
                    .font(.callout.weight(.medium))
                HStack {
                    TextField("0", text: $amount)
                        .textFieldStyle(.roundedBorder)
                        .monospacedDigit()
                        .platformDecimalInput()
                    Text("GRAM")
                        .foregroundStyle(.secondary)
                }
                Text("Available: \(account?.balanceGrams ?? "—") GRAM. Network fees are charged separately.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            if let errorMessage {
                Label(errorMessage, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
            }

            HStack {
                Button("Cancel") { dismiss() }
                    .disabled(isSubmitting)
                Spacer()
                Button {
                    isConfirming = true
                } label: {
                    if isSubmitting {
                        ProgressView()
                            .controlSize(.small)
                    } else {
                        Text("Send")
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(
                    isSubmitting || normalizedDestination.isEmpty || normalizedAmount.isEmpty
                )
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .desktopSheetSize(width: 520)
        .platformResizableModalPresentation()
        .interactiveDismissDisabled(isSubmitting)
#endif
        }
        .alert("Confirm transfer", isPresented: $isConfirming) {
            Button("Cancel", role: .cancel) {}
            Button("Send") { submit() }
        } message: {
            Text("Send \(normalizedAmount) GRAM to \(normalizedDestination)?")
        }
    }

    @ViewBuilder
    private var sendButton: some View {
        Button {
            focusedField = nil
            isConfirming = true
        } label: {
            if isSubmitting {
                ProgressView()
                    .controlSize(.small)
            } else {
                Text("Send")
            }
        }
        .disabled(isSubmitting || normalizedDestination.isEmpty || normalizedAmount.isEmpty)
    }

    private func submit() {
        isSubmitting = true
        errorMessage = nil
        Task {
            do {
                guard let nanograms = GramAmount.nanograms(from: normalizedAmount) else {
                    throw SendPresentationError.invalidAmount
                }
                let result = try await session.send(
                    SendRequest(
                        operationId: UUID().uuidString.lowercased(),
                        destination: normalizedDestination,
                        amount: .exact(nanograms: nanograms),
                        comment: nil,
                    )
                )
                switch result.phase {
                case .submitted, .confirmed:
                    onSubmitted()
                    dismiss()
                case .submissionUnknown:
                    errorMessage = "The transfer may have been submitted. Do not send it again. Message hash: \(result.messageHash)"
                case .failed, .replaced, .expired, .superseded, .lostRace:
                    errorMessage = session.snapshot.send.errorMessage
                        ?? "The transfer was rejected and was not submitted."
                case .cancelled:
                    errorMessage = "The transfer was cancelled."
                case .idle, .validating, .authorizing, .preparing, .persisting,
                     .readyToSubmit, .submitting:
                    errorMessage = "The transfer did not reach a final state."
                }
            } catch {
                errorMessage = error.localizedDescription
            }
            isSubmitting = false
        }
    }
}

private enum SendPresentationError: LocalizedError {
    case walletNotMigrated
    case invalidAmount

    var errorDescription: String? {
        switch self {
        case .walletNotMigrated:
            "This wallet must finish its  migration before it can send."
        case .invalidAmount:
            "Enter a positive GRAM amount with no more than 9 decimal places."
        }
    }
}

struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        ContentView()
    }
}
