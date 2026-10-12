import SwiftUI
import UIKit
import UniformTypeIdentifiers
import WebKit
import QmsgKit

extension MailboxRole {
    var symbol: String {
        switch self {
        case .inbox: "tray"
        case .tab: "tag"
        case .starred: "star"
        case .important: "exclamationmark.circle"
        case .drafts: "doc"
        case .sent: "paperplane"
        case .archive: "archivebox"
        case .all: "tray.2"
        case .junk: "xmark.bin"
        case .trash: "trash"
        case .other: "folder"
        }
    }
}

/// Calls a mail action, such as `star`, on one message. The email provider
/// offers the same set for every message in a folder.
@MainActor
private func runMailAction(_ id: String, on message: Message, key: ChannelKey, functions: [Function], arguments: Arguments = [:], store: QmsgStore) async throws {
    guard let function = functions.first(where: { $0.id == id }) else {
        throw StoreError.unsupported("This mail account can't do that.")
    }
    _ = try await store.call(function, context: .message(channel: key.channel, id: message.id), arguments: arguments, provider: key.provider).get()
}

// MARK: - Mailboxes

/// The Mail tab: each mail account's folders, like Mail's Mailboxes, with
/// the chosen folder's messages beside or above them.
struct MailView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var accounts
    @Environment(AppRouter.self) private var router
    @State private var selection: ChannelKey?
    /// Inbox opens by itself once, as Mail does; going back stays back.
    @State private var openedInbox = false
    @State private var composing: ChannelKey?
    @State private var expanded: Set<String> = []

    private var mailProviders: [ProviderState] {
        store.sortedProviders.filter { accounts.isMail($0) }
    }

    private struct MailAccount: Identifiable {
        var provider: ProviderState
        var org: OrganizationEntry
        var id: String { provider.name + "\n" + org.id }
        var inbox: ChannelKey? {
            org.scope.mailboxes.first { MailboxRole($0) == .inbox }
                .map { ChannelKey(provider: provider.name, channel: ChannelRef(organization: org.id, channel: $0.id)) }
        }
    }

    private var mailAccounts: [MailAccount] {
        mailProviders.flatMap { provider in
            provider.sortedOrganizations
                .filter { $0.scope.channels.values.contains(where: \.isMailbox) }
                .map { MailAccount(provider: provider, org: $0) }
        }
    }

    var body: some View {
        NavigationSplitView {
            List(selection: $selection) {
                ForEach(mailProviders) { provider in
                    let orgs = mailAccounts.filter { $0.provider.name == provider.name }
                    if orgs.isEmpty {
                        Section(accounts.label(for: provider.name)) { accountStatus(provider) }
                    }
                    ForEach(orgs) { account in
                        Section {
                            folders(account)
                        } header: {
                            Text(accounts.label(for: provider.name))
                        } footer: {
                            if account.org.name != accounts.label(for: provider.name) && orgs.count == 1 {
                                Text(account.org.name)
                            }
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .background(Color.appCanvas.ignoresSafeArea())
            .safeAreaInset(edge: .top, spacing: 0) {
                ConnectionBanner(link: store.link) { store.resume() }
            }
            .navigationTitle("Mailboxes")
            .toolbar {
                ToolbarItem(placement: .primaryAction) { composeButton }
            }
        } detail: {
            NavigationStack {
                if let selection {
                    MailboxView(key: selection)
                } else {
                    ContentUnavailableView("No Mailbox Selected", systemImage: "tray", description: Text("Choose a mailbox to see its mail."))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.appCanvas.ignoresSafeArea())
            .id(selection)
        }
        .sheet(isPresented: Binding(get: { composing != nil }, set: { if !$0 { composing = nil } })) {
            if let key = composing {
                NavigationStack { MailComposeView(provider: key.provider, mode: .new(.channel(key.channel))) }
            }
        }
        .task(id: mailAccounts.first?.inbox) {
            guard !openedInbox, selection == nil, let inbox = mailAccounts.first?.inbox else { return }
            openedInbox = true
            selection = inbox
        }
    }

    @ViewBuilder
    private var composeButton: some View {
        let senders = mailAccounts.compactMap { account in account.inbox.map { (account, $0) } }
        if senders.count > 1 {
            Menu("New Message", systemImage: "square.and.pencil") {
                ForEach(senders, id: \.0.id) { account, inbox in
                    Button(accounts.label(for: account.provider.name)) { composing = inbox }
                }
            }
        } else {
            Button("New Message", systemImage: "square.and.pencil") { composing = senders.first?.1 }
                .disabled(senders.isEmpty)
        }
    }

    /// Inbox and the special folders, then the rest folded under "More Folders".
    @ViewBuilder
    private func folders(_ account: MailAccount) -> some View {
        let all = account.org.scope.mailboxes
        let main = all.filter { MailboxRole($0) != .other }
        let others = all.filter { MailboxRole($0) == .other }
        ForEach(main) { folder in folderRow(account, folder) }
        if !others.isEmpty {
            DisclosureGroup("More Folders", isExpanded: Binding(get: { expanded.contains(account.id) }, set: { open in
                if open { expanded.insert(account.id) } else { expanded.remove(account.id) }
            })) {
                ForEach(others) { folder in folderRow(account, folder) }
            }
        }
    }

    private func folderRow(_ account: MailAccount, _ folder: Channel) -> some View {
        let key = ChannelKey(provider: account.provider.name, channel: ChannelRef(organization: account.org.id, channel: folder.id))
        let role = MailboxRole(folder)
        // Unread mail among what has been loaded; the server keeps no count.
        let unread = role == .inbox ? store.conversation(key).messages.reduce(0) { $0 + ($1.email?.read == false ? 1 : 0) } : 0
        return NavigationLink(value: key) {
            Label {
                HStack {
                    Text(folder.mailboxName).lineLimit(1)
                    Spacer(minLength: 4)
                    if unread > 0 { Text("\(unread)").foregroundStyle(.secondary).monospacedDigit() }
                }
            } icon: {
                Image(systemName: role.symbol).foregroundStyle(.tint)
            }
        }
    }

    /// A mail account whose folders haven't arrived: why, and what to do.
    @ViewBuilder
    private func accountStatus(_ provider: ProviderState) -> some View {
        let health = AccountHealth(status: provider.status, enabled: true)
        if health.needsUser {
            Button { router.show(account: provider.name) } label: {
                HStack {
                    Label(provider.status.loginStep != nil ? "Sign in to see your mail" : health.title, systemImage: "exclamationmark.circle.fill")
                        .foregroundStyle(health.color)
                    Spacer()
                    Image(systemName: "chevron.right").font(.caption.bold()).foregroundStyle(.tertiary)
                }
            }
            .buttonStyle(.plain)
        } else {
            HStack {
                Text(health.tone == .good ? "No folders" : "\(health.title)…").foregroundStyle(.secondary)
                Spacer()
                if health.tone == .busy { ProgressView().controlSize(.small) }
            }
        }
    }
}

// MARK: - Folder

/// A mail folder as a list, newest first, like Mail's message list.
struct MailboxView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var accounts
    let key: ChannelKey
    @State private var opened: String?
    @State private var search = ""
    @State private var composing = false
    @State private var functions: [Function] = []
    @State private var error: String?

    private var scope: Scope? { store.providers[key.provider]?.scope(key.channel.organization) }
    private var channel: Channel? { scope?.channels[key.channel.channel] }
    private var conversation: Conversation { store.conversation(key) }
    private var role: MailboxRole { channel.map(MailboxRole.init) ?? .other }
    private var gmailInbox: Bool {
        role == .inbox && accounts.item(key.provider, store)?.managed?.settings["service"] == "gmail"
    }
    private var mailboxTitle: String { channel?.mailboxName ?? key.channel.channel }
    private var mailboxSubtitle: String {
        let identity = store.providers[key.provider]?.accountIdentity
        return [gmailInbox ? "All categories" : nil, identity].compactMap { $0 }.joined(separator: " · ")
    }

    private var mail: [Message] {
        let newest = conversation.messages.reversed()
        guard !search.isEmpty else { return Array(newest) }
        return newest.filter { message in
            let people = (message.email?.from ?? []) + (message.email?.to ?? [])
            let fields = [message.email?.subject ?? "", message.mailBody] + people.flatMap { [$0.displayName, $0.address] }
            return fields.contains { $0.localizedCaseInsensitiveContains(search) }
        }
    }

    var body: some View {
        List {
            Group { rows }
                // Every row, including errors and "Load more", or plain
                // lists show the default black rows in dark mode.
                .listRowBackground(Color.appCanvas)
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .overlay { emptyState }
        .overlay(alignment: .top) { ConnectionBanner(link: store.link) { store.resume() } }
        .refreshable { await store.loadLatest(key) }
        .searchable(text: $search, prompt: "Search loaded mail")
        .navigationTitle(mailboxTitle)
        .background(Color.appCanvas.ignoresSafeArea())
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                VStack(spacing: 1) {
                    Text(mailboxTitle).font(.headline).lineLimit(1)
                    if !mailboxSubtitle.isEmpty {
                        Text(mailboxSubtitle).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                .accessibilityElement(children: .combine)
            }
            ToolbarItem(placement: .primaryAction) {
                Button("New message", systemImage: "square.and.pencil") { composing = true }
            }
        }
        .navigationDestination(item: $opened) { id in
            MailMessageView(key: key, id: id, functions: functions)
        }
        .sheet(isPresented: $composing) {
            NavigationStack { MailComposeView(provider: key.provider, mode: .new(.channel(key.channel))) }
        }
        .task {
            store.visible = key
            await store.loadLatest(key)
        }
        .task(id: conversation.messages.first?.id) { await loadFunctions() }
        .onDisappear { if store.visible == key { store.visible = nil } }
    }

    @ViewBuilder
    private var rows: some View {
        if let error = conversation.loadError {
            VStack(alignment: .leading, spacing: 8) {
                ErrorNote(message: error)
                Button("Try again") { Task { await store.loadLatest(key) } }
            }
        }
        if let error { ErrorNote(message: error) { self.error = nil } }
        ForEach(mail) { message in
            Button { opened = message.id } label: {
                MailRow(message: message, showsRecipients: role.showsRecipients)
            }
            .buttonStyle(.plain)
            .swipeActions(edge: .leading) {
                if let email = message.email {
                    Button(email.read ? "Unread" : "Read", systemImage: email.read ? "envelope.badge" : "envelope.open") {
                        act(email.read ? "mark_unread" : "mark_read", message)
                    }.tint(.blue)
                }
            }
            .swipeActions(edge: .trailing) {
                if functions.contains(where: { $0.id == "trash" }) {
                    Button("Trash", systemImage: "trash", role: .destructive) { act("trash", message) }
                }
                if let email = message.email {
                    Button(email.starred ? "Unstar" : "Star", systemImage: email.starred ? "star.slash" : "star") {
                        act(email.starred ? "unstar" : "star", message)
                    }.tint(.orange)
                }
            }
        }
        if search.isEmpty, !conversation.reachedStart, conversation.olderCursor != nil, !conversation.messages.isEmpty {
            HStack {
                Spacer()
                if conversation.isLoadingOlder { ProgressView() }
                else { Button("Load more") { Task { await store.loadOlder(key) } }.buttonStyle(.borderless) }
                Spacer()
            }
            .listRowSeparator(.hidden)
            .task { await store.loadOlder(key) }
        }
    }

    @ViewBuilder
    private var emptyState: some View {
        if conversation.messages.isEmpty {
            if conversation.isLoading || !conversation.loaded {
                if conversation.loadError == nil { ProgressView("Loading mail…") }
            } else {
                ContentUnavailableView("No mail", systemImage: role.symbol, description: Text("\(channel?.mailboxName ?? "This folder") is empty."))
            }
        } else if !search.isEmpty && mail.isEmpty {
            ContentUnavailableView.search(text: search)
        }
    }

    /// Message actions are the same for each message in a folder; ask once.
    private func loadFunctions() async {
        guard functions.isEmpty, let first = conversation.messages.first else { return }
        functions = (try? await store.functions(.message(channel: key.channel, id: first.id), provider: key.provider)) ?? []
    }

    private func act(_ id: String, _ message: Message) {
        Task {
            do { try await runMailAction(id, on: message, key: key, functions: functions, store: store) }
            catch { self.error = QmsgStore.describe(error) }
        }
    }
}

/// Sender, date, subject and a two-line preview, with an unread dot.
private struct MailRow: View {
    let message: Message
    let showsRecipients: Bool

    private var email: Email? { message.email }
    private var unread: Bool { email?.read == false }

    private var people: String {
        guard let email else { return message.author }
        if showsRecipients {
            let to = email.to.map(\.displayName)
            return to.isEmpty ? "No recipients" : "To: " + to.joined(separator: ", ")
        }
        return email.from.first?.displayName ?? message.author
    }

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Circle().fill(unread ? Color.accentColor : .clear).frame(width: 10, height: 10).padding(.top, 5)
                .accessibilityLabel(unread ? "Unread" : "")
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(people).font(.body.weight(unread ? .semibold : .regular)).lineLimit(1)
                    Spacer(minLength: 4)
                    Text(message.date.chatListStamp).font(.subheadline).foregroundStyle(.secondary)
                }
                HStack(spacing: 4) {
                    if email?.draft == true { Text("Draft").foregroundStyle(.red) }
                    Text(email.map { $0.subject.isEmpty ? "(No subject)" : $0.subject } ?? "").lineLimit(1)
                    Spacer(minLength: 4)
                    if !message.mailAttachments.isEmpty { Image(systemName: "paperclip").foregroundStyle(.secondary) }
                    if email?.starred == true { Image(systemName: "star.fill").foregroundStyle(.orange) }
                }
                .font(.subheadline.weight(unread ? .medium : .regular))
                Text(message.mailBody.replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression))
                    .font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
            }
        }
        .padding(.vertical, 4)
        .contentShape(Rectangle())
    }
}

