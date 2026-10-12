import SwiftUI
import UniformTypeIdentifiers
import UIKit
import PhotosUI
// The Linux toolchain needs the SwiftUI overlay imported explicitly.
import _PhotosUI_SwiftUI
import QmsgKit

struct ChatView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var accounts
    let key: ChannelKey
    @State private var text = ""
    @State private var attachments: [QmsgStore.Attachment] = []
    @State private var reply: Message?
    @State private var importing = false
    @State private var showingMembers = false
    @State private var pickingPhotos = false
    @State private var photos: [PhotosPickerItem] = []
    @State private var actionMessage: Message?
    @State private var tool: MessageTool = .react
    @State private var deleting: Message?
    @State private var error: String?
    /// Whether the reader is at the end; only then does the chat follow
    /// new messages, a growing composer or the keyboard.
    @State private var atBottom = true
    /// The reader is dragging or flinging; never scroll under their finger.
    @State private var scrolling = false
    /// Mentions picked in the composer: what's shown ("@Ana") to what's sent ("<@123>").
    @State private var mentions: [String: String] = [:]

    private var scope: Scope? { store.providers[key.provider]?.scope(key.channel.organization) }
    private var channel: Channel? { scope?.channels[key.channel.channel] }
    private var conversation: Conversation { store.conversation(key) }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2) {
                    if let error = conversation.loadError {
                        NoticeCard(symbol: "exclamationmark.triangle.fill", tint: .orange, title: "Couldn't load messages", detail: error) {
                            Button("Try again") { Task { await store.loadLatest(key) } }
                        }
                    }
                    if conversation.isLoading && conversation.messages.isEmpty {
                        ProgressView().frame(maxWidth: .infinity).padding(.vertical, 40)
                    }
                    if !conversation.reachedStart && conversation.olderCursor != nil {
                        Button {
                            let anchor = conversation.messages.first?.id
                            Task {
                                await store.loadOlder(key)
                                // Keep the reader where they were, not at the new top.
                                if let anchor { proxy.scrollTo(anchor, anchor: .top) }
                            }
                        } label: {
                            if conversation.isLoadingOlder { ProgressView() } else { Text("Load earlier messages") }
                        }
                        .buttonStyle(.bordered).controlSize(.small)
                        .frame(maxWidth: .infinity).padding(.vertical, 8)
                        .disabled(conversation.isLoadingOlder)
                    }
                    if conversation.loaded && conversation.messages.isEmpty && conversation.pending.isEmpty {
                        ContentUnavailableView("No messages yet", systemImage: "bubble.left", description: Text("Say hello below."))
                    }
                    let messages = conversation.messages
                    ForEach(Array(messages.enumerated()), id: \.element.id) { index, message in
                        let previous = index > 0 ? messages[index - 1] : nil
                        if previous.map({ !Calendar.current.isDate($0.date, inSameDayAs: message.date) }) ?? true {
                            DaySeparator(date: message.date)
                        }
                        MessageRow(
                            message: message, scope: scope,
                            reply: message.replyTo.flatMap { conversation.message($0) },
                            showsAuthor: startsGroup(message, after: previous),
                            react: { reaction in Task { await toggle(reaction, on: message) } }
                        )
                        .id(message.id)
                        .contextMenu { menu(for: message) }
                    }
                    ForEach(conversation.pending) { pending in
                        PendingRow(
                            pending: pending, scope: scope,
                            retry: { store.retry(pending, in: key) },
                            reload: { Task { await store.loadLatest(key) } },
                            remove: { store.discard(pending, in: key) }
                        )
                    }
                    TimelineView(.periodic(from: .now, by: 2)) { _ in
                        let names = conversation.typingUsers.map { scope?.userName($0) ?? "Someone" }
                        if !names.isEmpty {
                            Label("\(names.joined(separator: ", ")) \(names.count == 1 ? "is" : "are") typing…", systemImage: "ellipsis.bubble")
                                .font(.caption).foregroundStyle(.secondary).padding(.top, 6)
                        }
                    }
                    Color.clear.frame(height: 1).id("bottom")
                }
                .padding(.horizontal, 12).padding(.vertical, 8)
            }
            .defaultScrollAnchor(.bottom, for: .initialOffset)
            .scrollDismissesKeyboard(.interactively)
            .onScrollPhaseChange { _, phase in
                scrolling = phase == .tracking || phase == .interacting || phase == .decelerating
            }
            .onScrollGeometryChange(for: ScrollMetrics.self, of: ScrollMetrics.init) { old, new in
                // The keyboard, the composer or the last rows changed size while
                // the reader was at the end: stay there. Otherwise the offset is
                // left alone, so reading older messages never jumps.
                if atBottom, !scrolling, new.viewport != old.viewport || new.content != old.content,
                   old.distance <= ScrollMetrics.near {
                    proxy.scrollTo("bottom", anchor: .bottom)
                    return
                }
                // Hysteresis, so the jump button doesn't flicker at the edge.
                if atBottom ? new.distance > ScrollMetrics.far : new.distance <= ScrollMetrics.near {
                    atBottom.toggle()
                }
            }
            .onChange(of: conversation.messages.last?.id) { old, _ in
                guard old != nil else {
                    // The first page came in after the view appeared, so the
                    // initial anchor didn't apply. The entry task marks it read.
                    proxy.scrollTo("bottom", anchor: .bottom)
                    atBottom = true
                    return
                }
                // Follow new messages only if the reader is already at the end,
                // or the message is their own.
                if atBottom || conversation.messages.last?.author == scope?.me {
                    withAnimation { proxy.scrollTo("bottom", anchor: .bottom) }
                }
                if atBottom { Task { await store.markRead(key) } }
            }
            .onChange(of: conversation.pending.count) { old, new in
                if new > old { withAnimation { proxy.scrollTo("bottom", anchor: .bottom) } }
            }
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    if !atBottom {
                        Button("Jump to latest", systemImage: "chevron.down") {
                            withAnimation { proxy.scrollTo("bottom", anchor: .bottom) }
                        }
                        .labelStyle(.iconOnly)
                    }
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if channel?.isWritable == true {
                    composer
                } else if channel != nil {
                    Text("You can't send messages here.")
                        .font(.footnote).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity).padding(12).background(.bar)
                }
            }
            // No pull to refresh: in a chat that starts at the bottom, pulling
            // at the top means "older", and a reload there jumped the view.
            // An overlay, not an inset: changing the scroll view's insets as
            // the link comes and goes would move what the reader sees.
            .overlay(alignment: .top) {
                ConnectionBanner(link: store.link) { store.resume() }
            }
        }
        .navigationTitle(channel.flatMap { scope?.title(of: $0) } ?? key.channel.channel)
        .navigationBarTitleDisplayMode(.inline)
        .background(Color.appCanvas.ignoresSafeArea())
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Members", systemImage: "person.2") { showingMembers = true }
                    .disabled(channel == nil)
            }
            ToolbarItem(placement: .principal) {
                VStack(spacing: 0) {
                    Text(channel.flatMap { scope?.title(of: $0) } ?? key.channel.channel).font(.headline).lineLimit(1)
                    Text(subtitle).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                }
            }
        }
        .task {
            store.visible = key
            await store.loadIfNeeded(key)
            await store.markRead(key)
        }
        .toolbar(.hidden, for: .tabBar)
        .onDisappear { if store.visible == key { store.visible = nil } }
        .sheet(item: $actionMessage) { message in
            NavigationStack {
                MessageToolsView(key: key, message: message, tool: tool)
            }
            .presentationDetents(tool == .react ? [.medium] : [.medium, .large])
        }
        .sheet(isPresented: $showingMembers) {
            NavigationStack { ChatMembersView(key: key) }
        }
        .confirmationDialog("Delete this message?", isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), titleVisibility: .visible) {
            if let message = deleting {
                Button("Delete for everyone", role: .destructive) { Task { await delete(message) } }
            }
        }
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
                    attachments.append(.init(name: url.lastPathComponent, mime: mime, data: data))
                }
            } catch { self.error = QmsgStore.describe(error) }
        }
        .photosPicker(isPresented: $pickingPhotos, selection: $photos, maxSelectionCount: 10, matching: .any(of: [.images, .videos]))
        .onChange(of: photos) { _, items in
            guard !items.isEmpty else { return }
            photos = []
            Task { await addPhotos(items) }
        }
    }

    private var placeholder: String {
        guard let channel, let scope else { return "Message" }
        let title = scope.title(of: channel)
        return isFeed ? "Message #\(title)" : "Message \(title)"
    }

    /// Photos come as JPEG unless the library has them in a common format,
    /// since few networks take HEIC.
    private func addPhotos(_ items: [PhotosPickerItem]) async {
        for item in items {
            do {
                guard let data = try await item.loadTransferable(type: Data.self) else { continue }
                guard data.count <= MediaLoader.maxDownload else { throw MediaError.tooLarge }
                let type = item.supportedContentTypes.first { $0.conforms(to: .image) || $0.conforms(to: .movie) }
                if let type, [UTType.jpeg, .png, .gif].contains(type) || type.conforms(to: .movie),
                   let mime = type.preferredMIMEType {
                    attachments.append(.init(name: "Photo.\(type.preferredFilenameExtension ?? "jpg")", mime: mime, data: data))
                } else if let image = UIImage(data: data), let jpeg = image.jpegData(compressionQuality: 0.9) {
                    attachments.append(.init(name: "Photo.jpg", mime: "image/jpeg", data: jpeg))
                }
            } catch { self.error = QmsgStore.describe(error) }
        }
    }

    private var subtitle: String {
        let account = accounts.label(for: key.provider)
        if let organization = key.channel.organization, let name = store.providers[key.provider]?.organizations[organization]?.name {
            return "\(name) · \(account)"
        }
        return account
    }

    /// Server channels read like a feed; direct chats use bubbles.
    private var isFeed: Bool { key.channel.organization != nil }

    /// A new author, or a pause of more than five minutes, starts a new group.
    private func startsGroup(_ message: Message, after previous: Message?) -> Bool {
        guard let previous else { return true }
        return previous.author != message.author || message.date.timeIntervalSince(previous.date) > 300
            || message.replyTo != nil
    }

    @ViewBuilder
    private func menu(for message: Message) -> some View {
        Button("Reply", systemImage: "arrowshape.turn.up.left") { reply = message }
        Button("React", systemImage: "face.smiling") { tool = .react; actionMessage = message }
        Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = message.content.compactMap(\.text).joined(separator: "\n") }
        if message.author == scope?.me {
            Button("Edit", systemImage: "pencil") { tool = .edit; actionMessage = message }
            Divider()
            Button("Delete", systemImage: "trash", role: .destructive) { deleting = message }
        }
    }

    @ViewBuilder
    private var composer: some View {
        if #available(iOS 26.0, *) {
            GlassEffectContainer(spacing: 4) { composerContent }
        } else {
            composerContent
        }
    }

    private var composerContent: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let error {
                ErrorNote(message: error) { self.error = nil }
            }
            if let reply {
                HStack(spacing: 8) {
                    RoundedRectangle(cornerRadius: 2).fill(.tint).frame(width: 3)
                    VStack(alignment: .leading, spacing: 1) {
                        Text("Replying to \(scope?.userName(reply.author) ?? "message")").font(.caption.bold()).foregroundStyle(.tint)
                        Text(reply.preview(in: scope)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer()
                    Button("Cancel reply", systemImage: "xmark.circle.fill") { self.reply = nil }
                        .labelStyle(.iconOnly).foregroundStyle(.secondary)
                }.frame(height: 34)
            }
            if !attachments.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack {
                        ForEach(attachments.indices, id: \.self) { index in
                            HStack(spacing: 6) {
                                Image(systemName: attachments[index].mime.hasPrefix("image/") ? "photo" : "doc")
                                Text(attachments[index].name).lineLimit(1).frame(maxWidth: 140)
                                Button("Remove attachment", systemImage: "xmark.circle.fill") { attachments.remove(at: index) }
                                    .labelStyle(.iconOnly).foregroundStyle(.secondary)
                            }
                            .font(.caption).padding(.horizontal, 10).padding(.vertical, 6)
                            .background(.quaternary, in: Capsule())
                        }
                    }
                }
            }
            if mentionQuery != nil, !mentionCandidates.isEmpty {
                mentionPicker
            }
            HStack(alignment: .bottom, spacing: 8) {
                Menu {
                    Button("Photos", systemImage: "photo.on.rectangle") { pickingPhotos = true }
                    Button("Files", systemImage: "folder") { importing = true }
                } label: {
                    Label("Attach", systemImage: "plus")
                }
                    .labelStyle(.iconOnly).font(.title3.weight(.medium))
                    .foregroundStyle(.primary)
                    .frame(width: 44, height: 44)
                    .chatGlass(in: Circle(), interactive: true)
                ComposerTextView(text: $text, placeholder: placeholder) { attachments.append(contentsOf: $0) }
                    .padding(.horizontal, 16).padding(.vertical, 12)
                    .chatGlass(in: RoundedRectangle(cornerRadius: 24, style: .continuous))
                Button("Send", systemImage: "arrow.up") { send() }
                    .labelStyle(.iconOnly).font(.body.bold())
                    .foregroundStyle(canSend ? Color.white : Color.secondary)
                    .frame(width: 44, height: 44)
                    .chatGlass(in: Circle(), tint: canSend ? .accentColor : nil, interactive: true)
                    .disabled(!canSend)
            }
        }
        .padding(.horizontal, 12).padding(.top, 8).padding(.bottom, 6)
    }

    // MARK: Mentions

    /// How this network writes a mention, or nil when the app doesn't know.
    private var mentionStyle: ((User) -> (shown: String, sent: String))? {
        let type = accounts.type(for: key.provider) ?? key.provider
        if type == "discord" { return { ("@" + $0.name, "<@\($0.id)>") } }
        if type == "echo" { return { ("@" + $0.name, "@" + $0.name) } }
        return nil
    }

    /// The word being typed after an "@", if any.
    private var mentionQuery: String? {
        guard mentionStyle != nil, let at = text.lastIndex(of: "@") else { return nil }
        if at != text.startIndex, !text[text.index(before: at)].isWhitespace { return nil }
        let query = text[text.index(after: at)...]
        guard !query.contains(where: \.isNewline), query.count <= 32 else { return nil }
        return String(query)
    }

    private var mentionCandidates: [User] {
        guard let query = mentionQuery, let scope, let channel else { return [] }
        return scope.mentionSuggestions(in: channel, recentAuthors: conversation.messages.suffix(50).map(\.author), query: query)
    }

    private var mentionPicker: some View {
        ScrollView {
          VStack(alignment: .leading, spacing: 0) {
            ForEach(mentionCandidates) { user in
                Button { insertMention(user) } label: {
                    HStack(spacing: 10) {
                        InitialsAvatar(name: user.name, size: 32)
                        Text(user.name).font(.subheadline)
                        if let username = user.username, username != user.name {
                            Text(username).font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 14).padding(.vertical, 8)
                    .foregroundStyle(.primary)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
          }
        }
        .frame(height: CGFloat(min(mentionCandidates.count, 3)) * 48)
        .padding(.vertical, 6)
        .chatGlass(in: RoundedRectangle(cornerRadius: 24, style: .continuous))
    }

    private func insertMention(_ user: User) {
        guard let style = mentionStyle, let at = text.lastIndex(of: "@") else { return }
        var mention = style(user)
        if scope?.users.values.filter({ $0.name.caseInsensitiveCompare(user.name) == .orderedSame }).count ?? 0 > 1 {
            mention.shown += " (" + (user.username ?? String(user.id.suffix(4))) + ")"
        }
        text = String(text[..<at]) + mention.shown + " "
        mentions[mention.shown] = mention.sent
    }

    /// The text as the network expects it, with picked mentions swapped in.
    private func wireText(_ text: String) -> String {
        Mention.encodePicked(text, replacements: mentions)
    }

    private var canSend: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty
    }

    private func toggle(_ reaction: Reaction, on message: Message) async {
        do {
            let context = FunctionContext.message(channel: key.channel, id: message.id)
            let action: ActionKind = reaction.me ? .removeReaction : .addReaction
            guard let function = try await store.functions(context, provider: key.provider).first(where: { $0.action == action }) else {
                throw StoreError.unsupported("This reaction cannot be changed.")
            }
            _ = try await store.call(function, context: context, arguments: [StandardInput.reaction: .text(reaction.key)], provider: key.provider).get()
        } catch { self.error = QmsgStore.describe(error) }
    }

    private func delete(_ message: Message) async {
        do {
            let context = FunctionContext.message(channel: key.channel, id: message.id)
            guard let function = try await store.functions(context, provider: key.provider).first(where: { $0.action == .deleteMessage }) else {
                throw StoreError.unsupported("This message cannot be deleted.")
            }
            _ = try await store.call(function, context: context, arguments: [:], provider: key.provider).get()
        } catch { self.error = QmsgStore.describe(error) }
    }

    private func send() {
        guard let channel else { return }
        let trimmed = wireText(text.trimmingCharacters(in: .whitespacesAndNewlines))
        if !trimmed.isEmpty {
            guard let rule = channel.rule(for: .text) else { error = "This channel doesn't take plain text."; return }
            if let limit = rule.maxSize, UInt64(rule.textUnit.measure(trimmed)) > limit { error = "The message is too long."; return }
        }
        if let max = channel.limits.maxAttachments, attachments.count > Int(max) { error = "Too many attachments."; return }
        var total = UInt64(trimmed.utf8.count)
        for file in attachments {
            guard let rule = channel.rule(for: file.kind), rule.takes(mime: file.mime) else { error = "This channel doesn't take \(file.mime) files."; return }
            if let max = rule.maxSize, UInt64(file.data.count) > max { error = "\(file.name) is too large."; return }
            total += UInt64(file.data.count)
        }
        if let max = channel.limits.maxTotalSize, total > max { error = "The message and files are too large."; return }
        store.send(text: trimmed, attachments: attachments, replyTo: reply?.id, to: key)
        text = ""
        mentions = [:]
        attachments = []
        reply = nil
        error = nil
    }
}

/// Where the scroll view is relative to the end of the chat.
private struct ScrollMetrics: Equatable {
    /// Within this of the end counts as at the end.
    static let near: CGFloat = 80
    /// Farther than this counts as reading back.
    static let far: CGFloat = 200

    /// How far the end of the content is below the visible area.
    var distance: CGFloat
    var viewport: CGFloat
    var content: CGFloat

    init(_ geometry: ScrollGeometry) {
        // The container is the whole scroll view and the offset includes the
        // insets, so the furthest offset is the content plus its bottom inset
        // less the container.
        let maxOffset = geometry.contentSize.height + geometry.contentInsets.bottom - geometry.containerSize.height
        distance = maxOffset - geometry.contentOffset.y
        viewport = geometry.containerSize.height - geometry.contentInsets.top - geometry.contentInsets.bottom
        content = geometry.contentSize.height
    }
}

private struct DaySeparator: View {
    let date: Date
    var body: some View {
        Text(Calendar.current.isDateInToday(date) ? "Today" : Calendar.current.isDateInYesterday(date) ? "Yesterday" : date.formatted(date: .abbreviated, time: .omitted))
            .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            .padding(.horizontal, 10).padding(.vertical, 4)
            .background(.quaternary, in: Capsule())
            .frame(maxWidth: .infinity).padding(.vertical, 10)
    }
}

/// A card for problems that need a button, like a failed load.
private struct NoticeCard<Actions: View>: View {
    let symbol: String
    let tint: Color
    let title: String
    let detail: String?
    @ViewBuilder let actions: () -> Actions

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(title, systemImage: symbol).font(.subheadline.weight(.semibold)).foregroundStyle(tint)
            if let detail { Text(detail).font(.footnote).foregroundStyle(.secondary).textSelection(.enabled) }
            HStack { actions() }.buttonStyle(.bordered).controlSize(.small)
        }
        .padding(12).frame(maxWidth: .infinity, alignment: .leading)
        .background(tint.opacity(0.1), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .padding(.vertical, 6)
    }
}

