import Foundation
import GRPCCore
import Observation

public enum StoreError: Error, CustomStringConvertible, Sendable {
    case notConnected
    case unknownProvider(String)
    case unsupported(String)

    public var description: String {
        switch self {
        case .notConnected: "Not connected to the server."
        case .unknownProvider(let name): "The account “\(name)” is no longer available."
        case .unsupported(let what): what
        }
    }
}

/// Everything the app shows, kept current by one `Watch` stream.
@MainActor @Observable
public final class QmsgStore {
    public enum Link: Equatable, Sendable {
        case idle
        case connecting
        case live
        case retrying(String)
    }

    public private(set) var account: Account?
    /// The device token was rejected; pair again.
    public private(set) var needsPairing = false
    public private(set) var link: Link = .idle
    public private(set) var providers: [String: ProviderState] = [:]
    public private(set) var activity: [ChannelKey: ChannelActivity] = [:]
    /// The channel on screen. Its messages don't count as unread.
    public var visible: ChannelKey? {
        didSet {
            if let visible {
                if oldValue != visible { remember(visible) }
                activity[visible, default: .init(last: nil, unread: 0)].unread = 0
                if conversations[visible]?.stale == true { Task { await loadLatest(visible) } }
            }
        }
    }

    public let media: MediaLoader
    private let accounts: any AccountStore
    private var client: QmsgClient?
    /// Created on demand while views render, so not observed itself; each
    /// conversation is observable.
    @ObservationIgnored private var conversations: [ChannelKey: Conversation] = [:]
    private var watchTask: Task<Void, Never>?
    private var opened: [ChannelKey] = []
    private var epoch = ""
    private var sequence: UInt64 = 0

    public init(accounts: any AccountStore) {
        self.accounts = accounts
        media = MediaLoader()
        account = try? accounts.load()
    }

    public var sortedProviders: [ProviderState] {
        providers.values.sorted { $0.name.localizedLowercase < $1.name.localizedLowercase }
    }

    /// Recently opened chats keep their order when messages arrive elsewhere.
    public var recentKeys: [ChannelKey] {
        opened.filter { providers[$0.provider]?.channel($0.channel) != nil }.prefix(8).map { $0 }
    }

    private func remember(_ key: ChannelKey) {
        opened.removeAll { $0 == key }
        opened.insert(key, at: 0)
        if opened.count > 32 { opened.removeLast() }
    }

    public var isPaired: Bool { account != nil && !needsPairing }

    // MARK: Pairing and lifecycle

    /// Pairs with the backend in a scanned QR code, replacing any account.
    public func pair(code text: String, deviceName: String) async throws {
        let payload = try PairingError.payload(from: text.trimmingCharacters(in: .whitespacesAndNewlines))
        let endpoint = try ServerEndpoint(payload: payload)
        let client = try QmsgClient(endpoint: endpoint)
        let credential = try await client.pair(code: payload.code, deviceName: deviceName)
        let account = Account(endpoint: endpoint, deviceID: credential.deviceID, token: credential.token, expiresAtMs: credential.expiresAtMs)
        try accounts.save(account)
        stop()
        reset()
        client.setToken(account.token)
        self.account = account
        self.client = client
        needsPairing = false
        startWatching()
    }

    /// Connects with the saved account, if any. Call when the app becomes active.
    public func resume() {
        guard watchTask == nil, let account, !needsPairing else { return }
        if client == nil || client?.endpoint != account.endpoint {
            client = try? QmsgClient(endpoint: account.endpoint, token: account.token)
        }
        Task { await maintainToken() }
        startWatching()
    }

    /// Stops the event stream, such as when the app goes to the background.
    /// The cursor is kept, so `resume` replays what was missed.
    public func suspend() {
        watchTask?.cancel()
        watchTask = nil
        link = .idle
    }

    /// Revokes this device on the server, when reachable, and forgets it.
    public func unpair() async {
        if let account, !needsPairing {
            try? await client?.revoke(deviceID: account.deviceID)
        }
        stop()
        try? accounts.delete()
        account = nil
        needsPairing = false
        reset()
    }

    private func stop() {
        suspend()
        client?.close()
        client = nil
    }

    private func reset() {
        providers = [:]
        activity = [:]
        opened = []
        conversations = [:]
        epoch = ""
        sequence = 0
        media.clear()
    }