// MARK: - Message

struct MailMessageView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let key: ChannelKey
    let id: String
    let functions: [Function]
    @State private var loaded: [Function] = []
    @State private var details = false
    @State private var plainText = false
    @State private var replying = false
    @State private var busy = false
    @State private var error: String?
    @State private var markedRead = false
    @State private var loadingBody = false

    private var message: Message? { store.conversation(key).message(id) }
    private var scope: Scope? { store.providers[key.provider]?.scope(key.channel.organization) }
    private var actions: [Function] { loaded.isEmpty ? functions : loaded }
    private func has(_ id: String) -> Bool { actions.contains { $0.id == id } }

    var body: some View {
        Group {
            if let message, let email = message.email {
                content(message, email)
            } else {
                ContentUnavailableView("Message unavailable", systemImage: "envelope", description: Text("It was moved or deleted."))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.appCanvas.ignoresSafeArea())
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { if let message, let email = message.email { toolbar(message, email) } }
        // The message's own actions take the bottom, as in Mail.
        .toolbar(.hidden, for: .tabBar)
        .sheet(isPresented: $replying) {
            if let message {
                NavigationStack { MailComposeView(provider: key.provider, mode: .reply(message, key.channel)) }
            }
        }
        .task {
            await loadBody()
            if loaded.isEmpty {
                loaded = (try? await store.functions(.message(channel: key.channel, id: id), provider: key.provider)) ?? []
            }
            // Opening a mail reads it, as in Mail.
            if let message, message.email?.read == false, !markedRead {
                markedRead = true
                try? await runMailAction("mark_read", on: message, key: key, functions: actions, store: store)
            }
        }
    }

    private func content(_ message: Message, _ email: Email) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                if let error { ErrorNote(message: error) { self.error = nil } }
                Text(email.subject.isEmpty ? "(No subject)" : email.subject)
                    .font(.title2.bold()).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                header(message, email)
                Divider()
                if message.mailNeedsBody {
                    if loadingBody { ProgressView("Loading message…").frame(maxWidth: .infinity).padding(.vertical, 24) }
                    else { Button("Try loading again") { Task { await loadBody() } } }
                } else if let html = email.html, !plainText {
                    MailHTMLView(message: message, html: html)
                } else {
                    Text(MailQuote.styled(message.mailBody)).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                let files = message.mailAttachments
                if !files.isEmpty {
                    Divider()
                    Text(files.count == 1 ? "1 Attachment" : "\(files.count) Attachments")
                        .font(.footnote.weight(.semibold)).foregroundStyle(.secondary)
                    ForEach(Array(files.enumerated()), id: \.offset) { _, media in
                        MediaView(media: media)
                    }
                }
            }
            .padding(.horizontal, 16).padding(.vertical, 12)
        }
    }

    private func header(_ message: Message, _ email: Email) -> some View {
        let sender = email.from.first
        return VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: 12) {
                InitialsAvatar(name: sender?.displayName ?? message.author, size: 40)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(alignment: .firstTextBaseline) {
                        Text(sender?.displayName ?? message.author).font(.headline).lineLimit(1)
                        Spacer(minLength: 4)
                        Text(message.date.chatListStamp).font(.subheadline).foregroundStyle(.secondary)
                    }
                    Button {
                        withAnimation(.snappy) { details.toggle() }
                    } label: {
                        HStack(spacing: 4) {
                            Text("To: " + (email.to.isEmpty ? "undisclosed recipients" : email.to.map(\.displayName).joined(separator: ", ")))
                                .lineLimit(1)
                            Image(systemName: details ? "chevron.up" : "chevron.down").font(.caption2.bold())
                        }
                        .font(.subheadline).foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .accessibilityHint(details ? "Hides details" : "Shows all addresses")
                }
            }
            if details {
                VStack(alignment: .leading, spacing: 4) {
                    addressLine("From", email.from)
                    addressLine("Reply-To", email.replyTo)
                    addressLine("To", email.to)
                    addressLine("Cc", email.cc)
                    addressLine("Bcc", email.bcc)
                    LabeledContent("Date") { Text(message.date.formatted(date: .long, time: .shortened)) }
                }
                .font(.footnote)
                .padding(10)
                .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .textSelection(.enabled)
            }
        }
    }

    @ViewBuilder
    private func addressLine(_ label: String, _ addresses: [EmailAddress]) -> some View {
        if !addresses.isEmpty {
            LabeledContent(label) {
                VStack(alignment: .trailing, spacing: 1) {
                    ForEach(addresses, id: \.self) { address in
                        Text(address.name.map { "\($0) <\(address.address)>" } ?? address.address)
                            .multilineTextAlignment(.trailing)
                    }
                }
            }
        }
    }

    @ToolbarContentBuilder
    private func toolbar(_ message: Message, _ email: Email) -> some ToolbarContent {
        ToolbarItem(placement: .primaryAction) {
            Menu("More", systemImage: "ellipsis.circle") {
                if has("mark_unread") && email.read {
                    Button("Mark as Unread", systemImage: "envelope.badge") { act("mark_unread", message, leave: true) }
                }
                if email.html != nil && !message.mailBody.isEmpty {
                    Toggle("Show Plain Text", systemImage: "text.alignleft", isOn: $plainText)
                }
                Button("Copy Subject", systemImage: "doc.on.doc") { UIPasteboard.general.string = email.subject }
            }
            .disabled(busy)
        }
        // Mail's order: destructive on the left, reply on the right.
        ToolbarItemGroup(placement: .bottomBar) {
            if has("trash") {
                Button("Trash", systemImage: "trash") { act("trash", message, leave: true) }
                    .disabled(busy)
            }
            Spacer()
            // Tabs are views of Inbox, so mail can't move into one or from
            // one to Inbox.
            let inTab = scope?.channels[key.channel.channel].map { MailboxRole($0) == .tab } ?? false
            let folders = (scope?.mailboxes ?? []).filter {
                $0.id != key.channel.channel && MailboxRole($0) != .tab && !(inTab && MailboxRole($0) == .inbox)
            }
            if has("move") && !folders.isEmpty {
                Menu("Move", systemImage: "folder") {
                    ForEach(folders) { folder in
                        Button(folder.mailboxName, systemImage: MailboxRole(folder).symbol) {
                            act("move", message, arguments: ["mailbox": .text(folder.id)], leave: true)
                        }
                    }
                }
                .disabled(busy)
                Spacer()
            }
            if has(email.starred ? "unstar" : "star") {
                Button(email.starred ? "Unstar" : "Star", systemImage: email.starred ? "star.fill" : "star") {
                    act(email.starred ? "unstar" : "star", message)
                }
                .disabled(busy)
                Spacer()
            }
            if has("reply") {
                Button("Reply", systemImage: "arrowshape.turn.up.left") { replying = true }
                    .disabled(busy)
            }
        }
    }

    private func act(_ id: String, _ message: Message, arguments: Arguments = [:], leave: Bool = false) {
        busy = true
        error = nil
        Task {
            defer { busy = false }
            do {
                try await runMailAction(id, on: message, key: key, functions: actions, arguments: arguments, store: store)
                if leave { dismiss() }
            } catch { self.error = QmsgStore.describe(error) }
        }
    }

    private func loadBody() async {
        guard message?.mailNeedsBody == true else { return }
        loadingBody = true
        error = nil
        defer { loadingBody = false }
        do { try await store.loadMailBody(key, id: id) }
        catch { self.error = QmsgStore.describe(error) }
    }
}

