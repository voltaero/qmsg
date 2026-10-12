import SwiftUI
import QmsgKit

/// What the Chats sidebar can open: a conversation, or a server's channels.
enum ChatRoute: Hashable {
    case chat(ChannelKey)
    case server(provider: String, organization: String)
}

/// Every chat account in one list: chats opened lately, then servers, then
/// direct messages, newest first. Mail lives in its own tab.
struct ChatsView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var accounts
    @Environment(AppRouter.self) private var router
    @State private var selection: ChatRoute?
    @State private var composing = false
    @State private var search = ""
    @State private var filter: String?

    private var chatProviders: [ProviderState] {
        store.sortedProviders.filter { !accounts.isMail($0) }
    }

    private var shownProviders: [ProviderState] {
        chatProviders.filter { filter == nil || $0.name == filter }
    }

    var body: some View {
        NavigationSplitView {
            List(selection: $selection) {
                Group {
                    if search.isEmpty { browse } else { results }
                }
                .listRowBackground(Color.appCanvas)
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .background(Color.appCanvas.ignoresSafeArea())
            .safeAreaInset(edge: .top, spacing: 0) {
                ConnectionBanner(link: store.link) { store.resume() }
            }
            .overlay { emptyState }
            .navigationTitle("Chats")
            .searchable(text: $search, prompt: "Search")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button("New Message", systemImage: "square.and.pencil") { composing = true }
                        .disabled(chatProviders.isEmpty)
                }
            }
            .onChange(of: chatProviders.map(\.name)) { _, names in
                if let filter, !names.contains(filter) { self.filter = nil }
            }
        } detail: {
            NavigationStack {
                switch selection {
                case .chat(let key):
                    ChatView(key: key)
                case .server(let provider, let organization):
                    ServerView(provider: provider, organization: organization)
                case nil:
                    ContentUnavailableView("No Chat Selected", systemImage: "bubble.left.and.bubble.right", description: Text("Choose a conversation or a server."))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.appCanvas.ignoresSafeArea())
            .id(selection)
        }
        .sheet(isPresented: $composing) {
            NavigationStack {
                NewChatView { key in
                    selection = .chat(key)
                    composing = false
                }
            }
        }
    }

    // MARK: Browsing

    @ViewBuilder
    private var browse: some View {
        statusRows
        if chatProviders.count > 1 { filterChips }
        let recent = recentItems
        if !recent.isEmpty {
            Section("Recent") {
                ForEach(recent, id: \.key) { item in
                    NavigationLink(value: ChatRoute.chat(item.key)) { ChatRow(item: item) }
                }
            }
        }
        let servers = serverItems
        if !servers.isEmpty {
            Section("Servers") {
                ForEach(servers, id: \.route) { server in
                    NavigationLink(value: server.route) { ServerRow(server: server) }
                }
            }
        }
        let shown = Set(recent.map(\.key))
        let direct = directItems.filter { !shown.contains($0.key) }
        if !direct.isEmpty {
            Section("Direct Messages") {
                ForEach(direct, id: \.key) { item in
                    NavigationLink(value: ChatRoute.chat(item.key)) { ChatRow(item: item) }
                }
            }
        }
    }

    /// Accounts that need signing in or are still starting, so an empty
    /// list explains itself.
    @ViewBuilder
    private var statusRows: some View {
        let items = accounts.items(store).filter { item in
            if let live = item.live {
                if accounts.isMail(live) { return false }
            } else if (accounts.type(for: item.name) ?? "").contains("mail") {
                return false
            }
            if item.health.needsUser { return true }
            guard let live = item.live else { return false }
            return item.health.tone == .busy && live.standalone.channels.isEmpty && live.organizations.isEmpty
        }
        if !items.isEmpty {
            Section {
                ForEach(items) { item in
                    Button { router.show(account: item.name) } label: {
                        HStack(spacing: 12) {
                            NetworkBadge(style: item.style, size: 30)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(statusTitle(item)).font(.subheadline.weight(.semibold))
                                if item.health.needsUser, let detail = item.health.detail {
                                    Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                                }
                            }
                            Spacer(minLength: 0)
                            if item.health.needsUser {
                                Image(systemName: "chevron.right").font(.caption.bold()).foregroundStyle(.tertiary)
                            } else {
                                ProgressView().controlSize(.small)
                            }
                        }
                    }
                    .buttonStyle(.plain)
                    .listRowBackground(item.health.needsUser ? item.health.color.opacity(0.1) : nil)
                }
            }
        }
    }

    private func statusTitle(_ item: AccountsModel.Item) -> String {
        if item.live?.status.loginStep != nil { return "Sign in to \(item.label)" }
        if item.health.needsUser { return "\(item.label): \(item.health.title.lowercased())" }
        return "\(item.label) is \(item.health.title.lowercased())…"
    }

    private var filterChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                chip("All", style: nil, selected: filter == nil) { filter = nil }
                ForEach(chatProviders) { provider in
                    chip([accounts.label(for: provider.name), provider.accountIdentity].compactMap { $0 }.joined(separator: " · "), style: accounts.style(for: provider.name), selected: filter == provider.name) {
                        filter = filter == provider.name ? nil : provider.name
                    }
                }
            }
            .padding(.horizontal, 16).padding(.vertical, 6)
        }
        .listRowInsets(EdgeInsets())
        .listRowSeparator(.hidden)
    }

    private func chip(_ title: String, style: NetworkStyle?, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 6) {
                if let style { Image(systemName: style.symbol).font(.caption2).foregroundStyle(selected ? .white : style.color) }
                Text(title).font(.subheadline.weight(.medium))
            }
            .padding(.horizontal, 12).padding(.vertical, 7)
            .foregroundStyle(selected ? .white : .primary)
            .background(selected ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.quaternary), in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    // MARK: Search

    @ViewBuilder
    private var results: some View {
        let (chats, servers) = searchHits
        if !chats.isEmpty {
            Section("Chats") {
                ForEach(chats, id: \.key) { item in
                    NavigationLink(value: ChatRoute.chat(item.key)) { ChatRow(item: item) }
                }
            }
        }
        if !servers.isEmpty {
            Section("Servers") {
                ForEach(servers, id: \.route) { server in
                    NavigationLink(value: server.route) { ServerRow(server: server) }
                }
            }
        }
    }

    /// Chats and channels whose names match, then servers that do.
    private var searchHits: ([ChatItem], [ServerItem]) {
        var chats: [ChatItem] = []
        for provider in shownProviders {
            for channel in provider.standalone.channels.values where !channel.isContainer && matches(provider.standalone.title(of: channel)) {
                chats.append(chatItem(provider, organization: nil, channel: channel))
            }
            for org in provider.sortedOrganizations {
                for channel in org.scope.channels.values where !channel.isContainer && !channel.isMailbox && matches(org.scope.title(of: channel)) {
                    chats.append(chatItem(provider, organization: org.id, channel: channel))
                }
            }
        }
        chats.sort { ($0.date ?? .distantPast, $1.title) > ($1.date ?? .distantPast, $0.title) }
        let servers = serverItems.filter { matches($0.name) }
        return (Array(chats.prefix(60)), servers)
    }

    // MARK: Empty states

    @ViewBuilder
    private var emptyState: some View {
        if chatProviders.isEmpty {
            if store.link == .live {
                ContentUnavailableView {
                    Label("No Chat Accounts", systemImage: "bubble.left.and.bubble.right")
                } description: {
                    Text("Add Discord or another network to bring your chats here.")
                } actions: {
                    Button("Add Account") { router.addAccount() }.buttonStyle(.borderedProminent)
                }
            } else if store.providers.isEmpty {
                ProgressView("Loading your chats…")
            }
        } else if !search.isEmpty {
            let (chats, servers) = searchHits
            if chats.isEmpty && servers.isEmpty { ContentUnavailableView.search(text: search) }
        } else if recentItems.isEmpty && serverItems.isEmpty && directItems.isEmpty
                    && shownProviders.allSatisfy({ $0.status.isReady }) {
            ContentUnavailableView {
                Label("No Chats Yet", systemImage: "bubble.left")
            } description: {
                Text("Start a conversation to see it here.")
            } actions: {
                Button("New Message") { composing = true }.buttonStyle(.borderedProminent)
            }
        }
    }

    // MARK: Data

    private var recentItems: [ChatItem] {
        store.recentKeys.compactMap { key -> ChatItem? in
            guard filter == nil || key.provider == filter,
                  let provider = store.providers[key.provider], !accounts.isMail(provider),
                  let channel = provider.channel(key.channel), !channel.isMailbox else { return nil }
            return chatItem(provider, organization: key.channel.organization, channel: channel)
        }
    }

    private var directItems: [ChatItem] {
        shownProviders.flatMap { provider in
            provider.standalone.channels.values
                .filter { !$0.isContainer && !$0.isMailbox }
                .map { chatItem(provider, organization: nil, channel: $0) }
        }
        .sorted { ($0.date ?? .distantPast, $1.title) > ($1.date ?? .distantPast, $0.title) }
    }

    private var serverItems: [ServerItem] {
        // Unread per server, in one pass over the activity.
        var unread: [String: Int] = [:]
        for (key, entry) in store.activity where entry.unread > 0 {
            if let org = key.channel.organization { unread[key.provider + "\n" + org, default: 0] += entry.unread }
        }
        let showsAccount = chatProviders.count > 1
        return shownProviders.flatMap { provider in
            provider.sortedOrganizations
                .filter { !$0.scope.channels.values.contains(where: \.isMailbox) }
                .map { org in
                    ServerItem(
                        route: .server(provider: provider.name, organization: org.id),
                        name: org.name, icon: org.icon,
                        account: showsAccount ? accounts.label(for: provider.name) : nil,
                        unread: unread[provider.name + "\n" + org.id] ?? 0
                    )
                }
        }
    }

    private func chatItem(_ provider: ProviderState, organization: String?, channel: Channel) -> ChatItem {
        ChatItem(store: store, accounts: accounts, provider: provider, organization: organization, channel: channel)
    }

    private func matches(_ text: String) -> Bool {
        search.isEmpty || text.localizedCaseInsensitiveContains(search)
    }
}