/// A message on its way. Failed sends can be retried; an unconfirmed one
/// may have gone through, so it's never resent automatically.
private struct PendingRow: View {
    let pending: PendingSend
    let scope: Scope?
    let retry: () -> Void
    let reload: () -> Void
    let remove: () -> Void

    var body: some View {
        VStack(alignment: .trailing, spacing: 4) {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(pending.content.enumerated()), id: \.offset) { index, part in
                    if let data = pending.previews[index], let image = UIImage(data: data) {
                        Image(uiImage: image).resizable().scaledToFit().frame(maxHeight: 200)
                            .clipShape(RoundedRectangle(cornerRadius: 10))
                    } else if let text = part.text {
                        Text(Mentions.render(text, scope: scope))
                    } else {
                        Label(part.media?.name ?? "Attachment", systemImage: "doc")
                    }
                }
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .foregroundStyle(.white)
            .background(bubbleColor, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .opacity(pending.state == .sending ? 0.6 : 1)
            status
        }
        .padding(.leading, 48).padding(.top, 6)
        .frame(maxWidth: .infinity, alignment: .trailing)
    }

    private var bubbleColor: Color {
        switch pending.state {
        case .sending: .accentColor
        case .failed: .red.opacity(0.75)
        case .unknown: .orange.opacity(0.8)
        }
    }

