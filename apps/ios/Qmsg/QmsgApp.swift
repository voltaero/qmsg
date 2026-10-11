import SwiftUI
import QmsgKit

@main
@MainActor
struct QmsgApp: App {
    @State private var store = QmsgStore(accounts: KeychainAccountStore())
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            Group {
                if store.isPaired {
                    ChannelsView()
                } else {
                    PairingView()
                }
            }
            .environment(store)
            .task { store.resume() }
            .onChange(of: scenePhase) { _, phase in
                if phase == .active { store.resume() }
                if phase == .background { store.suspend() }
            }
        }
    }
}