struct ChatItem {
    var key: ChannelKey
    var title: String
    var subtitle: String?
    var place: String?
    var date: Date?
    var unread: Int
    var isGroup: Bool
    var isServerChannel: Bool
    var icon: Media?
    var network: NetworkStyle

    @MainActor
    init(store: QmsgStore, accounts: AccountsModel, provider: ProviderState, organization: String?, channel: Channel) {
        let key = ChannelKey(provider: provider.name, channel: ChannelRef(organization: organization, channel: channel.id))
        let scope = provider.scope(organization)
        let activity = store.activity[key]
        var subtitle = activity?.last?.preview(in: scope)
        if let last = activity?.last, let text = subtitle {
            if last.author == scope?.me { subtitle = "You: " + text }
            else if channel.kind != .direct, let scope { subtitle = scope.userName(last.author) + ": " + text }
        }
        let org = organization.flatMap { provider.organizations[$0] }
        self.key = key
        title = scope?.title(of: channel) ?? channel.id
        self.subtitle = subtitle
        place = org?.name
        date = activity?.last?.date
        unread = activity?.unread ?? 0
        isGroup = channel.kind != .direct
        isServerChannel = organization != nil
        icon = org?.icon ?? channel.icon
        network = accounts.style(for: provider.name)
    }
}

