import Foundation
import Observation

/// A send the app is waiting on, shown in its channel until the provider
/// reports the message.
public struct PendingSend: Identifiable, Hashable, Sendable {
    public enum State: Hashable, Sendable {
        case sending
        /// Definitely not sent; safe to try again.
        case failed(String)
        /// Maybe sent. Check the conversation before sending again.
        case unknown(String)
    }

    /// The send's nonce.
    public var id: String
    public var content: [Content]
    public var replyTo: String?
    public var operation: ActionOperation
    public var state: State
    /// The id the provider answered with, once it has.
    public var sentID: String?
    /// Local previews of attachments, by content index.
    public var previews: [Int: Data]
    /// Original files, kept until the send settles so failed uploads can retry.
    public var attachments: [QmsgStore.Attachment] = []

    public var date: Date { Date(timeIntervalSince1970: TimeInterval(operation.createdAtMs) / 1000) }
}

/// The loaded messages of one channel.
@MainActor @Observable
public final class Conversation: Identifiable {
    public let key: ChannelKey
    public nonisolated var id: ChannelKey { key }

    /// Oldest first, unique by id.
    public private(set) var messages: [Message] = []
    public internal(set) var pending: [PendingSend] = []
    public private(set) var olderCursor: String?
    public private(set) var reachedStart = false
    public internal(set) var isLoading = false
    public internal(set) var isLoadingOlder = false
    public internal(set) var loadError: String?
    public private(set) var loaded = false
    /// Users typing, until when.
    public private(set) var typing: [String: Date] = [:]
    /// The account's own read marker, if the provider reports one.
    public private(set) var readUpTo: String?
    /// Needs a reload: after a gap or a fresh snapshot.
    public internal(set) var stale = false

    init(key: ChannelKey) { self.key = key }

    public func message(_ id: String) -> Message? {
        messages.first { $0.id == id }
    }

    public var typingUsers: [String] {
        let now = Date()
        return typing.filter { $0.value > now }.map(\.key).sorted()
    }

    /// Bumped when loaded history is dropped for a page it doesn't join up
    /// with, so older pages requested before can be told apart.
    public private(set) var generation = 0
    /// Ids changed and deleted by events while a newest page is on its way.
    /// That page may predate them, so they win over it.
    private var during: (changed: Set<String>, deleted: Set<String>)?

    /// Another newest page is wanted once the running load finishes.
    var reloadQueued = false

    /// Call before requesting the newest page that `replace` will take.
    func beginLoad() { during = ([], []) }

    /// Call when a load ends without a page.
    func endLoad() { during = nil }

    /// Takes the newest page. Messages older than it are kept when the page
    /// joins up with them; within the page's span the page is the truth,
    /// so messages missing from it were deleted.
    func replace(with page: HistoryPage) {
        let changed = during?.changed ?? [], deleted = during?.deleted ?? []
        during = nil
        let incoming = Self.sorted(page.messages)
        let pageIDs = Set(incoming.map(\.id))
        let current = Dictionary(messages.map { ($0.id, $0) }) { $1 }
        var result: [Message] = []
        if let oldest = incoming.first, page.nextCursor != nil, messages.contains(where: { pageIDs.contains($0.id) }) {
            // Joins up: keep older history and its cursor.
            result = messages.filter { Self.precedes($0, oldest) }
        } else {
            if !messages.isEmpty { generation += 1 }
            olderCursor = page.nextCursor
            reachedStart = page.nextCursor == nil
        }
        // Prefer what events said since the request went out.
        result += incoming.map { changed.contains($0.id) ? current[$0.id] ?? $0 : $0 }
        result += messages.filter { changed.contains($0.id) && !pageIDs.contains($0.id) }
        messages = Self.sorted(Self.unique(result.filter { !deleted.contains($0.id) }))
        loaded = true
        stale = false
        loadError = nil
        settlePending()
    }

