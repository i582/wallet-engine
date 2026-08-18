import SwiftUI
import WalletEngineFFI

struct TonConnectView: View {
    @Environment(\.dismiss) private var dismiss

    let coordinator: TonConnectCoordinator

    @State private var link = ""
    @State private var localError: String?
    @State private var force = false
    @FocusState private var isLinkFocused: Bool

    var body: some View {
        NavigationStack {
            ScrollView {
                content
                    .frame(maxWidth: 560, alignment: .leading)
                    .padding()
            }
            .navigationTitle(title)
            .platformCompactNavigationTitle()
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(coordinator.approval == nil ? "Close" : rejectionTitle, action: close)
                        .disabled(coordinator.isWorking)
                }
                if coordinator.approval != nil {
                    ToolbarItem(placement: .confirmationAction) {
                        Button(action: approveCurrentApproval) {
                            if coordinator.isWorking {
                                ProgressView()
                                    .controlSize(.small)
                            } else {
                                Text(approvalTitle)
                            }
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(
                            coordinator.isWorking
                                || (isTransactionApproval && coordinator.canForceRetry && !force)
                        )
                    }
                } else if coordinator.connection == nil {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Continue", action: connect)
                            .buttonStyle(.borderedProminent)
                            .disabled(
                                coordinator.isWorking
                                    || link.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            )
                    }
                }
            }
        }
        .platformModalPresentation()
        .interactiveDismissDisabled(coordinator.isWorking)
        .defaultFocus($isLinkFocused, true)
        .onChange(of: coordinator.approval?.id) { _, _ in
            force = false
        }
        .onDisappear {
            guard coordinator.approval != nil, !coordinator.isWorking else { return }
            Task {
                await rejectCurrentApproval()
            }
        }
    }

    @ViewBuilder
    private var content: some View {
        if let approval = coordinator.approval {
            switch approval {
            case .connect(let manifest, let prompt):
                ConnectApprovalView(
                    manifest: manifest,
                    requestsProof: prompt.proofPayload != nil
                )
            case .transaction(let manifest, _, let preview):
                TonConnectTransactionView(
                    manifest: manifest,
                    preview: preview,
                    canForceRetry: coordinator.canForceRetry,
                    force: $force
                )
            }
        } else if let connection = coordinator.connection {
            ConnectedDAppView(
                connection: connection,
                isWorking: coordinator.isWorking,
                onDisconnect: disconnect
            )
        } else {
            connectForm
        }

        if let message = localError ?? coordinator.diagnostic {
            DismissibleDiagnostic(
                message: message,
                systemImage: "exclamationmark.triangle.fill",
                onDismiss: {
                    localError = nil
                    coordinator.dismissDiagnostic()
                }
            )
            .padding(.top, 16)
        }
    }

    private var connectForm: some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Connect an app")
                    .font(.title2.weight(.semibold))
                Text("Paste a TON Connect link from the app you want to use with this wallet.")
                    .foregroundStyle(.secondary)
            }

            TextField("tc://?v=2&id=…", text: $link)
                .textFieldStyle(.roundedBorder)
                .platformTonConnectLinkInput()
                .focused($isLinkFocused)
                .onSubmit { connect() }

            HStack {
                PasteButton(payloadType: String.self) { values in
                    link = values.first ?? ""
                }
            }
        }
    }

    private var title: String {
        switch coordinator.approval {
        case .connect:
            "Approve connection"
        case .transaction:
            "Review transaction"
        case nil:
            coordinator.connection == nil ? "TON Connect" : "Connected app"
        }
    }

    private var rejectionTitle: String {
        switch coordinator.approval {
        case .connect:
            "Cancel"
        case .transaction:
            "Reject"
        case nil:
            "Close"
        }
    }

    private var approvalTitle: String {
        switch coordinator.approval {
        case .connect:
            "Connect"
        case .transaction:
            "Send"
        case nil:
            ""
        }
    }

    private var isTransactionApproval: Bool {
        if case .transaction = coordinator.approval { return true }
        return false
    }

    private func connect() {
        let normalized = link.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return }
        localError = nil
        Task {
            do {
                try await coordinator.start(link: normalized)
            } catch {
                localError = error.localizedDescription
            }
        }
    }

    private func approveConnection() {
        Task {
            await coordinator.approveConnection()
            if coordinator.connection != nil {
                dismiss()
            }
        }
    }

    private func rejectConnection() {
        Task {
            await coordinator.rejectConnection()
            dismiss()
        }
    }

    private func approveTransaction() {
        Task {
            await coordinator.approveTransaction(force: force)
            if coordinator.approval == nil {
                dismiss()
            }
        }
    }

    private func rejectTransaction() {
        Task {
            await coordinator.rejectTransaction()
            dismiss()
        }
    }

    private func approveCurrentApproval() {
        switch coordinator.approval {
        case .connect:
            approveConnection()
        case .transaction:
            approveTransaction()
        case nil:
            break
        }
    }

    private func disconnect() {
        Task {
            await coordinator.disconnect()
            if coordinator.connection == nil {
                dismiss()
            }
        }
    }

    private func close() {
        guard coordinator.approval != nil else {
            dismiss()
            return
        }
        Task {
            await rejectCurrentApproval()
            dismiss()
        }
    }

    private func rejectCurrentApproval() async {
        switch coordinator.approval {
        case .connect:
            await coordinator.rejectConnection()
        case .transaction:
            await coordinator.rejectTransaction()
        case nil:
            break
        }
    }
}

