import Foundation

/// The `command_json` of an `ExecuteRequest`.
public enum Command: Encodable, Hashable, Sendable {
    case functions(context: FunctionContext)
    case call(FunctionCall)
    case verify(VerificationRequest)
    case complete(CompletionRequest)
    /// `limit` must be 1...100. Pages sort oldest first; the cursor asks for older ones.
    case history(channel: ChannelRef, cursor: String?, limit: UInt32)
    case getMessage(channel: ChannelRef, id: String)
    case send(channel: ChannelRef, replyTo: String?, content: [Content], nonce: String?)
    case openChannel(organization: String?, members: [String])

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .functions(let context):
            try encoder.variant("Functions") { try $0.set("context", context) }
        case .call(let call):
            try encoder.variant("Call") {
                try $0.set("function", call.function)
                try $0.set("context", call.context)
                try $0.set("arguments", call.arguments)
            }
        case .verify(let v): try encoder.variant("Verify", v)
        case .complete(let c): try encoder.variant("Complete", c)
        case .history(let channel, let cursor, let limit):
            try encoder.variant("History") {
                try $0.set("channel", channel); try $0.set("cursor", cursor); try $0.set("limit", limit)
            }
        case .getMessage(let channel, let id):
            try encoder.variant("GetMessage") { try $0.set("channel", channel); try $0.set("id", id) }
        case .send(let channel, let replyTo, let content, let nonce):
            try encoder.variant("Send") {
                try $0.set("channel", channel); try $0.set("reply_to", replyTo)
                try $0.set("content", content); try $0.set("nonce", nonce)
            }
        case .openChannel(let organization, let members):
            try encoder.variant("OpenChannel") { try $0.set("organization", organization); try $0.set("members", members) }
        }
    }

    /// Whether the server journals this command as an action. Calls are
    /// actions when their function is; the caller knows that from discovery.
    public var isAction: Bool {
        switch self {
        case .send, .openChannel: true
        default: false
        }
    }
}

public struct HistoryPage: Codable, Hashable, Sendable {
    /// Oldest first.
    public var messages: [Message]
    /// For older messages; nil at the start of the channel.
    public var nextCursor: String?

    enum CodingKeys: String, CodingKey { case messages, nextCursor = "next_cursor" }
}

public enum Reply: Decodable, Sendable {
    case functions([Function])
    case value(Value)
    case verified(Verification)
    case completed(CompletionPage)
    case sent(id: String)
    case opened(ChannelRef)
    case history(HistoryPage)
    case message(Message)
    case blob([UInt8])

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Functions": self = .functions(try tag.value())
        case "Value": self = .value(try tag.value())
        case "Verified": self = .verified(try tag.value())
        case "Completed": self = .completed(try tag.value())
        case "Sent": self = .sent(id: try tag.fields()("id"))
        case "Opened": self = .opened(try tag.value())
        case "History": self = .history(try tag.value())
        case "Message": self = .message(try tag.value())
        case "Blob": self = .blob(try tag.value())
        default: throw tag.unknown(Reply.self)
        }
    }
}

/// One provider in a directory snapshot.
public struct ProviderView: Codable, Hashable, Sendable {
    public var provider: ProviderID
    public var status: ProviderStatus
    public var organizations: [OrganizationEntry]
    public var standalone: Scope?
}

public struct Snapshot: Codable, Hashable, Sendable {
    public var providers: [ProviderView]
    public var epoch: String
    public var sequence: UInt64
}

/// What scanning a pairing QR code yields.
public struct PairingPayload: Codable, Hashable, Sendable {
    public var version: Int
    public var endpoint: String
    public var code: String
    public var expiresAtMs: UInt64
    public var certificatePEM: String?
    public var certificateSHA256: String?

    enum CodingKeys: String, CodingKey {
        case version, endpoint, code
        case expiresAtMs = "expires_at_ms"
        case certificatePEM = "certificate_pem"
        case certificateSHA256 = "certificate_sha256"
    }

    public var expiresAt: Date { Date(timeIntervalSince1970: TimeInterval(expiresAtMs) / 1000) }
}
