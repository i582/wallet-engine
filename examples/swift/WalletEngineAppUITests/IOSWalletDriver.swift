import XCTest

enum IOSWalletDriverError: LocalizedError {
    case missingElement(String)

    var errorDescription: String? {
        switch self {
        case .missingElement(let description):
            "The iOS wallet did not show \(description)"
        }
    }
}

struct IOSActivityObservation: Equatable {
    let amounts: [String]
    let directions: [String]
    let ids: [String]
}

@MainActor
final class IOSWalletDriver {
    let app: XCUIApplication

    private let bridgeURL: String
    private let providerURL: String
    private let storageNamespace: String

    /// Creates a driver for one isolated simulator scenario.
    init(
        bridgeURL: String,
        providerURL: String,
        storageNamespace: String
    ) {
        app = XCUIApplication()
        self.bridgeURL = bridgeURL
        self.providerURL = providerURL
        self.storageNamespace = storageNamespace
    }

    /// Launches the wallet with a deterministic dark appearance, locale, endpoints, and storage.
    func open() throws {
        app.launchEnvironment["WALLET_ENGINE_CLIENT_E2E"] = "1"
        app.launchEnvironment["WALLET_ENGINE_CLIENT_E2E_STORAGE"] = storageNamespace
        app.launchEnvironment["TONCENTER_BASE_URL"] = providerURL
        app.launchEnvironment["TON_CONNECT_BRIDGE_URL"] = bridgeURL
        app.launchArguments += [
            "-AppleInterfaceStyle", "Dark",
            "-AppleLanguages", "(en)",
            "-AppleLocale", "en_US",
            "-appAppearance", "Dark",
        ]
        app.launch()
        try require(app.navigationBars["Wallet"], description: "the wallet screen")
    }

    /// Creates a testnet wallet and waits until its recovery phrase is visible.
    func createWallet() throws {
        let create = app.buttons["Create testnet wallet"]
        try require(create, description: "the create-wallet action")
        create.tap()
        let generate = app.buttons["Generate"]
        try require(generate, description: "the wallet generation action")
        generate.tap()
        try require(app.navigationBars["Recovery phrase"], description: "the recovery screen")
    }

    /// Confirms recovery backup and waits for the persisted wallet dashboard.
    func acceptRecovery() throws {
        let confirmation = app.switches["I saved all 12 words in a safe place"]
        try require(confirmation, description: "the recovery confirmation")
        confirmation.tap()
        let useWallet = app.buttons["Use wallet"]
        try require(useWallet, description: "the use-wallet action")
        useWallet.tap()
        try require(app.navigationBars["My Wallet"], description: "the wallet dashboard")
    }

    /// Restarts the app without changing the scenario's isolated storage.
    func reloadDashboard() throws {
        app.terminate()
        app.launch()
        try require(app.navigationBars["My Wallet"], description: "the restored wallet dashboard")
    }