private struct ConnectApprovalView: View {
    let manifest: TonConnectManifest
    let requestsProof: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 24) {
            TonConnectAppIdentityView(manifest: manifest, status: nil)
            TonConnectPermissionsCard(requestsProof: requestsProof)
        }
    }
}

private struct TonConnectAppIdentityView: View {
    @ScaledMetric(relativeTo: .largeTitle) private var appIconSize = 76.0

    let manifest: TonConnectManifest
    let status: TonConnectAppStatus?

    var body: some View {
        VStack(spacing: 14) {
            AsyncImage(url: URL(string: manifest.iconUrl)) { phase in
                switch phase {
                case .success(let image):
                    image.resizable().scaledToFill()
                case .failure, .empty:
                    Image(systemName: "app.fill")
                        .font(.system(size: 30))
                        .foregroundStyle(.secondary)
                @unknown default:
                    Image(systemName: "app.fill")
                        .font(.system(size: 30))
                        .foregroundStyle(.secondary)
                }
            }
            .frame(width: appIconSize, height: appIconSize)
            .background(.background.secondary)
            .clipShape(.rect(cornerRadius: 20))
            .accessibilityHidden(true)

            VStack(spacing: 5) {
                Text(manifest.name)
                    .font(.title2.weight(.semibold))
                    .multilineTextAlignment(.center)
                Text(manifest.domain)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .textSelection(.enabled)
            }

            if let status {
                Label(status.title, systemImage: status.systemImage)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(status.color)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 7)
                    .background(status.color.opacity(0.14), in: Capsule())
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 8)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(status?.accessibilityLabel ?? "Connect to \(manifest.name)")
        .accessibilityValue(manifest.domain)
    }
}

private enum TonConnectAppStatus {
    case connected

    var title: String {
        switch self {
        case .connected:
            "Connected"
        }
    }

    var systemImage: String {
        switch self {
        case .connected:
            "checkmark.circle.fill"
        }
    }

    var color: Color {
        switch self {
        case .connected:
            .green
        }
    }

    var accessibilityLabel: String {
        switch self {
        case .connected:
            "Connected to this app"
        }
    }
}

private struct TonConnectPermissionsCard: View {
    @ScaledMetric(relativeTo: .body) private var iconColumnWidth = 28.0

    let requestsProof: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            TonConnectCapabilityRow(
                title: "Wallet address",
                detail: "Visible to this app",
                systemImage: "eye",
                iconWidth: iconColumnWidth
            )
            Divider()
                .padding(.leading, iconColumnWidth + 16)
            TonConnectCapabilityRow(
                title: "Transaction approvals",
                detail: "You review every request",
                systemImage: "checkmark.shield",
                iconWidth: iconColumnWidth
            )
            if requestsProof {
                Divider()
                    .padding(.leading, iconColumnWidth + 16)
                TonConnectCapabilityRow(
                    title: "Wallet ownership",
                    detail: "Signature requested to verify this wallet",
                    systemImage: "signature",
                    iconWidth: iconColumnWidth
                )
            }
        }
        .padding(.horizontal, 16)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 20))
    }
}

