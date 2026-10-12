import Foundation
import Testing
@testable import QmsgKit

@Suite struct ModelTests {
    @Test func directChatTitleFollowsItsRecipientsProfile() {
        var dm = channel("3100")
        dm.kind = .direct
        dm.name = "1002"
        dm.members = ["1001", "1002"]
        var scope = Scope(me: "1001", users: ["1002": User(id: "1002", name: "Ana", username: "ana.dev")])
        #expect(scope.title(of: dm) == "Ana")
        scope.users["1002"]?.name = "Ana Smith"
        #expect(scope.title(of: dm) == "Ana Smith")
    }
    @MainActor @Test func mailWithTheSameDateUsesNumericUIDOrder() throws {
        let nine = try mail("1:9", read: false, html: nil, contentIDs: [:])
        let ten = try mail("1:10", read: false, html: nil, contentIDs: [:])
        #expect(Conversation.sorted([ten, nine]).map(\.id) == ["1:9", "1:10"])
    }

    @MainActor @Test func mailSummaryUpdatesFlagsWithoutDroppingTheLoadedBody() throws {
        var full = try mail("1:9", read: false, html: "<p>Body</p>", contentIDs: [:])
        full.content = [.text("Body")]
        var summary = full
        summary.content = [.custom(kind: "email-summary", data: [])]
        summary.email?.html = nil
        summary.email?.read = true
        let conversation = Conversation(key: ChannelKey(provider: "gmail", channel: full.channel))
        conversation.merge([full])
        conversation.merge([summary])
        #expect(conversation.message(full.id)?.mailBody == "Body")
        #expect(conversation.message(full.id)?.email?.read == true)
        #expect(conversation.message(full.id)?.mailNeedsBody == false)
    }

    @Test func accountIdentityUsesTheSignedInUserRatherThanTheProviderLabel() {
        let discord = ProviderState(id: ProviderID(name: "discord", instance: 1), standalone: Scope(me: "1", users: ["1": User(id: "1", name: "Ana", username: "ana.dev")]))
        #expect(discord.accountIdentity == "Ana · @ana.dev")
        let mail = ProviderState(id: ProviderID(name: "gmail", instance: 1), standalone: Scope(me: "ana@example.com"))
        #expect(mail.accountIdentity == "ana@example.com")
    }
    @Test func mentionSuggestionsStayWithinTheChat() {
        var chat = channel("c")
        chat.members = ["me", "ana"]
        let scope = Scope(me: "me", users: [
            "me": User(id: "me", name: "Me"),
            "ana": User(id: "ana", name: "Ana"),
            "other": User(id: "other", name: "Other")
        ])
        #expect(scope.mentionSuggestions(in: chat, recentAuthors: ["other"], query: "").map(\.id) == ["ana"])
        chat.members = []
        #expect(scope.mentionSuggestions(in: chat, recentAuthors: ["other"], query: "").isEmpty)
        chat.members = nil
        #expect(scope.mentionSuggestions(in: chat, recentAuthors: ["ana"], query: "").map(\.id) == ["ana"])
        #expect(scope.mentionSuggestions(in: chat, recentAuthors: ["ana"], query: "oth").map(\.id) == ["other"])
    }

    @MainActor @Test func pendingSendNonceFitsDiscordAndRetryGetsANewNonce() throws {
        let store = QmsgStore(accounts: MemoryAccountStore())
        let key = ChannelKey(provider: "discord", channel: ChannelRef(organization: nil, channel: "test"))
        store.send(text: "test", to: key)
        let pending = try #require(store.conversation(key).pending.first)
        #expect(pending.id.utf8.count <= 25, "Discord rejects send nonces longer than 25 characters")
        store.retry(pending, in: key)
        let retry = try #require(store.conversation(key).pending.first)
        #expect(retry.id.utf8.count <= 25)
        #expect(retry.id != pending.id)
    }

