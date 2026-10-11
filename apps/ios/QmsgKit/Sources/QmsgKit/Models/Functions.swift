import Foundation

public typealias Arguments = [String: Value]

/// A function argument or result.
public indirect enum Value: Codable, Hashable, Sendable {
    case null
    case boolean(Bool)
    case integer(Int64)
    case text(String)
    case bytes([UInt8])
    case list([Value])
    case record(Arguments)
    case content([Content])
    case channel(ChannelRef)
    case message(Message)

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Null": self = .null
        case "Boolean": self = .boolean(try tag.value())
        case "Integer": self = .integer(try tag.value())
        case "Text": self = .text(try tag.value())
        case "Bytes": self = .bytes(try tag.value())
        case "List": self = .list(try tag.value())
        case "Record": self = .record(try tag.value())
        case "Content": self = .content(try tag.value())
        case "Channel": self = .channel(try tag.value())
        case "Message": self = .message(try tag.value())
        default: throw tag.unknown(Value.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .null: try encoder.unit("Null")
        case .boolean(let v): try encoder.variant("Boolean", v)
        case .integer(let v): try encoder.variant("Integer", v)
        case .text(let v): try encoder.variant("Text", v)
        case .bytes(let v): try encoder.variant("Bytes", v)
        case .list(let v): try encoder.variant("List", v)
        case .record(let v): try encoder.variant("Record", v)
        case .content(let v): try encoder.variant("Content", v)
        case .channel(let v): try encoder.variant("Channel", v)
        case .message(let v): try encoder.variant("Message", v)
        }
    }

    public subscript(field: String) -> Value? {
        if case .record(let fields) = self { return fields[field] }
        return nil
    }

    public var textValue: String? {
        if case .text(let t) = self { return t }
        return nil
    }

    public var integerValue: Int64? {
        if case .integer(let i) = self { return i }
        return nil
    }

    /// A short human-readable rendering, for showing results.
    public var summary: String {
        switch self {
        case .null: "Done"
        case .boolean(let b): b ? "Yes" : "No"
        case .integer(let i): String(i)
        case .text(let t): t
        case .bytes(let b): "\(b.count) bytes"
        case .list(let items): items.map(\.summary).joined(separator: ", ")
        case .record(let fields):
            fields.keys.sorted().map { "\($0): \(fields[$0]!.summary)" }.joined(separator: "\n")
        case .content(let parts): parts.compactMap(\.text).joined(separator: " ")
        case .channel(let c): c.channel
        case .message(let m): m.content.compactMap(\.text).joined(separator: " ")
        }
    }
}

public indirect enum ValueType: Codable, Hashable, Sendable {
    case any, null, boolean, integer, text, bytes, content, channel, message
    case list(ValueType)
    case record([Field])

    private static let units: [(String, ValueType)] = [
        ("Any", .any), ("Null", .null), ("Boolean", .boolean), ("Integer", .integer),
        ("Text", .text), ("Bytes", .bytes), ("Content", .content), ("Channel", .channel),
        ("Message", .message),
    ]

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "List": self = .list(try tag.value())
        case "Record": self = .record(try tag.value())
        default:
            guard let unit = Self.units.first(where: { $0.0 == tag.name })?.1 else {
                throw tag.unknown(ValueType.self)
            }
            self = unit
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .list(let v): try encoder.variant("List", v)
        case .record(let v): try encoder.variant("Record", v)
        default: try encoder.unit(Self.units.first { $0.1 == self }!.0)
        }
    }
}

public struct Field: Codable, Hashable, Sendable {
    public var id: String
    public var label: String
    public var valueType: ValueType
    public var required: Bool

    enum CodingKeys: String, CodingKey { case id, label, required, valueType = "value_type" }
}

public enum FunctionContext: Codable, Hashable, Sendable {
    case provider
    case organization(String)
    case channel(ChannelRef)
    case message(channel: ChannelRef, id: String)

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Provider": self = .provider
        case "Organization": self = .organization(try tag.value())
        case "Channel": self = .channel(try tag.value())
        case "Message":
            let f = try tag.fields()
            self = .message(channel: try f("channel"), id: try f("id"))
        default: throw tag.unknown(FunctionContext.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .provider: try encoder.unit("Provider")
        case .organization(let v): try encoder.variant("Organization", v)
        case .channel(let v): try encoder.variant("Channel", v)
        case .message(let channel, let id):
            try encoder.variant("Message") { try $0.set("channel", channel); try $0.set("id", id) }
        }
    }
}

/// Standard input ids of the action contracts.
public enum StandardInput {
    public static let content = "content"
    public static let replyTo = "reply_to"
    public static let nonce = "nonce"
    public static let members = "members"
    public static let reaction = "reaction"
}

public enum ActionKind: Codable, Hashable, Sendable {
    case sendMessage, editMessage, deleteMessage, openChannel, addReaction, removeReaction
    case markRead, markUnread, typing, login, logout
    case custom(String)