/// Dims quoted lines ("> ...") in plain text mail.
private enum MailQuote {
    static func styled(_ text: String) -> AttributedString {
        var result = AttributedString()
        for (index, line) in text.split(separator: "\n", omittingEmptySubsequences: false).enumerated() {
            var part = AttributedString((index > 0 ? "\n" : "") + line)
            if line.hasPrefix(">") { part.foregroundColor = .secondary }
            result += part
        }
        return result
    }
}

// MARK: - HTML

/// Mail HTML in a locked-down web view: no scripts, no network, no remote
/// images, links open outside. `cid:` images come from the message itself.
private struct MailHTMLView: View {
    @Environment(QmsgStore.self) private var store
    let message: Message
    let html: String
    @State private var document: String?
    @State private var height: CGFloat = 60

    /// Blocks every load except inline data, whatever the mail's own markup says.
    private static let head = """
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:; form-action 'none'">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <meta name="color-scheme" content="light only">
    <style>
    html, body { margin: 0; padding: 0; background: #fff; color: #000; overflow: hidden; -webkit-text-size-adjust: 100%; }
    body { font: -apple-system-body; overflow-wrap: anywhere; }
    img { max-width: 100%; height: auto; }
    pre { white-space: pre-wrap; }
    blockquote { margin-left: 0.5em; padding-left: 0.8em; border-left: 2px solid #ccc; color: #555; }
    </style>
    """

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let document {
                Web(document: document, height: $height)
                    .frame(height: height)
                    .padding(8)
                    .background(.white, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            } else {
                ProgressView().frame(maxWidth: .infinity, minHeight: 60)
            }
            if html.range(of: #"src\s*=\s*["']?\s*(https?:)?//"#, options: [.regularExpression, .caseInsensitive]) != nil {
                Label("Remote images are blocked for privacy.", systemImage: "eye.slash")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .task(id: message) { document = await build() }
    }

    private func build() async -> String {
        var body = html
        for (index, cid) in message.inlineImages.sorted(by: { $0.key < $1.key }) {
            guard let media = message.content[index].media,
                  let data = try? await store.data(for: media) else { continue }
            let uri = "data:\(media.mime ?? "image/png");base64,\(data.base64EncodedString())"
            body = body.replacingOccurrences(of: "cid:" + cid, with: uri, options: .caseInsensitive)
        }
        return Self.head + "<div id=\"qmsg-mail\">" + body + "</div>"
    }

    private struct Web: UIViewRepresentable {
        let document: String
        @Binding var height: CGFloat

        func makeCoordinator() -> Coordinator { Coordinator(height: $height) }

        func makeUIView(context: Context) -> WKWebView {
            let configuration = WKWebViewConfiguration()
            configuration.websiteDataStore = .nonPersistent()
            configuration.defaultWebpagePreferences.allowsContentJavaScript = false
            configuration.dataDetectorTypes = [.link, .phoneNumber]
            let view = WKWebView(frame: .zero, configuration: configuration)
            view.navigationDelegate = context.coordinator
            view.isOpaque = false
            view.backgroundColor = .white
            view.scrollView.isScrollEnabled = false
            view.scrollView.bounces = false
            context.coordinator.observe(view)
            return view
        }

        func updateUIView(_ view: WKWebView, context: Context) {
            context.coordinator.height = $height
            guard context.coordinator.document != document else { return }
            context.coordinator.document = document
            view.loadHTMLString(document, baseURL: nil)
        }

        static func dismantleUIView(_ view: WKWebView, coordinator: Coordinator) {
            coordinator.observation = nil
        }

        @MainActor
        final class Coordinator: NSObject, WKNavigationDelegate {
            var height: Binding<CGFloat>
            var document: String?
            var observation: NSKeyValueObservation?
            private var width: CGFloat = 0

            init(height: Binding<CGFloat>) { self.height = height }

            /// Re-fit when the width changes, such as on rotation.
            func observe(_ view: WKWebView) {
                observation = view.observe(\.bounds, options: [.new]) { [weak self] view, _ in
                    Task { @MainActor in
                        guard let self, abs(view.bounds.width - self.width) > 1 else { return }
                        self.width = view.bounds.width
                        self.fit(view)
                    }
                }
            }

            /// Scales wide mail down to the screen, like Mail, then sizes the
            /// view to the content. This runs in the app's own script world,
            /// which works with the page's scripts off.
            func fit(_ view: WKWebView) {
                let script = """
                var root = document.getElementById('qmsg-mail');
                if (!root) { 0 } else {
                  root.style.transform = ''; root.style.width = '';
                  var room = document.documentElement.clientWidth;
                  var wide = Math.max(root.scrollWidth, root.offsetWidth);
                  var scale = 1;
                  if (room > 0 && wide > room + 1) {
                    scale = room / wide;
                    root.style.width = wide + 'px';
                    root.style.transformOrigin = '0 0';
                    root.style.transform = 'scale(' + scale + ')';
                  }
                  root.offsetHeight * scale;
                }
                """
                view.evaluateJavaScript(script, in: nil, in: .defaultClient) { [weak self, weak view] result in
                    guard let self else { return }
                    let measured: CGFloat
                    switch result {
                    case .success(let value as NSNumber): measured = CGFloat(truncating: value)
                    // Without a measurement, show it all rather than clip it.
                    default: measured = view?.scrollView.contentSize.height ?? self.height.wrappedValue
                    }
                    let fitted = max(20, measured.rounded(.up))
                    if abs(fitted - self.height.wrappedValue) > 1 { self.height.wrappedValue = fitted }
                }
            }

            func webView(_ view: WKWebView, didFinish navigation: WKNavigation!) {
                width = view.bounds.width
                fit(view)
            }

            func webView(_ view: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
                // The mail itself loads as about:blank; anything else is a link.
                if action.navigationType == .other, action.request.url?.absoluteString == "about:blank" {
                    return decisionHandler(.allow)
                }
                if action.navigationType == .linkActivated, let url = action.request.url,
                   ["http", "https", "mailto", "tel"].contains(url.scheme?.lowercased() ?? "") {
                    UIApplication.shared.open(url)
                }
                decisionHandler(.cancel)
            }
        }
    }
}

// MARK: - Compose

/// A new mail or a reply, sent with the provider's `compose` or `reply`.
struct MailComposeView: View {
    enum Mode {
        case new(FunctionContext)
        case reply(Message, ChannelRef)
    }