    /// Refreshes account and activity data and waits for the complete UI update.
    func refresh() throws {
        let refresh = app.buttons["wallet-refresh-action"]
        try require(refresh, description: "the refresh action")
        refresh.tap()
        let progress = app.progressIndicators["activity-refreshing"]
        if progress.waitForExistence(timeout: 0.5), !progress.waitForNonExistence(timeout: 15) {
            throw IOSWalletDriverError.missingElement("a completed activity refresh")
        }
        let enabled = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "enabled == true"),
            object: refresh
        )
        guard XCTWaiter.wait(for: [enabled], timeout: 15) == .completed else {
            throw IOSWalletDriverError.missingElement("an enabled refresh action")
        }
    }

    /// Opens the TON Connect sheet for a new or restored dApp session.
    func openTonConnect() throws {
        let connect = app.buttons["Connect app"]
        try require(connect, description: "the connect-app action")
        connect.tap()
        try require(app.otherElements["ton-connect-sheet"], description: "the TON Connect sheet")
    }

    /// Enters a dApp-created connect link and waits for wallet approval.
    func handleConnectLink(_ link: String) throws {
        try openTonConnect()
        let input = app.textFields.firstMatch
        try require(input, description: "the TON Connect link field")
        input.tap()
        input.typeText(link)
        let continueButton = app.buttons["ton-connect-continue-action"]
        try require(continueButton, description: "the connection continue action")
        continueButton.tap()
        try require(
            app.navigationBars["Approve connection"],
            description: "the connection approval"
        )
    }

    /// Approves the current TON Connect connection request.
    func approveConnection() throws {
        let connect = app.buttons["ton-connect-approval-action"]
        try require(connect, description: "the connection approval action")
        connect.tap()
        try require(app.navigationBars["My Wallet"], description: "the wallet dashboard")
    }

    /// Approves the transaction currently displayed by the wallet.
    func approveRequest() throws {
        let send = app.buttons["ton-connect-approval-action"]
        try require(send, description: "the transaction approval action")
        send.tap()
        try require(app.navigationBars["My Wallet"], description: "the wallet dashboard")
    }

    /// Rejects the connection request currently displayed by the wallet.
    func rejectConnection() throws {
        let cancel = app.buttons["ton-connect-rejection-action"]
        try require(cancel, description: "the connection rejection action")
        cancel.tap()
        try require(app.navigationBars["My Wallet"], description: "the wallet dashboard")
    }

    /// Rejects the transaction currently displayed by the wallet.
    func rejectRequest() throws {
        let reject = app.buttons["ton-connect-rejection-action"]
        try require(reject, description: "the transaction rejection action")
        reject.tap()
        try require(app.navigationBars["My Wallet"], description: "the wallet dashboard")
    }

    /// Closes the current non-approval TON Connect sheet.
    func closeDialog() throws {
        let close = app.buttons["ton-connect-close-action"]
        try require(close, description: "the close action")
        close.tap()
        try require(app.navigationBars["My Wallet"], description: "the wallet dashboard")
    }

    /// Checks the empty-wallet state and its primary action.
    func assertWelcome() throws {
        try require(app.staticTexts["No wallet yet"], description: "the empty-wallet title")
        let create = app.buttons["Create testnet wallet"]
        try require(create, description: "the create-wallet action")
        guard create.isEnabled else {
            throw IOSWalletDriverError.missingElement("an enabled create-wallet action")
        }
    }

    /// Checks that all recovery words are present before backup confirmation.
    func assertRecovery() throws {
        try require(app.navigationBars["Recovery phrase"], description: "the recovery title")
        let words = app.descendants(matching: .any).matching(
            NSPredicate(format: "label BEGINSWITH %@", "Word ")
        )
        try require(words.firstMatch, description: "the first recovery word")
        guard words.count == 12 else {
            throw IOSWalletDriverError.missingElement("all 12 recovery words")
        }
        guard !app.buttons["Use wallet"].isEnabled else {
            throw IOSWalletDriverError.missingElement("a disabled use-wallet action")
        }
    }

    /// Checks the selected wallet, deterministic balance, and activity section.
    func assertDashboard() throws {
        try require(app.navigationBars["My Wallet"], description: "the wallet title")
        let balance = app.otherElements["Balance"]
        try require(balance, description: "the wallet balance")
        guard waitForValue("10 GRAM", in: balance) else {
            let diagnostics = app.staticTexts.allElementsBoundByIndex
                .map(\.label)
                .filter {
                    $0.localizedCaseInsensitiveContains("couldn")
                        || $0.localizedCaseInsensitiveContains("http")
                        || $0.localizedCaseInsensitiveContains("provider")
                }
                .joined(separator: " | ")
            throw IOSWalletDriverError.missingElement(
                "the 10 GRAM test balance; received \(String(describing: balance.value)); "
                    + "diagnostic: \(diagnostics)"
            )
        }
        try require(app.staticTexts["Recent activity"], description: "the activity section")
    }

    /// Reads stable activity identifiers and semantic row values in visual order.
    func observeActivity(count: Int) throws -> IOSActivityObservation {
        let rows = app.buttons.matching(
            NSPredicate(format: "identifier BEGINSWITH %@", "activity-")
        )
        let deadline = Date().addingTimeInterval(10)
        while rows.count != count, Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        }
        guard rows.count == count else {
            throw IOSWalletDriverError.missingElement("exactly \(count) activity rows")
        }
        let elements = rows.allElementsBoundByIndex
        return IOSActivityObservation(
            amounts: elements.map { element in
                ((element.value as? String) ?? "").split(separator: ",", maxSplits: 1)
                    .first.map(String.init) ?? ""
            },
            directions: elements.map { $0.label.lowercased() },
            ids: elements.map { String($0.identifier.dropFirst("activity-".count)) }
        )
    }

    /// Returns the amount phrase exposed to assistive technology for one activity row.
    func activityAmount(nanograms: String, direction: String) -> String {
        let prefix = direction == "received" ? "+" : "minus"
        return "\(prefix)\(formatNanograms(nanograms)) GRAM"
    }

    /// Checks the dApp identity, permissions, and enabled connect action.
    func assertConnectApproval(dappName: String) throws {
        try require(element(labeled: "Connect to \(dappName)"), description: "the dApp identity")
        try require(app.staticTexts["Wallet address"], description: "the address permission")
        try require(
            app.staticTexts["Transaction approvals"],
            description: "the transaction permission"
        )
        let connect = app.buttons["ton-connect-approval-action"]
        try require(connect, description: "the connection approval action")
        guard connect.isEnabled else {
            throw IOSWalletDriverError.missingElement("an enabled connection approval action")
        }
    }

    /// Checks the restored dApp identity and disconnect action.
    func assertConnectedDapp(dappName: String) throws {
        try require(
            element(labeled: "Connected to \(dappName)"),
            description: "the connected dApp identity"
        )
        let disconnect = app.buttons["Disconnect"]
        try require(disconnect, description: "the disconnect action")
        guard disconnect.isEnabled else {
            throw IOSWalletDriverError.missingElement("an enabled disconnect action")
        }
    }

    /// Checks the empty TON Connect form shown when no dApp session is active.
    func assertTonConnectEntry() throws {
        try require(app.navigationBars["TON Connect"], description: "the TON Connect title")
        try require(app.staticTexts["Connect an app"], description: "the connection form title")
        try require(app.textFields.firstMatch, description: "the connection link field")
        let continueButton = app.buttons["ton-connect-continue-action"]
        try require(continueButton, description: "the connection continue action")
        guard !continueButton.isEnabled else {
            throw IOSWalletDriverError.missingElement("a disabled empty-link continue action")
        }
    }

    /// Checks every transaction message rendered in the approval sheet.
    func assertTransaction(messages: [TransactionMessageConfig]) throws {
        try require(
            app.navigationBars["Review transaction"],
            description: "the transaction review"
        )
        for (index, message) in messages.enumerated() {
            try require(
                app.staticTexts["Message \(index + 1) of \(messages.count)"],
                description: "transaction message \(index + 1)"
            )
            try require(
                app.staticTexts["\(formatNanograms(message.amount)) GRAM"],
                description: "transaction amount \(index + 1)"
            )
        }
        guard app.staticTexts.matching(identifier: "StateInit").count == messages.count else {
            throw IOSWalletDriverError.missingElement("one StateInit row per message")
        }
        try scrollTransactionSummaryIntoView()
        let reject = app.buttons["ton-connect-rejection-action"]
        try require(reject, description: "the transaction rejection action")
        guard reject.isEnabled else {
            throw IOSWalletDriverError.missingElement("an enabled rejection action")
        }
    }

    /// Scrolls a long transaction batch until its signed-message summary is completely visible.
    private func scrollTransactionSummaryIntoView() throws {
        let summary = app.staticTexts["Message BOC"]
        try require(summary, description: "the complete transaction summary")
        let sheet = app.otherElements["ton-connect-sheet"]
        try require(sheet, description: "the TON Connect sheet")
        for _ in 0..<10 where !isFullyVisible(summary) {
            sheet.swipeUp()
        }
        try requireFullyVisible(summary, description: "the complete transaction summary")
    }

    /// Captures the complete wallet surface and masks recovery secrets when required.
    func capture(target: ScreenshotTarget) throws -> SnapshotCapture {
        let element = app
        var masks = [CGRect]()
        if target == .recovery {
            for identifier in ["recovery-wallet-address", "recovery-words"] {
                let secret = app.descendants(matching: .any)[identifier]
                if secret.exists {
                    masks.append(secret.frame.offsetBy(dx: -element.frame.minX, dy: -element.frame.minY))
                }
            }
        }
        return SnapshotCapture(
            screenshot: element.screenshot(),
            masks: masks,
            ignoredComparisonRegions: ignoredComparisonRegions(in: element, target: target)
        )
    }

    /// Finds system chrome and dynamic transaction timestamps that are not snapshot state.
    private func ignoredComparisonRegions(
        in element: XCUIElement,
        target: ScreenshotTarget
    ) -> [CGRect] {
        let origin = element.frame.origin
        let navigationBar = app.navigationBars.firstMatch
        let statusBarHeight = navigationBar.exists
            ? max(0, navigationBar.frame.minY - element.frame.minY)
            : 62
        var regions = [CGRect(
            x: 0,
            y: 0,
            width: element.frame.width,
            height: statusBarHeight
        )]
        guard target == .page else {
            return regions
        }
        let activityRows = app.buttons.matching(
            NSPredicate(format: "identifier BEGINSWITH %@", "activity-")
        )
        for row in activityRows.allElementsBoundByIndex {
            let timestamps = row.staticTexts.matching(
                NSPredicate(format: "label CONTAINS %@", " at ")
            )
            for timestamp in timestamps.allElementsBoundByIndex {
                regions.append(
                    timestamp.frame
                        .offsetBy(dx: -origin.x, dy: -origin.y)
                        .insetBy(dx: -2, dy: -2)
                )
            }
        }
        return regions
    }

    /// Waits for one required accessibility element and reports a semantic name on failure.
    private func require(
        _ element: XCUIElement,
        description: String,
        timeout: TimeInterval = 10
    ) throws {
        guard element.waitForExistence(timeout: timeout) else {
            throw IOSWalletDriverError.missingElement(description)
        }
    }

    /// Waits until an asynchronous accessibility value matches its expected rendered value.
    private func waitForValue(
        _ expected: String,
        in element: XCUIElement,
        timeout: TimeInterval = 10
    ) -> Bool {
        let predicate = NSPredicate(format: "value == %@", expected)
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: element)
        return XCTWaiter.wait(for: [expectation], timeout: timeout) == .completed
    }

    /// Requires an element and its complete frame to remain inside the captured application surface.
    private func requireFullyVisible(_ element: XCUIElement, description: String) throws {
        try require(element, description: description)
        guard isFullyVisible(element) else {
            throw IOSWalletDriverError.missingElement(description)
        }
    }

    /// Reports whether an element's complete frame is inside the captured application surface.
    private func isFullyVisible(_ element: XCUIElement) -> Bool {
        !element.frame.isEmpty && app.frame.insetBy(dx: -1, dy: -1).contains(element.frame)
    }

    /// Finds an element by label without depending on its SwiftUI accessibility type.
    private func element(labeled label: String) -> XCUIElement {
        app.descendants(matching: .any)
            .matching(NSPredicate(format: "label == %@", label))
            .firstMatch
    }

    /// Formats an integer nanogram amount with the decimal form shown by the wallet.
    private func formatNanograms(_ value: String) -> String {
        guard let nanograms = Decimal(string: value) else { return value }
        let grams = nanograms / Decimal(1_000_000_000)
        return NSDecimalNumber(decimal: grams).stringValue
    }
}
