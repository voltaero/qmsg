import SwiftUI
import QmsgKit

/// One inbox across accounts: recent chats first, then each account's direct
/// messages and servers.
struct ChatsView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var accounts
    @Environment(AppRouter.self) private var router
    @State private var selected: ChannelKey?
    @State private var composing = false
    @State private var search = ""
    @State private var filter: String?
    @State private var expanded: Set<String> = []

    private var providers: [ProviderState] {
        store.sortedProviders.filter { filter == nil || $0.name == filter }
    }

    var body: some View {
        NavigationSplitView {
            List(selection: $selected) {
                attentionRows
                if store.providers.count > 1 && search.isEmpty {
                    filterChips
                }
                let recent = recentChats
                if !recent.isEmpty {
                    Section("Recent") {
                        ForEach(recent, id: \.key) { item in
                            NavigationLink(value: item.key) { ChatRow(item: item) }
                        }
                    }
                }
                ForEach(providers) { provider in
                    accountSection(provider)
                }
            }
            .listStyle(.plain)
            .safeAreaInset(edge: .top, spacing: 0) {
                ConnectionBanner(link: store.link) { store.resume() }
            }
            .overlay { emptyState }
            .navigationTitle("Chats")
            .searchable(text: $search, prompt: "Search chats")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button("New chat", systemImage: "square.and.pencil") { composing = true }
                        .disabled(store.providers.isEmpty)
                }
            }
        } detail: {
            if let selected {
                if store.providers[selected.provider]?.channel(selected.channel)?.isMailbox == true {
                    // Mail pushes messages on its own stack.
                    NavigationStack { MailboxView(key: selected) }.id(selected)
                } else {
                    ChatView(key: selected).id(selected)
                }
            } else {
                ContentUnavailableView("Choose a chat", systemImage: "bubble.left.and.bubble.right", description: Text("Pick a conversation to read and reply."))
            }
        }
        .sheet(isPresented: $composing) { NavigationStack { NewChatView { selected = $0; composing = false } } }
    }

    // MARK: Sections

    @ViewBuilder
    private var attentionRows: some View {
        let waiting = accounts.items(store).filter { $0.health.needsUser }
        if !waiting.isEmpty && search.isEmpty {
            Section {
                ForEach(waiting) { item in
                    Button { router.show(account: item.name) } label: {
                        HStack(spacing: 12) {
                            NetworkBadge(style: item.style, size: 30)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(item.live?.status.loginStep != nil ? "Sign in to \(item.label)" : "\(item.label): \(item.health.title.lowercased())")
                                    .font(.subheadline.weight(.semibold))
                                if let detail = item.health.detail {
                                    Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                                }
                            }
                            Spacer(minLength: 0)
                            Image(systemName: "chevron.right").font(.caption.bold()).foregroundStyle(.tertiary)
                        }
                    }
                    .buttonStyle(.plain)
                    .listRowBackground(item.health.color.opacity(0.1))
                }
            }
        }
    }

    private var filterChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                chip("All", style: nil, selected: filter == nil) { filter = nil }
                ForEach(store.sortedProviders) { provider in
                    chip(accounts.label(for: provider.name), style: accounts.style(for: provider.name), selected: filter == provider.name) {
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
    }

    @ViewBuilder
    private func accountSection(_ provider: ProviderState) -> some View {
        let direct = provider.standalone.channelTree().map(\.channel)
            .filter { !$0.isContainer && matches(provider.standalone.title(of: $0)) }
            .map { chatItem(provider, organization: nil, channel: $0) }
            .sorted { ($0.date ?? .distantPast, $1.title) > ($1.date ?? .distantPast, $0.title) }
        let servers = provider.sortedOrganizations.filter { org in
            search.isEmpty || matches(org.name) || org.scope.channels.values.contains { matches(org.scope.title(of: $0)) }
        }
        if !direct.isEmpty || !servers.isEmpty || (search.isEmpty && provider.status.loginStep == nil) {
            Section {
                ForEach(direct, id: \.key) { item in
                    NavigationLink(value: item.key) { ChatRow(item: item) }
                }
                ForEach(servers) { org in
                    if org.scope.channels.values.contains(where: \.isMailbox) {
                        mailGroup(provider, org)
                    } else {
                        serverGroup(provider, org)
                    }
                }
                if direct.isEmpty && servers.isEmpty {
                    Text(provider.status.isReady ? "No chats yet" : AccountHealth(status: provider.status, enabled: true).title + "…")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            } header: {
                HStack(spacing: 8) {
                    NetworkBadge(style: accounts.style(for: provider.name), size: 18)
                    Text(accounts.label(for: provider.name))
                }
            }
        }
    }

    private func serverGroup(_ provider: ProviderState, _ org: OrganizationEntry) -> some View {
        let id = provider.name + "\n" + org.id
        let open = !search.isEmpty || expanded.contains(id)
        let unread = store.activity.reduce(0) { total, entry in
            entry.key.provider == provider.name && entry.key.channel.organization == org.id ? total + entry.value.unread : total
        }
        return DisclosureGroup(isExpanded: Binding(get: { open }, set: { open in
            if open { expanded.insert(id) } else { expanded.remove(id) }
        })) {
            // Only build a server's channel list while it's open.
            if open {
                ForEach(org.scope.channelTree().filter { search.isEmpty || $0.channel.isContainer || matches(org.scope.title(of: $0.channel)) }) { node in
                    let channel = node.channel
                    if channel.isContainer {
                        if search.isEmpty {
                            Text(org.scope.title(of: channel).uppercased())
                                .font(.caption2.weight(.semibold)).foregroundStyle(.tertiary)
                                .padding(.leading, CGFloat(node.depth) * 10).padding(.top, 2)
                                .listRowSeparator(.hidden)
                        }
                    } else {
                        let key = ChannelKey(provider: provider.name, channel: ChannelRef(organization: org.id, channel: channel.id))
                        NavigationLink(value: key) {
                            ServerChannelRow(title: org.scope.title(of: channel), kind: channel.kind, unread: store.activity[key]?.unread ?? 0)
                        }
                        .padding(.leading, CGFloat(node.depth) * 10)
                        .listRowSeparator(.hidden)
                    }
                }
            }
        } label: {
            HStack(spacing: 10) {
                ServerTile(name: org.name, size: 26)
                Text(org.name).font(.subheadline.weight(unread > 0 ? .semibold : .regular)).lineLimit(1)
                Spacer(minLength: 0)
                if unread > 0 { UnreadBadge(count: unread) }
            }
        }
    }

    /// A mail account's folders, like Mail's sidebar: Inbox and the special
    /// folders, then the rest folded away.
    @ViewBuilder
    private func mailGroup(_ provider: ProviderState, _ org: OrganizationEntry) -> some View {
        let folders = org.scope.mailboxes.filter { matches($0.mailboxName) || matches(org.name) }
        let main = folders.filter { MailboxRole($0) != .other }
        let others = folders.filter { MailboxRole($0) == .other }
        if org.name != accounts.label(for: provider.name) {
            Text(org.name).font(.footnote).foregroundStyle(.secondary).lineLimit(1)
                .listRowSeparator(.hidden)
        }
        ForEach(main) { folder in mailboxLink(provider, org, folder) }
        if !others.isEmpty {
            let id = provider.name + "\n" + org.id
            DisclosureGroup("Folders", isExpanded: Binding(get: { !search.isEmpty || expanded.contains(id) }, set: { open in
                if open { expanded.insert(id) } else { expanded.remove(id) }
            })) {
                ForEach(others) { folder in mailboxLink(provider, org, folder) }
            }
        }
    }

    private func mailboxLink(_ provider: ProviderState, _ org: OrganizationEntry, _ folder: Channel) -> some View {
        let key = ChannelKey(provider: provider.name, channel: ChannelRef(organization: org.id, channel: folder.id))
        let role = MailboxRole(folder)
        // Unread mail among what has been loaded; the server keeps no count.
        let unread = role == .inbox ? store.conversation(key).messages.reduce(0) { $0 + ($1.email?.read == false ? 1 : 0) } : 0
        return NavigationLink(value: key) {
            HStack(spacing: 10) {
                Image(systemName: role.symbol).foregroundStyle(.tint).frame(width: 24)
                Text(folder.mailboxName).lineLimit(1)
                Spacer(minLength: 4)
                if unread > 0 { Text("\(unread)").font(.subheadline).foregroundStyle(.secondary).monospacedDigit() }
            }
        }
    }

    @ViewBuilder
    private var emptyState: some View {
        if store.providers.isEmpty {
            if store.link == .live {
                ContentUnavailableView {
                    Label("No accounts connected", systemImage: "bubble.left.and.bubble.right")
                } description: {
                    Text("Add an account to bring your chats here.")
                } actions: {
                    Button("Add account") { router.tab = .accounts }.buttonStyle(.borderedProminent)
                }
            } else {
                ProgressView("Loading your chats…")
            }
        } else if !search.isEmpty && recentChats.isEmpty && providers.allSatisfy({ provider in
            !provider.standalone.channels.values.contains { matches(provider.standalone.title(of: $0)) }
                && !provider.organizations.values.contains { org in matches(org.name) || org.scope.channels.values.contains { matches(org.scope.title(of: $0)) } }
        }) {
            ContentUnavailableView.search(text: search)
        }
    }

    // MARK: Data

    private var recentChats: [ChatItem] {
        guard search.isEmpty else { return [] }
        return store.recentKeys.compactMap { key -> ChatItem? in
            guard filter == nil || key.provider == filter,
                  let provider = store.providers[key.provider],
                  let channel = provider.channel(key.channel), !channel.isMailbox else { return nil }
            return chatItem(provider, organization: key.channel.organization, channel: channel)
        }
    }

    private func chatItem(_ provider: ProviderState, organization: String?, channel: Channel) -> ChatItem {
        let key = ChannelKey(provider: provider.name, channel: ChannelRef(organization: organization, channel: channel.id))
        let scope = provider.scope(organization)
        let activity = store.activity[key]
        var subtitle = activity?.last?.preview(in: scope)
        if let last = activity?.last, let subtitleText = subtitle {
            if last.author == scope?.me { subtitle = "You: " + subtitleText }
            else if channel.kind != .direct, let scope { subtitle = scope.userName(last.author) + ": " + subtitleText }
        }
        let place = organization.flatMap { provider.organizations[$0]?.name }
        return ChatItem(
            key: key,
            title: (scope?.title(of: channel) ?? channel.id),
            subtitle: subtitle,
            place: place,
            date: activity?.last?.date,
            unread: activity?.unread ?? 0,
            isGroup: channel.kind != .direct,
            isServerChannel: organization != nil,
            network: accounts.style(for: provider.name)
        )
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
    var network: NetworkStyle
}

private struct ChatRow: View {
    let item: ChatItem

    var body: some View {
        HStack(spacing: 12) {
            Group {
                if item.isServerChannel { ServerTile(name: item.place ?? item.title, size: 40) }
                else { InitialsAvatar(name: item.title, size: 40) }
            }
                .overlay(alignment: .bottomTrailing) {
                    NetworkBadge(style: item.network, size: 16)
                        .overlay(RoundedRectangle(cornerRadius: 5, style: .continuous).stroke(Color(uiColor: .systemBackground), lineWidth: 2))
                        .offset(x: 3, y: 3)
                }
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline) {
                    Text(item.isServerChannel ? "#\(item.title)" : item.title)
                        .font(.body.weight(item.unread > 0 ? .semibold : .regular)).lineLimit(1)
                    Spacer(minLength: 4)
                    if let date = item.date {
                        Text(date.chatListStamp).font(.caption).foregroundStyle(item.unread > 0 ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                    }
                }
                HStack(alignment: .top) {
                    Text(item.subtitle ?? item.place ?? (item.isGroup ? "Group" : "Direct message"))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    if item.unread > 0 { UnreadBadge(count: item.unread) }
                }
            }
        }
        .padding(.vertical, 4)
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
        HStack(spacing: 8) {
            Image(systemName: symbol).font(.footnote).foregroundStyle(.tertiary).frame(width: 18)
            Text(title).font(.subheadline).fontWeight(unread > 0 ? .semibold : .regular)
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
