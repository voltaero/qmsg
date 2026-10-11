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

    func replace(with page: HistoryPage) {
        messages = Self.sorted(page.messages)
        olderCursor = page.nextCursor
        reachedStart = page.nextCursor == nil
        loaded = true
        stale = false
        loadError = nil
        settlePending()
    }

    func prepend(_ page: HistoryPage) {
        merge(page.messages)
        olderCursor = page.nextCursor
        reachedStart = page.nextCursor == nil
    }

    func merge(_ incoming: [Message]) {
        var byID = Dictionary(messages.map { ($0.id, $0) }) { $1 }
        for message in incoming { byID[message.id] = message }
        messages = Self.sorted(Array(byID.values))
        settlePending()
    }

    static func sorted(_ messages: [Message]) -> [Message] {
        messages.sorted { ($0.sentAt, $0.id) < ($1.sentAt, $1.id) }
    }

    /// Drops pending sends the provider has reported.
    private func settlePending() {
        let nonces = Set(messages.compactMap(\.nonce))
        let ids = Set(messages.map(\.id))
        pending.removeAll { nonces.contains($0.id) || ($0.sentID.map(ids.contains) ?? false) }
    }

    func apply(_ event: MessageEvent, me: String?) {
        switch event {
        case .received(let message):
            typing[message.author] = nil
            if loaded { merge([message]) } else { settleNonce(message) }
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
