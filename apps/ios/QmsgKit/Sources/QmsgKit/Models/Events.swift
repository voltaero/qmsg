import Foundation

public enum MessageEvent: Codable, Hashable, Sendable {
    case received(Message)
    case edited(channel: ChannelRef, id: String, content: [Content], editedAt: UInt64)
    case deleted(channel: ChannelRef, id: String)
    case reacted(channel: ChannelRef, id: String, user: String, key: String, added: Bool)
    case read(channel: ChannelRef, user: String, upTo: String?)
    case typing(channel: ChannelRef, user: String)
    /// Events may be missing; reload the channel's history.
    case gap(channel: ChannelRef)
    /// The send with this nonce never reached the platform.
    case notSent(channel: ChannelRef, nonce: String, error: CommandError)

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        if tag.name == "Received" {
            self = .received(try tag.value())
            return
        }
        let f = try tag.fields()
        switch tag.name {
        case "Edited":
            self = .edited(channel: try f("channel"), id: try f("id"), content: try f("content"), editedAt: try f("edited_at"))
        case "Deleted": self = .deleted(channel: try f("channel"), id: try f("id"))
        case "Reacted":
            self = .reacted(channel: try f("channel"), id: try f("id"), user: try f("user"), key: try f("key"), added: try f("added"))
        case "Read": self = .read(channel: try f("channel"), user: try f("user"), upTo: try f.optional("up_to"))
        case "Typing": self = .typing(channel: try f("channel"), user: try f("user"))
        case "Gap": self = .gap(channel: try f("channel"))
        case "NotSent": self = .notSent(channel: try f("channel"), nonce: try f("nonce"), error: try f("error"))
        default: throw tag.unknown(MessageEvent.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .received(let m): try encoder.variant("Received", m)
        case .edited(let channel, let id, let content, let editedAt):
            try encoder.variant("Edited") {
                try $0.set("channel", channel); try $0.set("id", id)
                try $0.set("content", content); try $0.set("edited_at", editedAt)
            }
        case .deleted(let channel, let id):
            try encoder.variant("Deleted") { try $0.set("channel", channel); try $0.set("id", id) }
        case .reacted(let channel, let id, let user, let key, let added):
            try encoder.variant("Reacted") {
                try $0.set("channel", channel); try $0.set("id", id); try $0.set("user", user)
                try $0.set("key", key); try $0.set("added", added)
            }
        case .read(let channel, let user, let upTo):
            try encoder.variant("Read") { try $0.set("channel", channel); try $0.set("user", user); try $0.set("up_to", upTo) }
        case .typing(let channel, let user):
            try encoder.variant("Typing") { try $0.set("channel", channel); try $0.set("user", user) }
        case .gap(let channel):
            try encoder.variant("Gap") { try $0.set("channel", channel) }
        case .notSent(let channel, let nonce, let error):
            try encoder.variant("NotSent") { try $0.set("channel", channel); try $0.set("nonce", nonce); try $0.set("error", error) }
        }
    }

    public var channel: ChannelRef {
        switch self {
        case .received(let m): m.channel
        case .edited(let c, _, _, _), .deleted(let c, _), .reacted(let c, _, _, _, _), .read(let c, _, _),
             .typing(let c, _), .gap(let c), .notSent(let c, _, _): c
        }
    }
}

