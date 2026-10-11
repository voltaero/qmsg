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
                    RootView()
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

/// Chats, Accounts and Settings, like other messaging apps.
struct RootView: View {
    @Environment(QmsgStore.self) private var store
    @State private var router = AppRouter()
    @State private var accounts = AccountsModel()

    private var attention: Int {
        accounts.items(store).filter { $0.health.needsUser }.count
    }

    var body: some View {
        @Bindable var router = router
        TabView(selection: $router.tab) {
            Tab("Chats", systemImage: "bubble.left.and.bubble.right.fill", value: AppRouter.Tab.chats) {
                ChatsView()
            }
            Tab("Accounts", systemImage: "person.crop.circle", value: AppRouter.Tab.accounts) {
                AccountsView()
            }
            .badge(attention)
            Tab("Settings", systemImage: "gearshape", value: AppRouter.Tab.settings) {
                NavigationStack { SettingsView() }
            }
        }
        .environment(router)
        .environment(accounts)
        // Reload the account list when the server's set of running accounts changes.
        .task(id: store.providers.keys.sorted()) { await accounts.load(store) }
        .task(id: store.link) { if store.link == .live { await accounts.load(store) } }
    }
}