private struct ChatRow: View {
    let item: ChatItem

    var body: some View {
        HStack(spacing: 12) {
            Group {
                if item.isServerChannel {
                    IconImage(media: item.icon, size: 44, shape: RoundedRectangle(cornerRadius: 12, style: .continuous)) {
                        ServerTile(name: item.place ?? item.title, size: 44)
                    }
                } else {
                    IconImage(media: item.icon, size: 44, shape: Circle()) {
                        InitialsAvatar(name: item.title, size: 44)
                    }
                }
            }
            .overlay(alignment: .bottomTrailing) {
                NetworkBadge(style: item.network, size: 16)
                    .overlay(RoundedRectangle(cornerRadius: 5, style: .continuous).stroke(Color.appCanvas, lineWidth: 2))
                    .offset(x: 3, y: 3)
            }
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline) {
                    Text(item.isServerChannel ? "#\(item.title)" : item.title)
                        .font(.body.weight(item.unread > 0 ? .semibold : .regular)).lineLimit(1)
                    Spacer(minLength: 4)
                    if let date = item.date {
                        Text(date.chatListStamp).font(.caption)
                            .foregroundStyle(item.unread > 0 ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                    }
                }
                if item.isServerChannel, let place = item.place {
                    Text(place).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                HStack(alignment: .top) {
                    Text(item.subtitle ?? (item.isServerChannel ? "No recent messages" : item.isGroup ? "Group" : "Direct message"))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(item.isServerChannel ? 1 : 2)
                    Spacer(minLength: 4)
                    if item.unread > 0 { UnreadBadge(count: item.unread) }
                }
            }
        }
        .padding(.vertical, 4)
    }
}

struct ServerItem {
    var route: ChatRoute
    var name: String
    var icon: Media?
    /// The account, when there's more than one to tell apart.
    var account: String?
    var unread: Int
}

private struct ServerRow: View {
    let server: ServerItem