    public func providerCatalog() async throws -> ProviderCatalog {
        guard let client else { throw StoreError.notConnected }
        return try await client.providerCatalog()
    }

    public func configureProvider(name: String, type: String, settings: [String: String], enabled: Bool, label: String? = nil) async throws {
        guard let client else { throw StoreError.notConnected }
        try await client.configureProvider(name: name, type: type, settings: settings, enabled: enabled, label: label)
        apply(try await client.snapshot())
    }

    public func removeProvider(name: String) async throws {
        guard let client else { throw StoreError.notConnected }
        try await client.removeProvider(name: name)
        apply(try await client.snapshot())
    }

    // MARK: Event stream

    private func startWatching() {
        guard watchTask == nil else { return }
        watchTask = Task { [weak self] in await self?.watchLoop() }
    }

    private func watchLoop() async {
        var failures = 0
        while !Task.isCancelled, let client {
            link = failures == 0 ? .connecting : link
            do {
                for try await event in client.watch(epoch: epoch, after: sequence) {
                    guard !Task.isCancelled else { return }
                    failures = 0
                    link = .live
                    apply(event)
                }
            } catch is CancellationError {
                return
            } catch {
                if Task.isCancelled { return }
                if error.isUnauthenticated, !(await recoverToken()) {
                    needsPairing = true
                    link = .idle
                    watchTask = nil
                    return
                }
                link = .retrying(Self.describe(error))
            }
            failures += 1
            // Backoff: 0.5 s doubling to 30 s, with jitter.
            let delay = min(30, 0.5 * pow(2, Double(min(failures, 6)))) * Double.random(in: 0.8...1.2)
            try? await Task.sleep(for: .seconds(delay))
        }
    }

    func apply(_ event: WatchEvent) {
        switch event {
        case .snapshot(let snapshot):
            apply(snapshot)
        case .update(let update, let epoch, let sequence):
            guard epoch == self.epoch, sequence > self.sequence else { return }
            self.sequence = sequence
            if let update { apply(update) }
        case .heartbeat:
            break
        }
    }

    private func apply(_ snapshot: Snapshot) {
        guard snapshot.epoch != epoch || snapshot.sequence >= sequence else { return }
        epoch = snapshot.epoch
        sequence = snapshot.sequence
        providers = Dictionary(snapshot.providers.map { ($0.provider.name, ProviderState($0)) }) { $1 }
        // The snapshot has no messages: reload what is on screen, and the rest when shown.
        for conversation in conversations.values { conversation.stale = true }
        if let visible { Task { await loadLatest(visible) } }
    }

    private func apply(_ event: ProviderEvent) {
        let id = event.provider
        if providers[id.name]?.id != id {
            providers[id.name] = ProviderState(id: id)
        }
        switch event {
        case .directory(_, let update):
            providers[id.name]?.apply(update)
        case .status(_, let status):
            providers[id.name]?.status = status
        case .exited(_, let error):
            providers[id.name] = ProviderState(id: id, status: .failed(error ?? "The account stopped."))
        case .message(_, let event):
            let key = ChannelKey(provider: id.name, channel: event.channel)
            let me = providers[id.name]?.scope(event.channel.organization)?.me
            conversations[key]?.apply(event, me: me)
            switch event {
            case .received(let message):
                var entry = activity[key] ?? .init(last: nil, unread: 0)
                if (entry.last?.sentAt ?? 0) <= message.sentAt { entry.last = message }
                if message.author != me && key != visible && message.nonce == nil { entry.unread += 1 }
                activity[key] = entry
            case .deleted(_, let id) where activity[key]?.last?.id == id:
                activity[key]?.last = nil
            case .read(_, let user, _) where user == me:
                activity[key]?.unread = 0
            case .gap where key == visible:
                Task { await loadLatest(key) }
            default:
                break
            }
        }
    }

    // MARK: Token maintenance

    /// Rotates a token that expires within 30 days, persisting the new
    /// secret first so a lost reply can be recovered.
    func maintainToken() async {
        guard var account, let client else { return }
        if account.pendingSecret != nil {
            _ = await recoverToken()
            return
        }
        guard account.expiresAt.timeIntervalSinceNow < 30 * 24 * 3600 else { return }
        let secret = Self.randomSecret()
        account.pendingSecret = secret
        guard (try? accounts.save(account)) != nil else { return }
        self.account = account
        do {
            let credential = try await client.rotate(secret: secret)
            adopt(token: credential.token, expiresAtMs: credential.expiresAtMs)
        } catch {
            _ = await recoverToken()
        }
    }

