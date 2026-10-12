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
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(uiColor: .systemBackground).ignoresSafeArea())
            .environment(store)
            .task { store.resume() }
            .onChange(of: scenePhase) { _, phase in
                if phase == .active { store.resume() }
                if phase == .background { store.suspend() }
            }
        }
    }
}

/// Chats, Mail when a mail account is set up, and Settings with the accounts.
struct RootView: View {
    @Environment(QmsgStore.self) private var store
    @State private var router = AppRouter()
    @State private var accounts = AccountsModel()

    private var attention: Int {
        accounts.items(store).filter { $0.health.needsUser }.count
    }

    private var mailAccounts: [ProviderState] {
        store.sortedProviders.filter { accounts.isMail($0) }
    }

    /// Unread mail among the loaded inboxes; the server keeps no count.
    private var unreadMail: Int {
        mailAccounts.reduce(0) { total, provider in
            total + provider.sortedOrganizations.reduce(0) { sum, org in
                guard let inbox = org.scope.mailboxes.first(where: { MailboxRole($0) == .inbox }) else { return sum }
                let key = ChannelKey(provider: provider.name, channel: ChannelRef(organization: org.id, channel: inbox.id))
                return sum + store.conversation(key).messages.reduce(0) { $0 + ($1.email?.read == false ? 1 : 0) }
            }
        }
    }

    var body: some View {
        @Bindable var router = router
        let hasMail = !mailAccounts.isEmpty
        TabView(selection: $router.tab) {
            Tab("Chats", systemImage: "bubble.left.and.bubble.right", value: AppRouter.Tab.chats) {
                ChatsView()
            }
            if hasMail {
                Tab("Mail", systemImage: "envelope", value: AppRouter.Tab.mail) {
                    MailView()
                }
                .badge(unreadMail)
            }
            Tab("Settings", systemImage: "gearshape", value: AppRouter.Tab.settings) {
                SettingsView()
            }
            .badge(attention)
        }
        .environment(router)
        .environment(accounts)
        .onChange(of: hasMail) { _, hasMail in
            if !hasMail && router.tab == .mail { router.tab = .chats }
        }
        // Reload the account list when the server's set of running accounts changes.
        .task(id: store.providers.keys.sorted()) { await accounts.load(store) }
        .task(id: store.link) { if store.link == .live { await accounts.load(store) } }
    }
}