public enum DirectoryUpdate: Codable, Hashable, Sendable {
    case organizationUpserted(Organization)
    case organizationUpdated(id: String, name: String, icon: Media?)
    case organizationRemoved(id: String)
    case userUpserted(organization: String?, user: User)
    case userRemoved(organization: String?, id: String)
    case channelUpserted(organization: String?, channel: Channel)
    case channelRemoved(ChannelRef)
    case me(organization: String?, id: String?)
    case presence(organization: String?, user: String, presence: Presence)
    case reset

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Reset": self = .reset; return
        case "OrganizationUpserted": self = .organizationUpserted(try tag.value()); return
        case "ChannelRemoved": self = .channelRemoved(try tag.value()); return
        default: break
        }
        let f = try tag.fields()
        switch tag.name {
        case "OrganizationUpdated": self = .organizationUpdated(id: try f("id"), name: try f("name"), icon: try f.optional("icon"))
        case "OrganizationRemoved": self = .organizationRemoved(id: try f("id"))
        case "UserUpserted": self = .userUpserted(organization: try f.optional("organization"), user: try f("user"))
        case "UserRemoved": self = .userRemoved(organization: try f.optional("organization"), id: try f("id"))
        case "ChannelUpserted": self = .channelUpserted(organization: try f.optional("organization"), channel: try f("channel"))
        case "Me": self = .me(organization: try f.optional("organization"), id: try f.optional("id"))
        case "Presence":
            self = .presence(organization: try f.optional("organization"), user: try f("user"), presence: try f("presence"))
        default: throw tag.unknown(DirectoryUpdate.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .reset: try encoder.unit("Reset")
        case .organizationUpserted(let o): try encoder.variant("OrganizationUpserted", o)
        case .channelRemoved(let c): try encoder.variant("ChannelRemoved", c)
        case .organizationUpdated(let id, let name, let icon):
            try encoder.variant("OrganizationUpdated") { try $0.set("id", id); try $0.set("name", name); try $0.set("icon", icon) }
        case .organizationRemoved(let id):
            try encoder.variant("OrganizationRemoved") { try $0.set("id", id) }
        case .userUpserted(let org, let user):
            try encoder.variant("UserUpserted") { try $0.set("organization", org); try $0.set("user", user) }
        case .userRemoved(let org, let id):
            try encoder.variant("UserRemoved") { try $0.set("organization", org); try $0.set("id", id) }
        case .channelUpserted(let org, let channel):
            try encoder.variant("ChannelUpserted") { try $0.set("organization", org); try $0.set("channel", channel) }
        case .me(let org, let id):
            try encoder.variant("Me") { try $0.set("organization", org); try $0.set("id", id) }
        case .presence(let org, let user, let presence):
            try encoder.variant("Presence") { try $0.set("organization", org); try $0.set("user", user); try $0.set("presence", presence) }
        }
    }
}

/// One update from `Watch`.
public enum ProviderEvent: Decodable, Sendable {
    case message(provider: ProviderID, event: MessageEvent)
    case directory(provider: ProviderID, update: DirectoryUpdate)
    case status(provider: ProviderID, status: ProviderStatus)
    case exited(provider: ProviderID, error: String?)

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        let f = try tag.fields()
        let provider: ProviderID = try f("provider")
        switch tag.name {
        case "Message": self = .message(provider: provider, event: try f("event"))
        case "Directory": self = .directory(provider: provider, update: try f("update"))
        case "Status": self = .status(provider: provider, status: try f("status"))
        case "Exited":
            let result: [String: String?] = try f("result")
            self = .exited(provider: provider, error: result["Err"] ?? nil)
        default: throw tag.unknown(ProviderEvent.self)
        }
    }

    public var provider: ProviderID {
        switch self {
        case .message(let p, _), .directory(let p, _), .status(let p, _), .exited(let p, _): p
        }
    }
}

