import SwiftUI
import UniformTypeIdentifiers
import QmsgKit

struct ChatView: View {
    @Environment(QmsgStore.self) private var store
    let key: ChannelKey
    @State private var text = ""
    @State private var attachments: [QmsgStore.Attachment] = []
    @State private var reply: Message?
    @State private var importing = false
    @State private var actionMessage: Message?
    @State private var channelActions = false
    @State private var error: String?

    private var scope: Scope? { store.providers[key.provider]?.scope(key.channel.organization) }
    private var channel: Channel? { scope?.channels[key.channel.channel] }
    private var conversation: Conversation { store.conversation(key) }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 18) {
                    if store.link != .live { ConnectionLabel(link: store.link) }
                    if let error = conversation.loadError {
                        Text(error).foregroundStyle(.red)
                        Button("Try again") { Task { await store.loadLatest(key) } }
                    }
                    if conversation.isLoading { ProgressView().frame(maxWidth: .infinity) }
                    if !conversation.reachedStart && conversation.olderCursor != nil {
                        Button("Load older messages") { Task { await store.loadOlder(key) } }
                            .disabled(conversation.isLoadingOlder)
                    }
                    if conversation.loaded && conversation.messages.isEmpty {
                        ContentUnavailableView("No messages yet", systemImage: "bubble.left", description: Text("Start the conversation below."))
                    }
                    ForEach(conversation.messages) { message in
                        MessageRow(message: message, scope: scope)
                            .id(message.id)
                            .contextMenu {
                                Button("Reply", systemImage: "arrowshape.turn.up.left") { reply = message }
                                Button("Actions", systemImage: "ellipsis") { actionMessage = message }
                            }
                    }
                    ForEach(conversation.pending) { pending in
                        VStack(alignment: .leading, spacing: 6) {
                            ForEach(Array(pending.content.enumerated()), id: \.offset) { _, part in
                                Text(part.text ?? part.media?.name ?? "Attachment")
                            }
                            switch pending.state {
                            case .sending: Label("Sending…", systemImage: "clock").foregroundStyle(.secondary)
                            case .failed(let reason):
                                Text(reason).foregroundStyle(.red)
                                Button("Retry") { store.retry(pending, in: key) }
                                Button("Remove") { store.discard(pending, in: key) }
                            case .unknown(let reason):
                                Text("Delivery unknown. \(reason) Check the chat before sending again.").foregroundStyle(.orange)
                                Button("Reload chat") { Task { await store.loadLatest(key) } }
                                Button("Remove") { store.discard(pending, in: key) }
                            }
                        }.font(.callout).padding(12).frame(maxWidth: .infinity, alignment: .leading)
                            .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
                    }
                    TimelineView(.periodic(from: .now, by: 2)) { _ in
                        let names = conversation.typingUsers.map { scope?.users[$0]?.name ?? $0 }
                        if !names.isEmpty { Text("\(names.joined(separator: ", ")) typing…").font(.caption).foregroundStyle(.secondary) }
                    }
                    Color.clear.frame(height: 1).id("bottom")
                }.padding()
            }
            .defaultScrollAnchor(.bottom)
            .onChange(of: conversation.messages.last?.id) { _, _ in
                proxy.scrollTo("bottom", anchor: .bottom)
                Task { await store.markRead(key) }
            }
            .onChange(of: conversation.pending.count) { _, _ in proxy.scrollTo("bottom", anchor: .bottom) }
            .refreshable { await store.loadLatest(key) }
        }
        .navigationTitle(channel.flatMap { scope?.title(of: $0) } ?? key.channel.channel)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { Button("Channel actions", systemImage: "ellipsis.circle") { channelActions = true } }
        .safeAreaInset(edge: .bottom) {
            if channel?.isWritable == true { composer }
        }
        .task {
            store.visible = key
            await store.loadLatest(key)
            await store.markRead(key)
        }
        .onDisappear { if store.visible == key { store.visible = nil } }
        .sheet(item: $actionMessage) { message in
            NavigationStack {
                ActionsView(provider: key.provider, context: .message(channel: key.channel, id: message.id), initialContent: message.content)
            }
        }
        .sheet(isPresented: $channelActions) {
            NavigationStack { ActionsView(provider: key.provider, context: .channel(key.channel)) }
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
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let reply {
                HStack {
                    Text("Reply to \(scope?.users[reply.author]?.name ?? reply.author)").font(.caption)
                    Spacer()
                    Button("Cancel reply", systemImage: "xmark") { self.reply = nil }.labelStyle(.iconOnly)
                }
            }
            ForEach(attachments.indices, id: \.self) { index in
                HStack {
                    Label(attachments[index].name, systemImage: "paperclip").font(.caption).lineLimit(1)
                    Spacer()
                    Button("Remove attachment", systemImage: "xmark") { attachments.remove(at: index) }.labelStyle(.iconOnly)
                }
            }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
            HStack(alignment: .bottom) {
                Button("Attach file", systemImage: "plus") { importing = true }.labelStyle(.iconOnly)
                TextField("Message", text: $text, axis: .vertical).lineLimit(1...6)
                    .padding(10).background(Color.secondary.opacity(0.1), in: RoundedRectangle(cornerRadius: 16))
                Button("Send", systemImage: "arrow.up.circle.fill") { send() }
                    .font(.title2).labelStyle(.iconOnly)
                    .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && attachments.isEmpty)
            }
        }.padding().background(.bar)
    }

    private func send() {
        guard let channel else { return }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            guard let rule = channel.rule(for: .text) else { error = "This channel doesn't take plain text."; return }
            if let limit = rule.maxSize, UInt64(rule.textUnit.measure(trimmed)) > limit { error = "The message is too long."; return }
        }
        if let max = channel.limits.maxAttachments, attachments.count > Int(max) { error = "Too many attachments."; return }
        var total = UInt64(trimmed.utf8.count)
        for file in attachments {
            let kind: ContentKind
            switch file.mime.split(separator: "/").first {
            case "image": kind = .image
            case "video": kind = .video
            case "audio": kind = .audio
            default: kind = .file
            }
            guard let rule = channel.rule(for: kind), rule.takes(mime: file.mime) else { error = "This channel doesn't take \(file.mime) files."; return }
            if let max = rule.maxSize, UInt64(file.data.count) > max { error = "\(file.name) is too large."; return }
            total += UInt64(file.data.count)
        }
        if let max = channel.limits.maxTotalSize, total > max { error = "The message and files are too large."; return }
        store.send(text: trimmed, attachments: attachments, replyTo: reply?.id, to: key)
        text = ""
        attachments = []
        reply = nil
        error = nil
    }
}

private struct MessageRow: View {
    let message: Message
    let scope: Scope?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(scope?.users[message.author]?.name ?? message.author).font(.subheadline.bold())
                Text(message.date, style: .time).font(.caption).foregroundStyle(.secondary)
                if message.editedAt != nil { Text("edited").font(.caption).foregroundStyle(.secondary) }
            }
            if let replyTo = message.replyTo { Label("Reply to \(replyTo)", systemImage: "arrowshape.turn.up.left").font(.caption).foregroundStyle(.secondary) }
            if let email = message.email { Text(email.subject).font(.headline) }
            ForEach(Array(message.content.enumerated()), id: \.offset) { _, content in
                if let text = content.text { Text(text).textSelection(.enabled) }
                else if let media = content.media { MediaView(media: media) }
                else { Text("Unsupported content").font(.caption).foregroundStyle(.secondary) }
            }
            if !message.reactions.isEmpty {
                Text(message.reactions.map { "\($0.key) \($0.count)" }.joined(separator: "   "))
                    .font(.caption).padding(6).background(Color.secondary.opacity(0.1), in: Capsule())
            }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
}