    @Environment(QmsgStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let provider: String
    let mode: Mode
    @State private var to = ""
    @State private var cc = ""
    @State private var bcc = ""
    @State private var showsCopies = false
    @State private var subject = ""
    @State private var message = ""
    @State private var files: [QmsgStore.Attachment] = []
    @State private var importing = false
    @State private var function: Function?
    @State private var busy = false
    @State private var error: String?
    @State private var warning: String?
    @State private var confirmDiscard = false

    private var original: Message? {
        if case .reply(let message, _) = mode { return message }
        return nil
    }

    private var context: FunctionContext {
        switch mode {
        case .new(let context): context
        case .reply(let message, let channel): .message(channel: channel, id: message.id)
        }
    }

    /// Who a reply goes to, as the provider picks: Reply-To, else From.
    private var replyRecipients: String {
        guard let email = original?.email else { return "" }
        let me = store.providers[provider]?.scope(original?.channel.organization)?.me
        let people = original?.author == me && !email.to.isEmpty ? email.to : (email.replyTo.isEmpty ? email.from : email.replyTo)
        return people.map(\.displayName).joined(separator: ", ")
    }

    private var edited: Bool {
        !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !files.isEmpty || (original == nil && (!to.isEmpty || !cc.isEmpty || !bcc.isEmpty || !subject.isEmpty))
    }

    private var canSend: Bool {
        function != nil && !busy && (original != nil || !addresses(to).isEmpty)
            && (!message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !files.isEmpty)
    }

    var body: some View {
        Form {
            Section {
                if original != nil {
                    LabeledContent("To", value: replyRecipients)
                    LabeledContent("Subject", value: replySubject)
                } else {
                    addressField("To", text: $to)
                    if showsCopies {
                        addressField("Cc", text: $cc)
                        addressField("Bcc", text: $bcc)
                    } else {
                        Button("Add Cc and Bcc") { showsCopies = true }.font(.subheadline)
                    }
                    TextField("Subject", text: $subject)
                }
            }
            Section {
                ComposerTextView(text: $message, maxLines: 14) { files.append(contentsOf: $0) }
                    .frame(minHeight: 220, alignment: .topLeading)
            }
            Section {
                ForEach(files.indices, id: \.self) { index in
                    HStack {
                        Label(files[index].name, systemImage: files[index].mime.hasPrefix("image/") ? "photo" : "doc")
                            .lineLimit(1)
                        Spacer()
                        Text(ByteCountFormatter.string(fromByteCount: Int64(files[index].data.count), countStyle: .file))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                .onDelete { files.remove(atOffsets: $0) }
                Button("Attach file", systemImage: "paperclip") { importing = true }
            }
            if let error { Section { ErrorNote(message: error) { self.error = nil } } }
        }
        .disabled(busy)
        .navigationTitle(original == nil ? "New Message" : "Reply")
        .navigationBarTitleDisplayMode(.inline)
        .interactiveDismissDisabled(edited)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { if edited { confirmDiscard = true } else { dismiss() } }
                    .disabled(busy)
            }
            ToolbarItem(placement: .confirmationAction) {
                Button { Task { await send() } } label: {
                    if busy { ProgressView() } else { Label("Send", systemImage: "arrow.up.circle.fill") }
                }
                .disabled(!canSend)
            }
        }
        .confirmationDialog("Discard this message?", isPresented: $confirmDiscard, titleVisibility: .visible) {
            Button("Discard", role: .destructive) { dismiss() }
        }
        .alert("Sent", isPresented: Binding(get: { warning != nil }, set: { if !$0 { warning = nil; dismiss() } })) {
            Button("OK") { warning = nil; dismiss() }
        } message: { Text(warning ?? "") }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            do {
                for url in try result.get() {
                    let granted = url.startAccessingSecurityScopedResource()
                    defer { if granted { url.stopAccessingSecurityScopedResource() } }
                    let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
                    guard size <= MediaLoader.maxDownload else { throw MediaError.tooLarge }
                    let data = try Data(contentsOf: url)
                    guard data.count <= MediaLoader.maxDownload else { throw MediaError.tooLarge }
                    let mime = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
                    files.append(.init(name: url.lastPathComponent, mime: mime, data: data))
                }
            } catch { self.error = QmsgStore.describe(error) }
        }
        .task {
            do {
                let id = original == nil ? "compose" : "reply"
                function = try await store.functions(context, provider: provider).first { $0.id == id }
                if function == nil { error = "This account can't send mail." }
            } catch { self.error = QmsgStore.describe(error) }
        }
    }