private struct TonConnectCapabilityRow: View {
    let title: String
    let detail: String
    let systemImage: String
    let iconWidth: CGFloat

    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 3) {
                Text(title)
                    .font(.body.weight(.medium))
                Text(detail)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        } icon: {
            Image(systemName: systemImage)
                .frame(width: iconWidth, alignment: .center)
                .foregroundStyle(.tint)
        }
        .padding(.vertical, 15)
        .accessibilityElement(children: .combine)
    }
}

private struct TonConnectTransactionView: View {
    let manifest: TonConnectManifest
    let preview: SendPreview
    let canForceRetry: Bool
    @Binding var force: Bool

    private var amount: String {
        guard case .exact(let nanograms) = preview.message.amount else { return "All balance" }
        return "\(GramAmount.format(nanograms: nanograms)) GRAM"
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("\(manifest.name) wants to send")
                .font(.title2.weight(.semibold))

            VStack(spacing: 0) {
                PreviewRow(label: "Send", value: amount)
                PreviewRow(label: "To", value: compact(preview.message.destination))
                PreviewRow(
                    label: "Network fee",
                    value: "\(GramAmount.format(nanograms: preview.emulation.walletFeesNanograms)) GRAM"
                )
                PreviewRow(
                    label: "Transactions",
                    value: String(preview.emulation.transactionCount)
                )
                PreviewRow(label: "Message BOC", value: compact(preview.messageBocBase64))
            }
            .background(.background.secondary, in: RoundedRectangle(cornerRadius: 14))

            if !preview.emulation.actions.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Actions")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    ForEach(preview.emulation.actions, id: \.actionId) { action in
                        Label(
                            action.kind.replacingOccurrences(of: "_", with: " "),
                            systemImage: action.succeeded ? "checkmark.circle" : "exclamationmark.triangle"
                        )
                        .foregroundStyle(action.succeeded ? Color.primary : Color.orange)
                    }
                }
            }

            if !preview.emulation.traceSucceeded || preview.emulation.isIncomplete {
                Label(
                    "Some emulated actions may fail. The network can still accept the wallet transaction.",
                    systemImage: "exclamationmark.triangle.fill"
                )
                .font(.callout)
                .foregroundStyle(.orange)
            }

            if canForceRetry {
                VStack(alignment: .leading, spacing: 8) {
                    Label("Previous transfer is unresolved", systemImage: "exclamationmark.triangle.fill")
                        .font(.callout.weight(.semibold))
                    Text("Its signed message may still execute. If you approve this request, both transfers can affect the balance.")
                        .font(.caption)
                    Toggle("I understand. Approve this transaction anyway.", isOn: $force)
                }
                .foregroundStyle(.orange)
                .padding(14)
                .background(.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 12))
            }
        }
    }

    private func compact(_ value: String) -> String {
        guard value.count > 24 else { return value }
        return "\(value.prefix(12))…\(value.suffix(8))"
    }
}

private struct ConnectedDAppView: View {
    let connection: TonConnectConnection
    let isWorking: Bool
    let onDisconnect: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 24) {
            TonConnectAppIdentityView(
                manifest: connection.manifest,
                status: .connected
            )
            TonConnectPermissionsCard(requestsProof: false)

            Button("Disconnect", role: .destructive, action: onDisconnect)
                .buttonStyle(.bordered)
                .controlSize(.large)
                .frame(maxWidth: .infinity)
                .disabled(isWorking)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct PreviewRow: View {
    let label: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 16) {
            Text(label)
                .foregroundStyle(.secondary)
            Spacer()
            Text(value)
                .multilineTextAlignment(.trailing)
                .lineLimit(2)
                .textSelection(.enabled)
        }
        .padding(14)
        .overlay(alignment: .bottom) {
            Divider()
        }
        .accessibilityElement(children: .combine)
    }
}