    /// Adds an older page. Returns false, changing nothing, if the history
    /// it was requested for has since been replaced.
    @discardableResult
    func prepend(_ page: HistoryPage, generation: Int? = nil) -> Bool {
        if let generation, generation != self.generation { return false }
        merge(page.messages)
        olderCursor = page.nextCursor
        reachedStart = page.nextCursor == nil
        return true
    }

    func merge(_ incoming: [Message]) {
        var byID = Dictionary(messages.map { ($0.id, $0) }) { $1 }
        for message in incoming { byID[message.id] = message }
        messages = Self.sorted(Array(byID.values))
        settlePending()
    }

    static func sorted(_ messages: [Message]) -> [Message] {
        messages.sorted(by: precedes)
    }

    static func precedes(_ a: Message, _ b: Message) -> Bool {
        (a.sentAt, a.id) < (b.sentAt, b.id)
    }

    private static func unique(_ messages: [Message]) -> [Message] {
        var seen = Set<String>()
        return messages.filter { seen.insert($0.id).inserted }
    }

    /// Drops pending sends the provider has reported.
    private func settlePending() {
        let nonces = Set(messages.compactMap(\.nonce))
        let ids = Set(messages.map(\.id))
        pending.removeAll { nonces.contains($0.id) || ($0.sentID.map(ids.contains) ?? false) }
    }

    func apply(_ event: MessageEvent, me: String?) {
        if during != nil {
            switch event {
            case .received(let message):
                during?.changed.insert(message.id)
                during?.deleted.remove(message.id)
            case .edited(_, let id, _, _), .reacted(_, let id, _, _, _):
                during?.changed.insert(id)
            case .deleted(_, let id):
                during?.deleted.insert(id)
            default:
                break
            }
        }
        switch event {
        case .received(let message):
            typing[message.author] = nil
            if loaded || during != nil { merge([message]) } else { settleNonce(message) }
        case .edited(_, let id, let content, let editedAt):
            guard let i = messages.firstIndex(where: { $0.id == id }) else { return }
            messages[i].content = content
            messages[i].editedAt = editedAt
        case .deleted(_, let id):
            messages.removeAll { $0.id == id }
        case .reacted(_, let id, let user, let key, let added):
            guard let i = messages.firstIndex(where: { $0.id == id }) else { return }
            var reactions = messages[i].reactions
            let mine = user == me
            if let r = reactions.firstIndex(where: { $0.key == key }) {
                if added {
                    reactions[r].count += 1
                    if mine { reactions[r].me = true }
                } else {
                    reactions[r].count = reactions[r].count > 0 ? reactions[r].count - 1 : 0
                    if mine { reactions[r].me = false }
                    if reactions[r].count == 0 { reactions.remove(at: r) }
                }
            } else if added {
                reactions.append(Reaction(key: key, count: 1, me: mine))
            }
            messages[i].reactions = reactions
        case .read(_, let user, let upTo):
            if user == me { readUpTo = upTo }
        case .typing(_, let user):
            if user != me { typing[user] = Date().addingTimeInterval(10) }
        case .gap:
            stale = true
        case .notSent(_, let nonce, let error):
            if let i = pending.firstIndex(where: { $0.id == nonce }) {
                pending[i].state = .failed(error.description)
            }
        }
    }

    private func settleNonce(_ message: Message) {
        pending.removeAll { $0.id == message.nonce || $0.sentID == message.id }
    }

    func markSent(nonce: String, id: String) {
        guard let i = pending.firstIndex(where: { $0.id == nonce }) else { return }
        pending[i].sentID = id
        if messages.contains(where: { $0.id == id }) { pending.remove(at: i) }
    }

    func update(nonce: String, _ change: (inout PendingSend) -> Void) {
        guard let i = pending.firstIndex(where: { $0.id == nonce }) else { return }
        change(&pending[i])
    }
}

/// What the chat list shows for a channel.
public struct ChannelActivity: Hashable, Sendable {
    public var last: Message?
    public var unread: Int
}