    private var replySubject: String {
        let subject = original?.email?.subject ?? ""
        return subject.lowercased().hasPrefix("re:") ? subject : "Re: " + subject
    }

    private func addressField(_ label: String, text: Binding<String>) -> some View {
        LabeledContent(label) {
            TextField("", text: text, prompt: Text(label == "To" ? "Required" : "Optional"))
                .keyboardType(.emailAddress).textContentType(.emailAddress)
                .textInputAutocapitalization(.never).autocorrectionDisabled()
        }
    }

    private func addresses(_ text: String) -> [Value] {
        text.split(whereSeparator: { $0 == "," || $0 == ";" || $0.isNewline })
            .map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }.map { .text($0) }
    }

    private func send() async {
        guard let function else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            var content: [Content] = [.text(message)]
            for file in files { content.append(try await store.upload(file, provider: provider)) }
            var arguments: Arguments = [StandardInput.content: .content(content)]
            if original == nil {
                arguments["to"] = .list(addresses(to))
                arguments["subject"] = .text(subject)
                if !addresses(cc).isEmpty { arguments["cc"] = .list(addresses(cc)) }
                if !addresses(bcc).isEmpty { arguments["bcc"] = .list(addresses(bcc)) }
            }
            switch try await store.call(function, context: context, arguments: arguments, provider: provider) {
            case .success(let reply):
                let note = reply["warning"]?.textValue ?? ""
                if note.isEmpty { dismiss() } else { warning = note }
            case .failure(let failure):
                error = failure.deliveryUnknown
                    ? "This message may have been sent. Check your Sent folder before sending it again."
                    : failure.description
            }
        } catch { self.error = QmsgStore.describe(error) }
    }
}