    @ViewBuilder
    private var status: some View {
        switch pending.state {
        case .sending:
            Label("Sending…", systemImage: "clock").font(.caption2).foregroundStyle(.secondary)
        case .failed(let reason):
            VStack(alignment: .trailing, spacing: 4) {
                Label("Not sent", systemImage: "exclamationmark.circle.fill").font(.caption.bold()).foregroundStyle(.red)
                if !reason.isEmpty { Text(reason).font(.caption2).foregroundStyle(.secondary).multilineTextAlignment(.trailing) }
                HStack {
                    Button("Delete", role: .destructive, action: remove)
                    Button("Retry", action: retry).buttonStyle(.borderedProminent)
                }.buttonStyle(.bordered).controlSize(.mini)
            }
        case .unknown(let reason):
            VStack(alignment: .trailing, spacing: 4) {
                Label("Could not confirm delivery", systemImage: "questionmark.circle.fill").font(.caption.bold()).foregroundStyle(.orange)
                Text("It may have been sent. Reload the chat to check before sending again.")
                    .font(.caption2).foregroundStyle(.secondary).multilineTextAlignment(.trailing)
                if !reason.isEmpty { Text(reason).font(.caption2).foregroundStyle(.tertiary).multilineTextAlignment(.trailing) }
                HStack {
                    Button("Dismiss", action: remove)
                    Button("Reload chat", action: reload).buttonStyle(.borderedProminent).tint(.orange)
                }.buttonStyle(.bordered).controlSize(.mini)
            }
        }
    }
}

