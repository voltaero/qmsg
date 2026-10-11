import Foundation

/// One provider's account as the app knows it.
public struct ProviderState: Hashable, Sendable, Identifiable {
    public var id: ProviderID
    public var status: ProviderStatus
    public var organizations: [String: OrganizationEntry]
    public var standalone: Scope

    public var name: String { id.name }

    public init(id: ProviderID, status: ProviderStatus = .connecting, organizations: [String: OrganizationEntry] = [:], standalone: Scope = Scope()) {
        self.id = id
        self.status = status
        self.organizations = organizations
        self.standalone = standalone
    }

    init(_ view: ProviderView) {
        self.init(
            id: view.provider,
            status: view.status,
            organizations: Dictionary(view.organizations.map { ($0.id, $0) }) { $1 },
            standalone: view.standalone ?? Scope()
        )
    }

    public func scope(_ organization: String?) -> Scope? {
        guard let organization else { return standalone }
        return organizations[organization]?.scope
    }

    public func channel(_ ref: ChannelRef) -> Channel? {
        scope(ref.organization)?.channels[ref.channel]
    }

    public func user(_ id: String, in organization: String?) -> User? {
        scope(organization)?.users[id]
    }

    /// Organizations by name.
    public var sortedOrganizations: [OrganizationEntry] {
        organizations.values.sorted { ($0.name.localizedLowercase, $0.id) < ($1.name.localizedLowercase, $1.id) }
    }

    /// Applies a directory update the way the orchestrator does. Updates
    /// for unknown organizations change nothing.
    public mutating func apply(_ update: DirectoryUpdate) {
        switch update {
        case .organizationUpserted(let o):
            organizations[o.id] = OrganizationEntry(
                id: o.id, name: o.name, icon: o.icon,
                scope: Scope(
                    me: o.me,
                    users: Dictionary(o.users.map { ($0.id, $0) }) { $1 },
                    channels: Dictionary(o.channels.map { ($0.id, $0) }) { $1 }
                )
            )
        case .organizationUpdated(let id, let name, let icon):
            organizations[id]?.name = name
            organizations[id]?.icon = icon
        case .organizationRemoved(let id):
            organizations[id] = nil
        case .userUpserted(let org, let user):
            modify(org) { $0.users[user.id] = user }
        case .userRemoved(let org, let id):
            modify(org) { scope in
                scope.users[id] = nil
                scope.presence[id] = nil
                if scope.me == id { scope.me = nil }
            }
        case .channelUpserted(let org, let channel):
            modify(org) { $0.channels[channel.id] = channel }
        case .channelRemoved(let ref):
            modify(ref.organization) { $0.channels[ref.channel] = nil }
        case .me(let org, let id):
            modify(org) { $0.me = id }
        case .presence(let org, let user, let presence):
            modify(org) { $0.presence[user] = presence }
        case .reset:
            organizations = [:]
            standalone = Scope()
        }
    }

    private mutating func modify(_ organization: String?, _ change: (inout Scope) -> Void) {
        if let organization {
            guard organizations[organization] != nil else { return }
            change(&organizations[organization]!.scope)
        } else {
            change(&standalone)
        }
    }
}

extension ProviderStatus {
    public var isReady: Bool { self == .ready }

    public var label: String {
        switch self {
        case .connecting: "Connecting"
        case .loginRequired: "Login required"
        case .syncing: "Syncing"
        case .ready: "Connected"
        case .disconnected(let reason): reason.isEmpty ? "Disconnected" : "Disconnected: \(reason)"
        case .failed(let reason): "Failed: \(reason)"
        }
    }

    public var loginStep: LoginStep? {
        if case .loginRequired(let step) = self { return step }
        return nil
    }
}

/// Identifies a channel across providers.
public struct ChannelKey: Hashable, Sendable, Codable {
    public var provider: String
    public var channel: ChannelRef

    public init(provider: String, channel: ChannelRef) {
        self.provider = provider
        self.channel = channel
    }
}

extension Channel {
    /// Sort key among siblings: position, then name.
    public var sortKey: (UInt32, String, String) {
        (position ?? .max, name.localizedLowercase, id)
    }
}

/// A channel in a scope's tree, with how deeply it is nested.
public struct ChannelNode: Identifiable, Hashable, Sendable {
    public var channel: Channel
    public var depth: Int
    public var id: String { channel.id }
}

extension Scope {
    /// A name suitable for chat labels, even before a profile arrives.
    public func userName(_ id: String) -> String {
        guard let user = users[id] else { return "Unknown user" }
        let name = user.name.trimmingCharacters(in: .whitespacesAndNewlines)
        if !name.isEmpty && name != id { return name }
        if let username = user.username, !username.isEmpty { return username }
        return "Unknown user"
    }

    /// The channels of this scope as a tree: top-level channels and
    /// categories, each followed by its children.
    public func channelTree() -> [ChannelNode] {
        let children = Dictionary(grouping: channels.values) { $0.parent.flatMap { channels[$0] != nil ? $0 : nil } }
        var result: [ChannelNode] = []
        func visit(_ parent: String?, depth: Int) {
            for channel in (children[parent] ?? []).sorted(by: { $0.sortKey < $1.sortKey }) {
                result.append(ChannelNode(channel: channel, depth: depth))
                if depth < 4 { visit(channel.id, depth: depth + 1) }
            }
        }
        visit(nil, depth: 0)
        return result
    }

    /// A display name for a channel: its name, or for a direct or group
    /// conversation without one, its other members.
    public func title(of channel: Channel) -> String {
        if !channel.name.isEmpty { return channel.name }
        let others = (channel.members ?? []).filter { $0 != me }.map { userName($0) }
        return others.isEmpty ? channel.id : others.joined(separator: ", ")
    }
}