    @MainActor @Test func recentDoesNotFollowTrafficInUnopenedServerChannels() {
        let store = QmsgStore(accounts: MemoryAccountStore())
        let id = ProviderID(name: "discord", instance: 1)
        let org = OrganizationEntry(id: "org", name: "Server", icon: nil, scope: Scope(channels: ["visited": channel("visited"), "busy": channel("busy")]))
        store.apply(.snapshot(Snapshot(providers: [ProviderView(provider: id, status: .ready, organizations: [org], standalone: Scope())], epoch: "epoch", sequence: 0)))
        let visited = ChannelKey(provider: "discord", channel: ChannelRef(organization: "org", channel: "visited"))
        store.visible = visited
        var first = message("1", at: 1); first.channel = visited.channel
        store.apply(.update(.message(provider: id, event: .received(first)), epoch: "epoch", sequence: 1))
        let busy = ChannelKey(provider: "discord", channel: ChannelRef(organization: "org", channel: "busy"))
        var next = message("2", at: 2); next.channel = busy.channel
        store.apply(.update(.message(provider: id, event: .received(next)), epoch: "epoch", sequence: 2))
        #expect(store.recentKeys == [visited], "traffic in unopened server channels must not change Recent")
        #expect(store.activity[busy]?.unread == 1)
    }

    @MainActor @Test func reopeningLoadedChatUsesCachedMessages() async {
        let store = QmsgStore(accounts: MemoryAccountStore())
        let key = ChannelKey(provider: "discord", channel: ChannelRef(organization: nil, channel: "c"))
        let conversation = store.conversation(key)
        conversation.replace(with: HistoryPage(messages: [message("1", at: 1)], nextCursor: nil))
        await store.loadIfNeeded(key)
        #expect(conversation.loadError == nil, "a loaded chat should open without a new network request")
        #expect(conversation.messages.count == 1)
    }

    @Test func pickedMentionsKeepNamesAndAddressesIntact() {
        let picks = ["@Ana": "<@123>", "@Bo Smith": "<@456>"]
        #expect(Mention.encodePicked("Hello @Ana, and @Bo Smith!", replacements: picks) == "Hello <@123>, and <@456>!")
        #expect(Mention.encodePicked("@Anabelle and mail@Ana", replacements: picks) == "@Anabelle and mail@Ana")
        #expect(Mention.encodePicked("@Ana @Ana", replacements: picks) == "<@123> <@123>")
        let scope = Scope(users: ["123": User(id: "123", name: "Ana", username: "ana.dev")])
        #expect(scope.userName("123") == "Ana")
        #expect(scope.userName("999") == "Unknown user")
    }

    @Test func commandsEncodeAsSerdeExpects() throws {
        let channel = ChannelRef(organization: nil, channel: "3100")
        #expect(try QmsgJSON.encode(Command.history(channel: channel, cursor: nil, limit: 50))
            == #"{"History":{"channel":{"channel":"3100"},"cursor":null,"limit":50}}"#)
        #expect(try QmsgJSON.encode(Command.functions(context: .provider)) == #"{"Functions":{"context":"Provider"}}"#)
        let call = FunctionCall(function: "react", context: .message(channel: channel, id: "1"), arguments: ["reaction": .text("👍")])
        #expect(try QmsgJSON.encode(Command.call(call))
            == #"{"Call":{"arguments":{"reaction":{"Text":"👍"}},"context":{"Message":{"channel":{"channel":"3100"},"id":"1"}},"function":"react"}}"#)
        let send = Command.send(channel: channel, replyTo: nil, content: [.text("hi"), .image(Media(name: "a.png", mime: "image/png", size: 3, source: .blob("h")))], nonce: "n")
        let json = try QmsgJSON.encode(send)
        #expect(json.contains(#"{"Image":{"metadata":{"spoiler":false},"mime":"image/png","name":"a.png","size":3,"source":{"Blob":"h"}}}"#))
        #expect(json.contains(#""content":[{"Text":"hi"}"#))
    }