    private static let units: [(String, ActionKind)] = [
        ("SendMessage", .sendMessage), ("EditMessage", .editMessage), ("DeleteMessage", .deleteMessage),
        ("OpenChannel", .openChannel), ("AddReaction", .addReaction), ("RemoveReaction", .removeReaction),
        ("MarkRead", .markRead), ("MarkUnread", .markUnread), ("Typing", .typing), ("Login", .login),
        ("Logout", .logout),
    ]

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        if tag.name == "Custom" {
            self = .custom(try tag.value())
        } else if let kind = Self.units.first(where: { $0.0 == tag.name })?.1 {
            self = kind
        } else {
            throw tag.unknown(ActionKind.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        if case .custom(let name) = self { return try encoder.variant("Custom", name) }
        try encoder.unit(Self.units.first { $0.1 == self }!.0)
    }
}

public enum FunctionKind: Codable, Hashable, Sendable {
    case action(ActionKind, result: ValueType)
    case lookup(ValueType)
    case verification(ValueType)
    case completion(ValueType)

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Action":
            let f = try tag.fields()
            self = .action(try f("action"), result: try f("result"))
        case "Lookup": self = .lookup(try tag.value())
        case "Verification": self = .verification(try tag.value())
        case "Completion": self = .completion(try tag.value())
        default: throw tag.unknown(FunctionKind.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .action(let action, let result):
            try encoder.variant("Action") { try $0.set("action", action); try $0.set("result", result) }
        case .lookup(let v): try encoder.variant("Lookup", v)
        case .verification(let v): try encoder.variant("Verification", v)
        case .completion(let v): try encoder.variant("Completion", v)
        }
    }
}

public struct Choice: Codable, Hashable, Sendable {
    public var label: String
    public var value: Value
}

public enum InputHint: Codable, Hashable, Sendable {
    case plain, secret, multiline
    case choices([Choice])

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Plain": self = .plain
        case "Secret": self = .secret
        case "Multiline": self = .multiline
        case "Choices": self = .choices(try tag.value())
        default: throw tag.unknown(InputHint.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .plain: try encoder.unit("Plain")
        case .secret: try encoder.unit("Secret")
        case .multiline: try encoder.unit("Multiline")
        case .choices(let v): try encoder.variant("Choices", v)
        }
    }
}

public struct FunctionInput: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var label: String
    public var description: String
    public var valueType: ValueType
    public var required: Bool
    public var hint: InputHint
    public var verification: String?
    public var completion: String?

    enum CodingKeys: String, CodingKey {
        case id, label, description, required, hint, verification, completion
        case valueType = "value_type"
    }
}

public struct Function: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var label: String
    public var description: String
    public var kind: FunctionKind
    public var inputs: [FunctionInput]

    public var action: ActionKind? {
        if case .action(let action, _) = kind { return action }
        return nil
    }

    public var isAction: Bool { action != nil }
}

public struct FunctionCall: Codable, Hashable, Sendable {
    public var function: String
    public var context: FunctionContext
    public var arguments: Arguments

    public init(function: String, context: FunctionContext, arguments: Arguments) {
        self.function = function
        self.context = context
        self.arguments = arguments
    }
}

public struct FunctionInputRef: Codable, Hashable, Sendable {
    public var function: String
    public var input: String

    public init(function: String, input: String) {
        self.function = function
        self.input = input
    }
}

public struct VerificationRequest: Codable, Hashable, Sendable {
    public var call: FunctionCall
    public var input: FunctionInputRef?
    public var value: Value

    public init(call: FunctionCall, input: FunctionInputRef?, value: Value) {
        self.call = call
        self.input = input
        self.value = value
    }
}

public struct CompletionRequest: Codable, Hashable, Sendable {
    public var call: FunctionCall
    public var input: FunctionInputRef?
    public var query: String
    public var cursor: String?
    public var limit: UInt32

    public init(call: FunctionCall, input: FunctionInputRef?, query: String, cursor: String? = nil, limit: UInt32 = 20) {
        self.call = call
        self.input = input
        self.query = query
        self.cursor = cursor
        self.limit = limit
    }
}

public struct InputIssue: Codable, Hashable, Sendable {
    public var input: String?
    public var code: String
    public var message: String
}

public enum Verification: Codable, Hashable, Sendable {
    case valid
    case invalid([InputIssue])

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Valid": self = .valid
        case "Invalid": self = .invalid(try tag.value())
        default: throw tag.unknown(Verification.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .valid: try encoder.unit("Valid")
        case .invalid(let v): try encoder.variant("Invalid", v)
        }
    }
}

public struct CompletionItem: Codable, Hashable, Sendable {
    public var label: String
    public var description: String?
    public var value: Value
}

public struct CompletionPage: Codable, Hashable, Sendable {
    public var items: [CompletionItem]
    public var nextCursor: String?

    enum CodingKeys: String, CodingKey { case items, nextCursor = "next_cursor" }
}