private struct MessageRow: View {
    let message: Message
    let scope: Scope?
    let reply: Message?
    let showsAuthor: Bool
    let react: (Reaction) -> Void

    private var mine: Bool { message.author == scope?.me }
    private var author: String { scope?.userName(message.author) ?? "Unknown user" }

    var body: some View { bubbleRow }

    private var bubbleRow: some View {
        VStack(alignment: mine ? .trailing : .leading, spacing: 3) {
            if showsAuthor && !mine {
                Text(author).font(.footnote.weight(.semibold)).padding(.leading, 4)
            }
            bubble
            reactionsRow
        }
        .padding(mine ? .leading : .trailing, 48)
        .padding(.top, showsAuthor ? 12 : 1)
        .frame(maxWidth: .infinity, alignment: mine ? .trailing : .leading)
    }

    @ViewBuilder
    private var replyQuote: some View {
        if message.replyTo != nil {
            VStack(alignment: .leading, spacing: 2) {
                Text(reply.map { scope?.userName($0.author) ?? "Unknown user" } ?? "Earlier message").bold()
                if let reply { Text(reply.preview(in: scope)).lineLimit(2) }
            }
            .font(.caption)
            .padding(.leading, 8)
            .overlay(alignment: .leading) { RoundedRectangle(cornerRadius: 1).fill(mine ? Color.white.opacity(0.7) : Color.secondary.opacity(0.5)).frame(width: 2) }
            .opacity(0.85)
        }
    }