    /// After a rejected token or a lost rotation reply: tries the pending
    /// rotation's token. Returns whether a working token is in place.
    private func recoverToken() async -> Bool {
        guard var account, let client else { return false }
        guard let proposed = account.proposedToken else { return false }
        do {
            _ = try await client.snapshot(token: proposed)
            // The rotation went through. Its expiry is 90 days from then.
            adopt(token: proposed, expiresAtMs: Clock.nowMs() + 89 * 24 * 3600 * 1000)
            return true
        } catch where error.isUnauthenticated {
            // It didn't; the old token may still work.
            account.pendingSecret = nil
            try? accounts.save(account)
            self.account = account
            do {
                _ = try await client.snapshot()
                return true
            } catch {
                return !error.isUnauthenticated
            }
        } catch {
            // Unreachable; decide later.
            return true
        }
    }

    private func adopt(token: String, expiresAtMs: UInt64) {
        guard var account else { return }
        account.token = token
        account.expiresAtMs = expiresAtMs
        account.pendingSecret = nil
        try? accounts.save(account)
        self.account = account
        client?.setToken(token)
    }

    static func randomSecret() -> String {
        var generator = SystemRandomNumberGenerator()
        return (0..<32).map { _ in String(format: "%02x", UInt8.random(in: .min ... .max, using: &generator)) }.joined()
    }

    // MARK: Commands

    /// Runs a command on a provider's current instance. Reads, and actions
    /// with their journaled operation, are retried after transient failures.
    public func execute(_ command: Command, provider name: String, operation: ActionOperation? = nil, timeout: Duration = .seconds(60)) async throws -> ExecuteResult {
        var attempt = 0
        while true {
            guard let client else { throw StoreError.notConnected }
            guard let provider = providers[name] else { throw StoreError.unknownProvider(name) }
            do {
                return try await client.execute(command, provider: provider.id, operation: operation, timeout: timeout)
            } catch where attempt < 3 && Self.isRetryable(error) {
                attempt += 1
                if error.rpcCode == .failedPrecondition {
                    // The provider restarted; refresh its instance, then retry.
                    if let snapshot = try? await client.snapshot() { apply(snapshot) }
                } else {
                    try await Task.sleep(for: .seconds(Double(attempt)))
                }
            }
        }
    }

    static func isRetryable(_ error: any Error) -> Bool {
        switch error.rpcCode {
        case .unavailable, .resourceExhausted, .failedPrecondition: true
        default: false
        }
    }

    public func conversation(_ key: ChannelKey) -> Conversation {
        if let existing = conversations[key] { return existing }
        let conversation = Conversation(key: key)
        conversations[key] = conversation
        return conversation
    }

    public func loadIfNeeded(_ key: ChannelKey) async {
        let conversation = conversation(key)
        guard !conversation.loaded || conversation.stale else { return }
        await loadLatest(key)
    }

    /// Loads the newest page of a channel, replacing what was loaded.
    public func loadLatest(_ key: ChannelKey) async {
        let conversation = conversation(key)
        guard !conversation.isLoading else { return }
        conversation.isLoading = true
        defer { conversation.isLoading = false }
        do {
            let result = try await execute(.history(channel: key.channel, cursor: nil, limit: 50), provider: key.provider)
            guard case .history(let page) = try result.result.get() else { return }
            conversation.replace(with: page)
            if let last = page.messages.last {
                var entry = activity[key] ?? .init(last: nil, unread: 0)
                if (entry.last?.sentAt ?? 0) <= last.sentAt { entry.last = last }
                activity[key] = entry
            }
        } catch {
            conversation.loadError = Self.describe(error)
        }
    }

    /// Loads the page before the oldest loaded message.
    public func loadOlder(_ key: ChannelKey) async {
        let conversation = conversation(key)
        guard let cursor = conversation.olderCursor, !conversation.isLoadingOlder, !conversation.isLoading else { return }
        conversation.isLoadingOlder = true
        defer { conversation.isLoadingOlder = false }
        do {
            let result = try await execute(.history(channel: key.channel, cursor: cursor, limit: 50), provider: key.provider)
            guard case .history(let page) = try result.result.get() else { return }
            conversation.prepend(page)
        } catch {
            conversation.loadError = Self.describe(error)
        }
    }