    @Test func decodesEventsAndStatuses() throws {
        let received = """
        {"Message":{"provider":{"name":"echo","instance":4},"event":{"Received":{"id":"7","channel":{"organization":"s","channel":"s"},"author":"me","sent_at":1760000000000,"reply_to":null,"content":[{"Formatted":{"text":"@Bo hi","spans":[{"start":0,"end":3,"style":{"Mention":{"User":"bo"}}},{"start":4,"end":6,"style":{"CodeBlock":{"language":null}}}]}},{"Custom":{"kind":"sticker","data":[1,2]}}],"edited_at":null,"reactions":[{"key":"👍","count":2,"me":true}],"nonce":"n1","email":null}}}}
        """
        guard case .message(let provider, .received(let message)) = try QmsgJSON.decode(ProviderEvent.self, from: received) else {
            Issue.record("wrong event"); return
        }
        #expect(provider == ProviderID(name: "echo", instance: 4))
        #expect(message.nonce == "n1")
        #expect(message.reactions.first?.count == 2)
        guard case .formatted(let text) = message.content[0] else { Issue.record("not formatted"); return }
        #expect(text.spans[0].style == .mention(.user("bo")))
        #expect(text.spans[1].style == .codeBlock(language: nil))
        #expect(message.content[1] == .custom(kind: "sticker", data: [1, 2]))

        let status = #"{"Status":{"provider":{"name":"d","instance":1},"status":{"LoginRequired":{"message":"Scan","function":null,"link":"https://x","qr_code":"q"}}}}"#
        guard case .status(_, .loginRequired(let step)) = try QmsgJSON.decode(ProviderEvent.self, from: status) else {
            Issue.record("wrong status"); return
        }
        #expect(step.link == "https://x" && step.qrCode == "q")
        let exited = #"{"Exited":{"provider":{"name":"d","instance":1},"result":{"Err":"boom"}}}"#
        guard case .exited(_, let error) = try QmsgJSON.decode(ProviderEvent.self, from: exited) else { Issue.record("wrong"); return }
        #expect(error == "boom")
        let ok = #"{"Exited":{"provider":{"name":"d","instance":1},"result":{"Ok":null}}}"#
        guard case .exited(_, nil) = try QmsgJSON.decode(ProviderEvent.self, from: ok) else { Issue.record("wrong"); return }
        let disconnected = try QmsgJSON.decode(ProviderStatus.self, from: #"{"Disconnected":{"reason":"net"}}"#)
        #expect(disconnected == .disconnected(reason: "net"))
    }

    @Test func decodesFunctionsAndErrors() throws {
        let json = """
        {"Functions":[{"id":"login","label":"Log in","description":"","kind":{"Action":{"action":"Login","result":"Null"}},"inputs":[{"id":"password","label":"Password","description":"","value_type":"Text","required":true,"hint":"Secret","verification":null,"completion":null},{"id":"tags","label":"Tags","description":"","value_type":{"List":"Text"},"required":false,"hint":{"Choices":[{"label":"A","value":{"Text":"a"}}]},"verification":null,"completion":"tags"}]},{"id":"tags","label":"","description":"","kind":{"Completion":"Text"},"inputs":[]}]}
        """
        guard case .functions(let functions) = try QmsgJSON.decode(Reply.self, from: json) else { Issue.record("wrong"); return }
        #expect(functions[0].action == .login)
        #expect(functions[0].inputs[0].hint == .secret)
        #expect(functions[0].inputs[1].valueType == .list(.text))
        #expect(functions[0].inputs[1].hint == .choices([Choice(label: "A", value: .text("a"))]))
        #expect(functions[1].kind == .completion(.text))

        let error = try QmsgJSON.decode(AppError.self, from: #"{"code":"provider","message":"x","command":{"Rejected":{"TextTooLong":{"length":3000,"max":2000}}},"delivery_unknown":false}"#)
        #expect(error.description == "Rejected: the text is too long (3000 of 2000)")
        let limited = try QmsgJSON.decode(AppError.self, from: #"{"code":"provider","message":"x","command":{"RateLimited":{"retry_after_ms":4000}},"delivery_unknown":true}"#)
        #expect(limited.deliveryUnknown && limited.description.contains("4 s"))
    }

    @Test func pairingPayloadValidation() throws {
        let future = Clock.nowMs() + 60_000
        func payload(_ endpoint: String, expires: UInt64 = future, pem: String? = nil, sha: String? = nil) -> PairingPayload {
            PairingPayload(version: 1, endpoint: endpoint, code: "c", expiresAtMs: expires, certificatePEM: pem, certificateSHA256: sha)
        }
        let tunnel = try ServerEndpoint(payload: payload("http://localhost:50051"))
        #expect(!tunnel.secure && tunnel.port == 50051)
        #expect(throws: PairingError.insecureEndpoint) { try ServerEndpoint(payload: payload("http://192.168.1.2:50051")) }
        #expect(throws: PairingError.expired) { try ServerEndpoint(payload: payload("https://a:1", expires: 1)) }
        #expect(throws: PairingError.invalidEndpoint("https://a:1/path")) { try ServerEndpoint(payload: payload("https://a:1/path")) }
        let v6 = try ServerEndpoint(payload: payload("http://[::1]:9"))
        #expect(v6.host == "::1")
        #expect(throws: PairingError.notAPairingCode) { try PairingError.payload(from: "hello") }
    }

    @Test func directoryFollowsOrchestratorRules() {
        var provider = ProviderState(id: ProviderID(name: "p", instance: 1))
        let general = channel("general")
        provider.apply(.organizationUpserted(Organization(id: "org", name: "Org", icon: nil, me: "ana", users: [User(id: "ana", name: "Ana")], channels: [general])))
        provider.apply(.channelUpserted(organization: "missing", channel: general))
        #expect(provider.organizations.count == 1)
        provider.apply(.presence(organization: "org", user: "ana", presence: .busy))
        provider.apply(.userRemoved(organization: "org", id: "ana"))
        #expect(provider.scope("org")?.me == nil)
        #expect(provider.scope("org")?.presence.isEmpty == true)
        var child = channel("child")
        child.parent = "general"
        provider.apply(.channelUpserted(organization: "org", channel: child))
        #expect(provider.scope("org")!.channelTree().map(\.channel.id) == ["general", "child"])
        provider.apply(.reset)
        #expect(provider.organizations.isEmpty)
    }

    @MainActor @Test func conversationMergesAndSettlesPending() {
        let key = ChannelKey(provider: "p", channel: ChannelRef(organization: nil, channel: "c"))
        let conversation = Conversation(key: key)
        conversation.pending = [PendingSend(id: "n", content: [.text("hi")], operation: ActionOperation(), state: .sending, previews: [:])]
        conversation.replace(with: HistoryPage(messages: [message("2", at: 2), message("1", at: 1)], nextCursor: "c"))
        #expect(conversation.messages.map(\.id) == ["1", "2"])
        var mine = message("3", at: 3)
        mine.nonce = "n"
        conversation.apply(.received(mine), me: "me")
        #expect(conversation.pending.isEmpty)
        conversation.apply(.reacted(channel: key.channel, id: "1", user: "me", key: "🎉", added: true), me: "me")
        #expect(conversation.messages[0].reactions == [Reaction(key: "🎉", count: 1, me: true)])
        conversation.apply(.reacted(channel: key.channel, id: "1", user: "me", key: "🎉", added: false), me: "me")
        #expect(conversation.messages[0].reactions.isEmpty)
        conversation.apply(.deleted(channel: key.channel, id: "2"), me: "me")
        #expect(conversation.messages.map(\.id) == ["1", "3"])
    }

    @MainActor @Test func refreshingKeepsOlderHistoryButDropsDeletesInItsWindow() {
        let key = ChannelKey(provider: "p", channel: ChannelRef(organization: nil, channel: "c"))
        let conversation = Conversation(key: key)
        conversation.replace(with: HistoryPage(messages: [message("3", at: 3), message("4", at: 4), message("5", at: 5)], nextCursor: "3"))
        conversation.prepend(HistoryPage(messages: [message("1", at: 1), message("2", at: 2)], nextCursor: "1"))
        // Pull to refresh: "4" was deleted while offline and "6" is new.
        conversation.replace(with: HistoryPage(messages: [message("3", at: 3), message("5", at: 5), message("6", at: 6)], nextCursor: "3"))
        #expect(conversation.messages.map(\.id) == ["1", "2", "3", "5", "6"], "a refresh must not throw away loaded older history")
        #expect(conversation.olderCursor == "1", "the cursor must still point before the oldest loaded message")
        #expect(!conversation.reachedStart)
    }

    @MainActor @Test func refreshWithoutOverlapStartsOver() {
        let key = ChannelKey(provider: "p", channel: ChannelRef(organization: nil, channel: "c"))
        let conversation = Conversation(key: key)
        conversation.replace(with: HistoryPage(messages: [message("1", at: 1), message("2", at: 2)], nextCursor: "1"))
        let generation = conversation.generation
        conversation.replace(with: HistoryPage(messages: [message("8", at: 8), message("9", at: 9)], nextCursor: "8"))
        #expect(conversation.messages.map(\.id) == ["8", "9"], "a gap between pages can't be bridged, so the old ones go")
        #expect(conversation.olderCursor == "8")
        #expect(conversation.generation != generation)
        // An older page requested before the gap no longer fits.
        #expect(!conversation.prepend(HistoryPage(messages: [message("0", at: 0)], nextCursor: nil), generation: generation))
        #expect(conversation.messages.map(\.id) == ["8", "9"])
        #expect(conversation.olderCursor == "8")
    }

    @MainActor @Test func eventsDuringALoadSurviveItsPage() {
        let key = ChannelKey(provider: "p", channel: ChannelRef(organization: nil, channel: "c"))
        let conversation = Conversation(key: key)
        conversation.replace(with: HistoryPage(messages: [message("1", at: 1), message("2", at: 2)], nextCursor: nil))
        conversation.beginLoad()
        // While the request is out: "3" arrives and "2" is deleted.
        conversation.apply(.received(message("3", at: 3)), me: "me")
        conversation.apply(.deleted(channel: key.channel, id: "2"), me: "me")
        // The page was read before either.
        conversation.replace(with: HistoryPage(messages: [message("1", at: 1), message("2", at: 2)], nextCursor: nil))
        #expect(conversation.messages.map(\.id) == ["1", "3"], "the page must neither resurrect a delete nor drop a newer arrival")
    }

    @MainActor @Test func firstLoadOfAnEmptyChannel() {
        let key = ChannelKey(provider: "p", channel: ChannelRef(organization: nil, channel: "c"))
        let conversation = Conversation(key: key)
        conversation.replace(with: HistoryPage(messages: [], nextCursor: nil))
        #expect(conversation.loaded && conversation.reachedStart && conversation.messages.isEmpty)
    }

    @MainActor @Test func mailFlagChangesDoNotCountAsUnreadArrivals() throws {
        let store = QmsgStore(accounts: MemoryAccountStore())
        let id = ProviderID(name: "gmail", instance: 1)
        let org = OrganizationEntry(id: "me@example.com", name: "me@example.com", icon: nil, scope: Scope(me: "me@example.com", channels: ["INBOX": try mailbox("INBOX", [])]))
        store.apply(.snapshot(Snapshot(providers: [ProviderView(provider: id, status: .ready, organizations: [org], standalone: Scope())], epoch: "epoch", sequence: 0)))
        let key = ChannelKey(provider: "gmail", channel: ChannelRef(organization: org.id, channel: "INBOX"))
        let mail = try mail("7:1", read: true, html: nil, contentIDs: [:], channel: key.channel)
        // The provider re-sends a mail after star or mark read.
        store.apply(.update(.message(provider: id, event: .received(mail)), epoch: "epoch", sequence: 1))
        store.apply(.update(.message(provider: id, event: .received(mail)), epoch: "epoch", sequence: 2))
        #expect(store.activity[key]?.unread ?? 0 == 0)
        #expect(store.activity[key]?.last?.id == "7:1")
    }

    @Test func mailboxesSortLikeMailAndUseTheirOwnNames() throws {
        let scope = Scope(channels: [
            "Receipts": try mailbox("Receipts", []),
            "[Gmail]/Trash": try mailbox("[Gmail]/Trash", ["\\HasNoChildren", "\\Trash"]),
            "[Gmail]/Sent Mail": try mailbox("[Gmail]/Sent Mail", ["\\Sent"]),
            "INBOX": try mailbox("INBOX", []),
            "[Gmail]/Starred": try mailbox("[Gmail]/Starred", ["\\Flagged"]),
            "Spam": try mailbox("Spam", []),
        ])
        #expect(scope.mailboxes.map(\.mailboxName) == ["Inbox", "Starred", "Sent Mail", "Spam", "Trash", "Receipts"])
        #expect(MailboxRole(scope.channels["Spam"]!) == .junk)
    }

    @Test func mailKeepsUnreferencedInlineImagesAsAttachments() throws {
        let image = Content.image(Media(name: "logo.png", mime: "image/png", size: 1, source: .bytes([1])))
        let photo = Content.image(Media(name: "photo.jpg", mime: "image/jpeg", size: 1, source: .bytes([2])))
        var mail = try mail("1:1", read: false, html: #"<p>Hi&nbsp;<b>there</b></p><img src="cid:Logo@x">"#, contentIDs: ["0": "logo@x", "1": "photo@x"])
        mail.content = [image, photo]
        #expect(mail.inlineImages == [0: "logo@x"])
        #expect(mail.mailAttachments.map(\.name) == ["photo.jpg"])
        #expect(mail.mailBody == "Hi there")
    }

    @Test func mailIgnoresInvalidInlineImageIndexes() throws {
        var mail = try mail("1:1", read: false, html: #"<img src="cid:logo@x">"#,
                            contentIDs: ["-1": "logo@x", "1": "logo@x", "invalid": "logo@x"])
        mail.content = [.image(Media(name: "logo.png", mime: "image/png", size: 1, source: .bytes([1])))]
        #expect(mail.inlineImages.isEmpty)
        #expect(mail.mailAttachments.count == 1)
    }

    private func mailbox(_ id: String, _ attributes: [String]) throws -> Channel {
        let attrs = String(data: try JSONEncoder().encode(attributes), encoding: .utf8)!
        return try QmsgJSON.decode(Channel.self, from: """
        {"id":"\(id)","name":"\(id)","topic":null,"icon":null,"kind":{"Custom":"mailbox"},"parent":null,"position":null,"members":null,"accepted_content":[],"limits":{"max_attachments":null,"max_total_size":null},"mailbox":{"delimiter":"/","attributes":\(attrs)}}
        """)
    }

    private func mail(_ id: String, read: Bool, html: String?, contentIDs: [String: String], channel: ChannelRef = ChannelRef(organization: "me@example.com", channel: "INBOX")) throws -> Message {
        let encoder = JSONEncoder()
        let html = String(data: try encoder.encode(html), encoding: .utf8)!
        let ids = String(data: try encoder.encode(contentIDs), encoding: .utf8)!
        let ref = String(data: try encoder.encode(channel), encoding: .utf8)!
        return try QmsgJSON.decode(Message.self, from: """
        {"id":"\(id)","channel":\(ref),"author":"ana@example.com","sent_at":1,"reply_to":null,"content":[],"edited_at":null,"reactions":[],"nonce":null,
         "email":{"subject":"Hi","from":[{"address":"ana@example.com","name":"Ana"}],"to":[],"cc":[],"bcc":[],"reply_to":[],"message_id":null,"in_reply_to":null,"references":[],"html":\(html),"content_ids":\(ids),"read":\(read),"starred":false,"draft":false}}
        """)
    }

    @MainActor @Test func imagesReserveTheirShapeAndShareTheCache() {
        var metadata = MediaMetadata()
        metadata.width = 1600
        metadata.height = 900
        let wide = Media(name: "a.png", mime: "image/png", size: 10, source: .url("https://cdn.example/a.png"), metadata: metadata)
        #expect(wide.aspectRatio == 1600.0 / 900.0)
        metadata.height = 0
        #expect(Media(name: nil, mime: "image/png", size: nil, source: .blob("b"), metadata: metadata).aspectRatio == nil)
        #expect(MediaLoader.key(wide) == "url:https://cdn.example/a.png")
        #expect(MediaLoader.key(Media(name: nil, mime: nil, size: nil, source: .blob("b"))) == "blob:b")
        #expect(MediaLoader.key(Media(name: nil, mime: nil, size: nil, source: .bytes([1]))) == nil)
    }

    private func channel(_ id: String) -> Channel {
        try! QmsgJSON.decode(Channel.self, from: """
        {"id":"\(id)","name":"\(id)","topic":null,"icon":null,"kind":"Text","parent":null,"position":null,"members":null,"accepted_content":[{"kind":"Text","max_size":null,"text_unit":"Chars","mime_types":[]}],"limits":{"max_attachments":null,"max_total_size":null},"mailbox":null}
        """)
    }

    private func message(_ id: String, at time: UInt64) -> Message {
        try! QmsgJSON.decode(Message.self, from: """
        {"id":"\(id)","channel":{"organization":null,"channel":"c"},"author":"bo","sent_at":\(time),"reply_to":null,"content":[{"Text":"x"}],"edited_at":null,"reactions":[],"nonce":null,"email":null}
        """)
    }
}
