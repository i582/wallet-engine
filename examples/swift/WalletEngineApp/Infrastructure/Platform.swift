import SwiftUI

#if os(macOS)
import AppKit
typealias PlatformImage = NSImage
#elseif os(iOS)
import UIKit
typealias PlatformImage = UIImage
#endif

extension Color {
    static var platformWindowBackground: Color {
#if os(macOS)
        Color(nsColor: .windowBackgroundColor)
#else
        Color(uiColor: .systemBackground)
#endif
    }
}

enum PlatformPasteboard {
    static func copy(_ value: String) {
#if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(value, forType: .string)
#else
        UIPasteboard.general.string = value
#endif
    }
}

enum PlatformCopy {
    static var localDeviceName: String {
#if os(macOS)
        "Mac"
#else
        "iPhone"
#endif
    }

    static var localStorageIcon: String {
#if os(macOS)
        "externaldrive"
#else
        "iphone"
#endif
    }
}

extension View {
    @ViewBuilder
    func platformTonConnectLinkInput() -> some View {
#if os(iOS)
        textInputAutocapitalization(.never)
            .autocorrectionDisabled()
#else
        self
#endif
    }

    @ViewBuilder
    func desktopMinimumSize(width: CGFloat, height: CGFloat) -> some View {
#if os(macOS)
        frame(minWidth: width, minHeight: height)
#else
        self
#endif
    }

    @ViewBuilder
    func desktopSheetSize(width: CGFloat, minHeight: CGFloat? = nil) -> some View {
#if os(macOS)
        if let minHeight {
            frame(width: width).frame(minHeight: minHeight)
        } else {
            frame(width: width)
        }
#else
        self
#endif
    }

    @ViewBuilder
    func desktopHelp(_ text: String) -> some View {
#if os(macOS)
        help(text)
#else
        accessibilityHint(text)
#endif
    }

    @ViewBuilder
    func platformLinkButtonStyle() -> some View {
#if os(macOS)
        buttonStyle(.link)
#else
        buttonStyle(.plain)
            .foregroundStyle(.tint)
#endif
    }

    @ViewBuilder
    func platformConfirmationToggleStyle() -> some View {
#if os(macOS)
        toggleStyle(.checkbox)
#else
        toggleStyle(.switch)
#endif
    }

    @ViewBuilder
    func platformModalPresentation() -> some View {
#if os(iOS)
        presentationDetents([.large])
            .presentationDragIndicator(.visible)
#else
        self
#endif
    }

    @ViewBuilder
    func platformResizableModalPresentation() -> some View {
#if os(iOS)
        presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
#else
        self
#endif
    }

    @ViewBuilder
    func desktopFormPadding() -> some View {
#if os(macOS)
        padding(.horizontal, 24)
#else
        self
#endif
    }

    @ViewBuilder
    func platformCompactNavigationTitle() -> some View {
#if os(iOS)
        navigationBarTitleDisplayMode(.inline)
#else
        self
#endif
    }

    @ViewBuilder
    func platformCompactIconButtonFrame() -> some View {
#if os(iOS)
        frame(width: 44, height: 44)
#else
        frame(width: 32, height: 32)
#endif
    }

    @ViewBuilder
    func platformWalletAddressInput() -> some View {
#if os(iOS)
        textInputAutocapitalization(.never)
            .autocorrectionDisabled()
#else
        self
#endif
    }

    @ViewBuilder
    func platformDecimalInput() -> some View {
#if os(iOS)
        keyboardType(.decimalPad)
#else
        self
#endif
    }
}