    // MARK: Sending

    /// A file to send.
    public struct Attachment: Hashable, Sendable {
        public var name: String
        public var mime: String
        public var data: Data

        public init(name: String, mime: String, data: Data) {
            self.name = name
            self.mime = mime
            self.data = data
        }

        var kind: ContentKind {
            switch mime.split(separator: "/").first {
            case "image": .image
            case "video": .video
            case "audio": .audio
            default: .file
            }
        }
    }

    // 128 random bits in 22 URL-safe characters, within Discord's 25-character limit.
    private static func sendNonce() -> String {
        var generator = SystemRandomNumberGenerator()
        return Data((0..<16).map { _ in UInt8.random(in: .min ... .max, using: &generator) })
            .base64EncodedString().replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }

    /// Sends text and files to a channel. The send shows as pending until
    /// the provider reports the message.
    public func send(text: String, attachments: [Attachment] = [], replyTo: String? = nil, to key: ChannelKey) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        var content: [Content] = []
        if !trimmed.isEmpty { content.append(.text(trimmed)) }
        var previews: [Int: Data] = [:]
        for attachment in attachments {
            if attachment.kind == .image { previews[content.count] = attachment.data }
            // A placeholder until the upload finishes.
            content.append(Self.content(attachment.kind, Media(name: attachment.name, mime: attachment.mime, size: UInt64(attachment.data.count), source: .bytes([]))))
        }
        guard !content.isEmpty else { return }
        let nonce = Self.sendNonce()
        let pending = PendingSend(id: nonce, content: content, replyTo: replyTo, operation: ActionOperation(), state: .sending, previews: previews, attachments: attachments)
        conversation(key).pending.append(pending)
        Task { await deliver(pending, attachments: attachments, to: key) }
    }

    /// Tries a failed or uncertain send again, as a new operation.
    public func retry(_ pending: PendingSend, in key: ChannelKey) {
        let conversation = conversation(key)
        conversation.pending.removeAll { $0.id == pending.id }
        var again = pending
        again.id = Self.sendNonce()
        again.operation = ActionOperation()
        again.state = .sending
        again.sentID = nil
        conversation.pending.append(again)
        Task { await deliver(again, attachments: again.attachments, to: key) }
    }

    public func discard(_ pending: PendingSend, in key: ChannelKey) {
        conversation(key).pending.removeAll { $0.id == pending.id }
    }

    private func deliver(_ pending: PendingSend, attachments: [Attachment], to key: ChannelKey) async {
        let conversation = conversation(key)
        var content = pending.content
        var submitted = false
        do {
            // Upload files first; the media handles are tied to this provider instance.
            var uploads = attachments.makeIterator()
            for (i, part) in content.enumerated() {
                guard part.media != nil else { continue }
                // Keep the original bytes until delivery settles. A retry must
                // upload again because a previous handle may have expired.
                guard let attachment = uploads.next() else { continue }
                guard let client, let provider = providers[key.provider] else { throw StoreError.notConnected }
                let uploaded = try await client.upload(provider: provider.id, name: attachment.name, mime: attachment.mime, data: attachment.data)
                content[i] = Self.content(attachment.kind, uploaded)
            }
            conversation.update(nonce: pending.id) { $0.content = content }
            submitted = true
            let result = try await execute(
                .send(channel: key.channel, replyTo: pending.replyTo, content: content, nonce: pending.id),
                provider: key.provider, operation: pending.operation
            )
            switch result.result {
            case .success(.sent(let id)):
                conversation.markSent(nonce: pending.id, id: id)
            case .success:
                conversation.update(nonce: pending.id) { $0.state = .unknown("Unexpected reply") }
            case .failure(let error):
                conversation.update(nonce: pending.id) {
                    $0.state = error.deliveryUnknown ? .unknown(error.description) : .failed(error.description)
                }
            }
        } catch {
            // Without an answer, the journal can't say whether it was sent.
            let message = Self.describe(error)
            conversation.update(nonce: pending.id) {
                $0.state = !submitted || error is StoreError || error.rpcCode == .invalidArgument || $0.content.contains(where: Self.isUnsentUpload) ? .failed(message) : .unknown(message)
            }
        }
    }

    private static func isUnsentUpload(_ content: Content) -> Bool {
        if case .bytes = content.media?.source { return true }
        return false
    }

    static func content(_ kind: ContentKind, _ media: Media) -> Content {
        switch kind {
        case .image: .image(media)
        case .video: .video(media)
        case .audio: .audio(media)
        default: .file(media)
        }
    }

    // MARK: Functions

    /// Functions available in a context, such as a message's actions.
    public func functions(_ context: FunctionContext, provider: String) async throws -> [Function] {
        let result = try await execute(.functions(context: context), provider: provider)
        guard case .functions(let functions) = try result.result.get() else { return [] }
        return functions
    }

    /// Calls a discovered function. Actions are journaled with `operation`;
    /// pass the same one to retry the same call.
    public func call(_ function: Function, context: FunctionContext, arguments: Arguments, provider: String, operation: ActionOperation? = nil, timeout: Duration = .seconds(60)) async throws -> Result<Value, AppError> {
        let operation = function.isAction ? (operation ?? ActionOperation()) : nil
        let call = FunctionCall(function: function.id, context: context, arguments: arguments)
        let result = try await execute(.call(call), provider: provider, operation: operation, timeout: timeout)
        return result.result.flatMap { reply in
            if case .value(let value) = reply { return .success(value) }
            return .success(.null)
        }
    }

    /// Suggestions for a function input.
    public func complete(_ function: Function, input: FunctionInput, context: FunctionContext, arguments: Arguments, query: String, provider: String) async throws -> [CompletionItem] {
        guard let helper = input.completion else { return [] }
        let request = CompletionRequest(
            call: FunctionCall(function: helper, context: context, arguments: arguments),
            input: FunctionInputRef(function: function.id, input: input.id),
            query: query, limit: 20
        )
        let result = try await execute(.complete(request), provider: provider)
        guard case .completed(let page) = try result.result.get() else { return [] }
        return page.items
    }

    /// Opens a conversation with members, or finds the one already open.
    public func openChannel(provider: String, organization: String?, members: [String]) async throws -> ChannelKey {
        let result = try await execute(.openChannel(organization: organization, members: members), provider: provider, operation: ActionOperation())
        guard case .opened(let ref) = try result.result.get() else { throw StoreError.unsupported("Unexpected reply") }
        return ChannelKey(provider: provider, channel: ref)
    }

    /// Marks a channel read up to its newest message, when the provider
    /// offers a read action there.
    public func markRead(_ key: ChannelKey) async {
        guard let last = conversations[key]?.messages.last else { return }
        let context = FunctionContext.message(channel: key.channel, id: last.id)
        guard let functions = try? await functions(context, provider: key.provider),
              let markRead = functions.first(where: { $0.action == .markRead })
        else { return }
        _ = try? await call(markRead, context: context, arguments: [:], provider: key.provider)
    }

    // MARK: Devices

    public func devices() async throws -> [PairedDevice] {
        guard let client else { throw StoreError.notConnected }
        return try await client.devices()
    }

    public func revoke(_ device: PairedDevice) async throws {
        guard let client else { throw StoreError.notConnected }
        try await client.revoke(deviceID: device.id)
        if device.id == account?.deviceID {
            await unpair()
        }
    }

    // MARK: Media

    /// The bytes of a media item from a provider.
    public func data(for item: Media) async throws -> Data {
        try await media.data(for: item, client: client)
    }

    // MARK: Helpers

    public static func describe(_ error: any Error) -> String {
        if let error = error as? RPCError {
            switch error.code {
            case .unavailable: return "Can't reach the server."
            case .deadlineExceeded: return "The server took too long to answer."
            case .unauthenticated: return "This device is no longer paired."
            case .resourceExhausted: return error.message.isEmpty ? "The server is busy." : error.message
            default: return error.message.isEmpty ? "\(error.code)" : error.message
            }
        }
        switch error {
        case let error as AppError: return error.description
        case let error as PairingError: return error.description
        case let error as StoreError: return error.description
        case let error as MediaError: return error.description
        default: return error.localizedDescription
        }
    }
}