/// Why a provider couldn't carry out a command.
public enum CommandError: Codable, Hashable, Sendable, CustomStringConvertible {
    case unsupported
    case unknownFunction(String)
    case invalidInput([InputIssue])
    case provider(code: String, message: String, details: Value?)
    case unknownOrganization(String)
    case unknownChannel(ChannelRef)
    case unknownUser(String)
    case unknownMessage(String)
    case unknownBlob(String)
    case rejected(JSON)
    case rateLimited(retryAfterMs: UInt64?)
    case forbidden(String)
    case loginRequired
    case cancelled
    case failed(String)

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Unsupported": self = .unsupported
        case "UnknownFunction": self = .unknownFunction(try tag.value())
        case "InvalidInput": self = .invalidInput(try tag.value())
        case "Provider":
            let f = try tag.fields()
            self = .provider(code: try f("code"), message: try f("message"), details: try f.optional("details"))
        case "UnknownOrganization": self = .unknownOrganization(try tag.value())
        case "UnknownChannel": self = .unknownChannel(try tag.value())
        case "UnknownUser": self = .unknownUser(try tag.value())
        case "UnknownMessage": self = .unknownMessage(try tag.value())
        case "UnknownBlob": self = .unknownBlob(try tag.value())
        case "Rejected": self = .rejected(try tag.value())
        case "RateLimited": self = .rateLimited(retryAfterMs: try tag.fields().optional("retry_after_ms"))
        case "Forbidden": self = .forbidden(try tag.value())
        case "LoginRequired": self = .loginRequired
        case "Cancelled": self = .cancelled
        case "Failed": self = .failed(try tag.value())
        default: throw tag.unknown(CommandError.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .unsupported: try encoder.unit("Unsupported")
        case .unknownFunction(let v): try encoder.variant("UnknownFunction", v)
        case .invalidInput(let v): try encoder.variant("InvalidInput", v)
        case .provider(let code, let message, let details):
            try encoder.variant("Provider") { try $0.set("code", code); try $0.set("message", message); try $0.set("details", details) }
        case .unknownOrganization(let v): try encoder.variant("UnknownOrganization", v)
        case .unknownChannel(let v): try encoder.variant("UnknownChannel", v)
        case .unknownUser(let v): try encoder.variant("UnknownUser", v)
        case .unknownMessage(let v): try encoder.variant("UnknownMessage", v)
        case .unknownBlob(let v): try encoder.variant("UnknownBlob", v)
        case .rejected(let v): try encoder.variant("Rejected", v)
        case .rateLimited(let ms): try encoder.variant("RateLimited") { try $0.set("retry_after_ms", ms) }
        case .forbidden(let v): try encoder.variant("Forbidden", v)
        case .loginRequired: try encoder.unit("LoginRequired")
        case .cancelled: try encoder.unit("Cancelled")
        case .failed(let v): try encoder.variant("Failed", v)
        }
    }

    public var description: String {
        switch self {
        case .unsupported: "Not supported"
        case .unknownFunction(let id): "Unknown function “\(id)”"
        case .invalidInput(let issues):
            issues.isEmpty ? "Invalid input" : issues.map { issue in
                issue.input.map { "\($0): \(issue.message)" } ?? issue.message
            }.joined(separator: "\n")
        case .provider(_, let message, _): message
        case .unknownOrganization: "Unknown organization"
        case .unknownChannel: "Unknown channel"
        case .unknownUser: "Unknown user"
        case .unknownMessage: "Unknown message"
        case .unknownBlob: "That file is no longer available"
        case .rejected(let violation): "Rejected: \(Self.describe(violation))"
        case .rateLimited(let ms):
            ms.map { "Rate limited; try again in \(max(1, $0 / 1000)) s" } ?? "Rate limited; try again later"
        case .forbidden(let reason): "Not allowed: \(reason)"
        case .loginRequired: "Log in first"
        case .cancelled: "Cancelled"
        case .failed(let reason): reason
        }
    }

    /// Readable text for a `Violation`.
    static func describe(_ violation: JSON) -> String {
        func number(_ json: JSON?) -> String {
            switch json {
            case .int(let v): String(v)
            case .uint(let v): String(v)
            default: "?"
            }
        }
        switch violation {
        case .string("Empty"): return "the message is empty"
        case .object(let o) where o.count == 1:
            let (name, value) = o.first!
            let f: [String: JSON] = if case .object(let f) = value { f } else { [:] }
            switch name {
            case "TextTooLong": return "the text is too long (\(number(f["length"])) of \(number(f["max"])))"
            case "TooLarge": return "a file is too large"
            case "TooManyAttachments": return "too many attachments (max \(number(f["max"])))"
            case "TotalTooLarge": return "the files are too large together"
            case "Unsupported": return "this channel doesn't take that kind of content"
            case "UnsupportedMime": return "this channel doesn't take that file type"
            case "InvalidFormatting": return "invalid formatting"
            default: return name
            }
        default: return "the message breaks the channel's rules"
        }
    }
}

/// The `error_json` of an `Outcome`.
public struct AppError: Codable, Hashable, Sendable, Error, CustomStringConvertible {
    public var code: String
    public var message: String
    public var command: CommandError?
    /// The action may or may not have taken effect.
    public var deliveryUnknown: Bool

    enum CodingKeys: String, CodingKey { case code, message, command, deliveryUnknown = "delivery_unknown" }

    public var description: String { command?.description ?? message }
}