    var body: some View {
        HStack(spacing: 12) {
            IconImage(media: server.icon, size: 36, shape: RoundedRectangle(cornerRadius: 10, style: .continuous)) {
                ServerTile(name: server.name, size: 36)
            }
            VStack(alignment: .leading, spacing: 1) {
                Text(server.name).font(.body.weight(server.unread > 0 ? .semibold : .regular)).lineLimit(1)
                if let account = server.account {
                    Text(account).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            Spacer(minLength: 4)
            if server.unread > 0 { UnreadBadge(count: server.unread) }
        }
        .padding(.vertical, 2)
    }
}

// MARK: - Server

/// A server's channels under their categories, like Discord's channel list.
struct ServerView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var accounts
    let provider: String
    let organization: String
    @State private var search = ""

    private var org: OrganizationEntry? { store.providers[provider]?.organizations[organization] }

    /// A category and the channels under it; the first may have no category.
    private struct ChannelGroup: Identifiable {
        var id: String
        var title: String?
        var channels: [ChannelNode]
    }

    private func groups(_ org: OrganizationEntry) -> [ChannelGroup] {
        // Channels outside any category come first, as in Discord.
        var loose = ChannelGroup(id: "", title: nil, channels: [])
        var groups: [ChannelGroup] = []
        var inCategory = false
        for node in org.scope.channelTree() {
            let channel = node.channel
            if node.depth == 0 {
                inCategory = channel.isContainer
                if inCategory { groups.append(ChannelGroup(id: channel.id, title: org.scope.title(of: channel), channels: [])) }
            }
            guard !channel.isContainer,
                  search.isEmpty || org.scope.title(of: channel).localizedCaseInsensitiveContains(search) else { continue }
            if inCategory { groups[groups.count - 1].channels.append(node) } else { loose.channels.append(node) }
        }
        return ([loose] + groups).filter { !$0.channels.isEmpty }
    }

    var body: some View {
        Group {
            if let org {
                let groups = groups(org)
                List {
                    ForEach(groups) { group in
                        Section {
                            ForEach(group.channels) { node in
                                let key = ChannelKey(provider: provider, channel: ChannelRef(organization: organization, channel: node.channel.id))
                                NavigationLink(value: key) {
                                    ServerChannelRow(title: org.scope.title(of: node.channel), kind: node.channel.kind, unread: store.activity[key]?.unread ?? 0)
                                }
                                .listRowBackground(Color.appCanvas)
                                // Threads sit under their channel; a category's own children don't indent.
                                .padding(.leading, CGFloat(max(0, node.depth - (group.title == nil ? 0 : 1))) * 16)
                            }
                        } header: {
                            if let title = group.title { Text(title) }
                        }
                    }
                }
                .overlay {
                    if groups.isEmpty {
                        if search.isEmpty {
                            ContentUnavailableView("No Channels", systemImage: "number", description: Text("You can't see any channels in \(org.name)."))
                        } else {
                            ContentUnavailableView.search(text: search)
                        }
                    }
                }
                .scrollContentBackground(.hidden)
                .background(Color.appCanvas.ignoresSafeArea())
                .searchable(text: $search, prompt: "Search channels")
            } else {
                ContentUnavailableView("Server Unavailable", systemImage: "questionmark.square.dashed", description: Text("You may have left this server."))
            }
        }
        .navigationTitle(org?.name ?? "Server")
        .navigationBarTitleDisplayMode(.inline)
        .navigationDestination(for: ChannelKey.self) { ChatView(key: $0) }
    }
}

private struct ServerChannelRow: View {
    let title: String
    let kind: ChannelKind
    let unread: Int

    private var symbol: String {
        switch kind {
        case .voice: "speaker.wave.2"
        case .video: "video"
        case .announcement: "megaphone"
        case .thread: "text.bubble"
        case .direct, .group: "person.2"
        default: "number"
        }
    }

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: symbol).font(.subheadline).foregroundStyle(.secondary).frame(width: 22)
            Text(title).fontWeight(unread > 0 ? .semibold : .regular)
                .foregroundStyle(unread > 0 ? .primary : .secondary).lineLimit(1)
            Spacer(minLength: 4)
            if unread > 0 { UnreadBadge(count: unread) }
        }
    }
}

struct UnreadBadge: View {
    let count: Int
    var body: some View {
        Text(count > 99 ? "99+" : "\(count)")
            .font(.caption2.bold()).monospacedDigit()
            .foregroundStyle(.white)
            .padding(.horizontal, 7).padding(.vertical, 3)
            .frame(minWidth: 22)
            .background(Color.accentColor, in: Capsule())
    }
}
