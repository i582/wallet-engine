//
//  WalletEngineApp.swift
//  TON Wallet
//
//  Created by Petr Makhnev on 12.08.26.
//

import SwiftUI

@main
struct WalletEngineApp: App {
    var body: some Scene {
#if os(macOS)
        WindowGroup {
            ContentView()
        }
        .defaultSize(width: 1120, height: 760)

        Settings {
            SettingsView()
        }
#else
        WindowGroup {
            ContentView()
        }
#endif
    }
}