    @ViewBuilder
    private var messageContent: some View {
        if let email = message.email { Text(email.subject).font(.headline) }
        ForEach(Array(message.content.enumerated()), id: \.offset) { _, content in
            if let text = content.text { Text(Mentions.render(text, scope: scope)).textSelection(.enabled) }
            else if let media = content.media { MediaView(media: media) }
            else { Text("Unsupported content").font(.caption).opacity(0.7) }
        }
    }

    @ViewBuilder
    private var reactionsRow: some View {
        if !message.reactions.isEmpty {
            HStack(spacing: 4) {
                ForEach(message.reactions, id: \.key) { reaction in
                    Button { react(reaction) } label: {
                        Text("\(reaction.key) \(reaction.count)").font(.caption)
                            .padding(.horizontal, 8).padding(.vertical, 4)
                            .background(reaction.me ? Color.accentColor.opacity(0.2) : Color.appCanvas, in: Capsule())
                            .overlay(Capsule().stroke(reaction.me ? Color.accentColor : .clear, lineWidth: 1))
                    }.buttonStyle(.plain)
                }
            }
        }
    }

    private var bubble: some View {
        VStack(alignment: .leading, spacing: 6) {
            replyQuote
            messageContent
            HStack(spacing: 4) {
                if message.editedAt != nil { Text("edited") }
                Text(message.date, style: .time)
            }
            .font(.caption2).opacity(0.65)
        }
        .padding(.horizontal, 12).padding(.vertical, 8)
        .foregroundStyle(mine ? Color.white : Color.primary)
        .tint(mine ? Color.white : Color.accentColor)
        .background(mine ? Color.accentColor : Color(uiColor: .tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .fixedSize(horizontal: false, vertical: true)
    }
}
