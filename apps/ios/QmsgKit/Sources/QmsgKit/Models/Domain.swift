import Foundation

// Mirrors of the qmsg-types JSON the app API carries. Field names follow the
// Rust structs; enums use Serde's external tag. See APP_API.md.

public struct ChannelRef: Codable, Hashable, Sendable {
    public var organization: String?
    public var channel: String

    public init(organization: String?, channel: String) {
        self.organization = organization
        self.channel = channel
    }
}

public struct ProviderID: Codable, Hashable, Sendable {
    public var name: String
    public var instance: UInt64

    public init(name: String, instance: UInt64) {
        self.name = name
        self.instance = instance
    }
}

public struct Message: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var channel: ChannelRef
    public var author: String
    /// Milliseconds since the Unix epoch.
    public var sentAt: UInt64
    public var replyTo: String?
    public var content: [Content]
    public var editedAt: UInt64?
    public var reactions: [Reaction]
    public var nonce: String?
    public var email: Email?

    enum CodingKeys: String, CodingKey {
        case id, channel, author, content, reactions, nonce, email
        case sentAt = "sent_at", replyTo = "reply_to", editedAt = "edited_at"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        channel = try c.decode(ChannelRef.self, forKey: .channel)
        author = try c.decode(String.self, forKey: .author)
        sentAt = try c.decode(UInt64.self, forKey: .sentAt)
        replyTo = try c.decodeIfPresent(String.self, forKey: .replyTo)
        content = try c.decode([Content].self, forKey: .content)
        editedAt = try c.decodeIfPresent(UInt64.self, forKey: .editedAt)
        reactions = try c.decodeIfPresent([Reaction].self, forKey: .reactions) ?? []
        nonce = try c.decodeIfPresent(String.self, forKey: .nonce)
        email = try c.decodeIfPresent(Email.self, forKey: .email)
    }

    public var date: Date { Date(timeIntervalSince1970: TimeInterval(sentAt) / 1000) }
}

public struct EmailAddress: Codable, Hashable, Sendable {
    public var address: String
    public var name: String?
}

public struct Email: Codable, Hashable, Sendable {
    public var subject: String
    public var from: [EmailAddress]
    public var to: [EmailAddress]
    public var cc: [EmailAddress]
    public var bcc: [EmailAddress]
    public var replyTo: [EmailAddress]
    public var messageID: String?
    public var inReplyTo: String?
    public var references: [String]
    /// Untrusted; never render without sanitizing.
    public var html: String?
    public var contentIDs: [String: String]
    public var read: Bool
    public var starred: Bool
    public var draft: Bool

    enum CodingKeys: String, CodingKey {
        case subject, from, to, cc, bcc, references, html, read, starred, draft
        case replyTo = "reply_to", messageID = "message_id", inReplyTo = "in_reply_to"
        case contentIDs = "content_ids"
    }
}

public struct Mailbox: Codable, Hashable, Sendable {
    public var delimiter: String?
    public var attributes: [String]
}

public enum Content: Codable, Hashable, Sendable {
    case text(String)
    case formatted(FormattedText)
    case image(Media)
    case video(Media)
    case audio(Media)
    case file(Media)
    case custom(kind: String, data: [UInt8])

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Text": self = .text(try tag.value())
        case "Formatted": self = .formatted(try tag.value())
        case "Image": self = .image(try tag.value())
        case "Video": self = .video(try tag.value())
        case "Audio": self = .audio(try tag.value())
        case "File": self = .file(try tag.value())
        case "Custom":
            let f = try tag.fields()
            self = .custom(kind: try f("kind"), data: try f("data"))
        default: throw tag.unknown(Content.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .text(let v): try encoder.variant("Text", v)
        case .formatted(let v): try encoder.variant("Formatted", v)
        case .image(let v): try encoder.variant("Image", v)
        case .video(let v): try encoder.variant("Video", v)
        case .audio(let v): try encoder.variant("Audio", v)
        case .file(let v): try encoder.variant("File", v)
        case .custom(let kind, let data):
            try encoder.variant("Custom") { try $0.set("kind", kind); try $0.set("data", data) }
        }
    }

    public var media: Media? {
        switch self {
        case .image(let m), .video(let m), .audio(let m), .file(let m): m
        default: nil
        }
    }

    /// The plain text of a text part.
    public var text: String? {
        switch self {
        case .text(let t): t
        case .formatted(let f): f.text
        default: nil
        }
    }
}

public struct Media: Codable, Hashable, Sendable {
    public var name: String?
    public var mime: String?
    public var size: UInt64?
    public var source: MediaSource
    public var metadata: MediaMetadata

    public init(name: String?, mime: String?, size: UInt64?, source: MediaSource, metadata: MediaMetadata = .init()) {
        self.name = name
        self.mime = mime
        self.size = size
        self.source = source
        self.metadata = metadata
    }

    enum CodingKeys: String, CodingKey { case name, mime, size, source, metadata }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decodeIfPresent(String.self, forKey: .name)
        mime = try c.decodeIfPresent(String.self, forKey: .mime)
        size = try c.decodeIfPresent(UInt64.self, forKey: .size)
        source = try c.decode(MediaSource.self, forKey: .source)
        metadata = try c.decodeIfPresent(MediaMetadata.self, forKey: .metadata) ?? .init()
    }
}

public struct MediaMetadata: Codable, Hashable, Sendable {
    public var width: UInt32?
    public var height: UInt32?
    public var durationMs: UInt64?
    public var expiresAtMs: UInt64?
    public var previewURL: String?
    public var description: String?
    public var spoiler: Bool

    public init() { spoiler = false }

    enum CodingKeys: String, CodingKey {
        case width, height, description, spoiler
        case durationMs = "duration_ms", expiresAtMs = "expires_at_ms", previewURL = "preview_url"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        width = try c.decodeIfPresent(UInt32.self, forKey: .width)
        height = try c.decodeIfPresent(UInt32.self, forKey: .height)
        durationMs = try c.decodeIfPresent(UInt64.self, forKey: .durationMs)
        expiresAtMs = try c.decodeIfPresent(UInt64.self, forKey: .expiresAtMs)
        previewURL = try c.decodeIfPresent(String.self, forKey: .previewURL)
        description = try c.decodeIfPresent(String.self, forKey: .description)
        spoiler = try c.decodeIfPresent(Bool.self, forKey: .spoiler) ?? false
    }
}

public enum MediaSource: Codable, Hashable, Sendable {
    case bytes([UInt8])
    /// A public download URL; untrusted.
    case url(String)
    /// A device-owned handle, read with `ReadBlob`.
    case blob(String)

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Bytes": self = .bytes(try tag.value())
        case "Url": self = .url(try tag.value())
        case "Blob": self = .blob(try tag.value())
        default: throw tag.unknown(MediaSource.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .bytes(let v): try encoder.variant("Bytes", v)
        case .url(let v): try encoder.variant("Url", v)
        case .blob(let v): try encoder.variant("Blob", v)
        }
    }
}

public enum ContentKind: Codable, Hashable, Sendable {
    case text, formatted, image, video, audio, file
    case custom(String)

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Text": self = .text
        case "Formatted": self = .formatted
        case "Image": self = .image
        case "Video": self = .video
        case "Audio": self = .audio
        case "File": self = .file
        case "Custom": self = .custom(try tag.value())
        default: throw tag.unknown(ContentKind.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .text: try encoder.unit("Text")
        case .formatted: try encoder.unit("Formatted")
        case .image: try encoder.unit("Image")
        case .video: try encoder.unit("Video")
        case .audio: try encoder.unit("Audio")
        case .file: try encoder.unit("File")
        case .custom(let v): try encoder.variant("Custom", v)
        }
    }
}

public enum TextUnit: String, Codable, Hashable, Sendable {
    case chars = "Chars", utf16 = "Utf16", bytes = "Bytes"

    public func measure(_ text: String) -> Int {
        switch self {
        case .chars: text.unicodeScalars.count
        case .utf16: text.utf16.count
        case .bytes: text.utf8.count
        }
    }
}

public struct ContentRule: Codable, Hashable, Sendable {
    public var kind: ContentKind
    public var maxSize: UInt64?
    public var textUnit: TextUnit
    public var mimeTypes: [String]

    enum CodingKeys: String, CodingKey {
        case kind, maxSize = "max_size", textUnit = "text_unit", mimeTypes = "mime_types"
    }

    /// Whether `mime` matches one of the rule's patterns, such as `image/*`.
    /// An empty list takes any type.
    public func takes(mime: String?) -> Bool {
        if mimeTypes.isEmpty { return true }
        guard let mime = mime?.lowercased() else { return false }
        return mimeTypes.contains { pattern in
            let pattern = pattern.lowercased()
            if pattern.hasSuffix("/*") {
                return mime.split(separator: "/").first.map(String.init) == String(pattern.dropLast(2))
            }
            return pattern == mime
        }
    }
}

public struct MessageLimits: Codable, Hashable, Sendable {
    public var maxAttachments: UInt32?
    public var maxTotalSize: UInt64?

    enum CodingKeys: String, CodingKey {
        case maxAttachments = "max_attachments", maxTotalSize = "max_total_size"
    }
}

public struct User: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var username: String?
    public var avatar: Media?
}

public enum ChannelKind: Codable, Hashable, Sendable {
    case text, voice, video, announcement, forum, thread, category, direct, group
    case custom(String)

    private static let units: [(String, ChannelKind)] = [
        ("Text", .text), ("Voice", .voice), ("Video", .video), ("Announcement", .announcement),
        ("Forum", .forum), ("Thread", .thread), ("Category", .category), ("Direct", .direct),
        ("Group", .group),
    ]

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        if tag.name == "Custom" {
            self = .custom(try tag.value())
        } else if let kind = Self.units.first(where: { $0.0 == tag.name })?.1 {
            self = kind
        } else {
            throw tag.unknown(ChannelKind.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        if case .custom(let name) = self { return try encoder.variant("Custom", name) }
        try encoder.unit(Self.units.first { $0.1 == self }!.0)
    }
}

public struct Channel: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var topic: String?
    public var icon: Media?
    public var kind: ChannelKind
    public var parent: String?
    public var position: UInt32?
    public var members: [String]?
    public var acceptedContent: [ContentRule]
    public var limits: MessageLimits
    public var mailbox: Mailbox?

    enum CodingKeys: String, CodingKey {
        case id, name, topic, icon, kind, parent, position, members, limits, mailbox
        case acceptedContent = "accepted_content"
    }

    public func rule(for kind: ContentKind) -> ContentRule? {
        acceptedContent.first { $0.kind == kind }
    }

    /// Whether anything can be sent here at all.
    public var isWritable: Bool { !acceptedContent.isEmpty }

    /// Holds no messages of its own.
    public var isContainer: Bool { kind == .category || kind == .forum }
}

public enum Presence: String, Codable, Hashable, Sendable {
    case online = "Online", idle = "Idle", busy = "Busy", offline = "Offline"
}

/// The users and channels of an organization, or outside any. Keyed by id.
public struct Scope: Codable, Hashable, Sendable {
    public var me: String?
    public var users: [String: User]
    public var channels: [String: Channel]
    public var presence: [String: Presence]

    public init(me: String? = nil, users: [String: User] = [:], channels: [String: Channel] = [:], presence: [String: Presence] = [:]) {
        self.me = me
        self.users = users
        self.channels = channels
        self.presence = presence
    }

    public var isEmpty: Bool { me == nil && users.isEmpty && channels.isEmpty && presence.isEmpty }
}

/// An organization as the snapshot carries it.
public struct OrganizationEntry: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var icon: Media?
    public var scope: Scope
}

/// An organization as a provider reports it in a directory update.
public struct Organization: Codable, Hashable, Sendable {
    public var id: String
    public var name: String
    public var icon: Media?
    public var me: String?
    public var users: [User]
    public var channels: [Channel]
}

public struct LoginStep: Codable, Hashable, Sendable {
    public var message: String
    /// A login function in the provider context to fill in and call.
    public var function: String?
    /// A page to open in a browser.
    public var link: String?
    /// Text to show as a QR code for the platform's phone app.
    public var qrCode: String?

    enum CodingKeys: String, CodingKey { case message, function, link, qrCode = "qr_code" }
}

public enum ProviderStatus: Codable, Hashable, Sendable {
    case connecting
    case loginRequired(LoginStep)
    case syncing
    case ready
    case disconnected(reason: String)
    case failed(String)

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Connecting": self = .connecting
        case "LoginRequired": self = .loginRequired(try tag.value())
        case "Syncing": self = .syncing
        case "Ready": self = .ready
        case "Disconnected": self = .disconnected(reason: try tag.fields()("reason"))
        case "Failed": self = .failed(try tag.value())
        default: throw tag.unknown(ProviderStatus.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .connecting: try encoder.unit("Connecting")
        case .loginRequired(let s): try encoder.variant("LoginRequired", s)
        case .syncing: try encoder.unit("Syncing")
        case .ready: try encoder.unit("Ready")
        case .disconnected(let reason): try encoder.variant("Disconnected") { try $0.set("reason", reason) }
        case .failed(let s): try encoder.variant("Failed", s)
        }
    }
}

// MARK: Formatting

public struct FormattedText: Codable, Hashable, Sendable {
    public var text: String
    /// Byte ranges into the UTF-8 text.
    public var spans: [Span]

    public init(text: String, spans: [Span] = []) {
        self.text = text
        self.spans = spans
    }
}

public struct Span: Codable, Hashable, Sendable {
    public var start: UInt32
    public var end: UInt32
    public var style: Style
}

public enum Mention: Codable, Hashable, Sendable {
    case user(String), channel(String), role(String), everyone

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "User": self = .user(try tag.value())
        case "Channel": self = .channel(try tag.value())
        case "Role": self = .role(try tag.value())
        case "Everyone": self = .everyone
        default: throw tag.unknown(Mention.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .user(let v): try encoder.variant("User", v)
        case .channel(let v): try encoder.variant("Channel", v)
        case .role(let v): try encoder.variant("Role", v)
        case .everyone: try encoder.unit("Everyone")
        }
    }
}

public enum Style: Codable, Hashable, Sendable {
    case bold, italic, underline, strikethrough, code, quote, spoiler
    case codeBlock(language: String?)
    case link(String)
    case mention(Mention)

    public init(from decoder: Decoder) throws {
        let tag = try Tag(from: decoder)
        switch tag.name {
        case "Bold": self = .bold
        case "Italic": self = .italic
        case "Underline": self = .underline
        case "Strikethrough": self = .strikethrough
        case "Code": self = .code
        case "Quote": self = .quote
        case "Spoiler": self = .spoiler
        case "CodeBlock": self = .codeBlock(language: try tag.fields().optional("language"))
        case "Link": self = .link(try tag.value())
        case "Mention": self = .mention(try tag.value())
        default: throw tag.unknown(Style.self)
        }
    }

    public func encode(to encoder: Encoder) throws {
        switch self {
        case .bold: try encoder.unit("Bold")
        case .italic: try encoder.unit("Italic")
        case .underline: try encoder.unit("Underline")
        case .strikethrough: try encoder.unit("Strikethrough")
        case .code: try encoder.unit("Code")
        case .quote: try encoder.unit("Quote")
        case .spoiler: try encoder.unit("Spoiler")
        case .codeBlock(let language): try encoder.variant("CodeBlock") { try $0.set("language", language) }
        case .link(let v): try encoder.variant("Link", v)
        case .mention(let v): try encoder.variant("Mention", v)
        }
    }
}

public struct Reaction: Codable, Hashable, Sendable {
    public var key: String
    public var count: UInt32
    public var me: Bool
}

extension Mention {
    /// Converts names picked in the composer to their network mention tokens.
    public static func encodePicked(_ text: String, replacements: [String: String]) -> String {
        let names = replacements.keys.filter { !$0.isEmpty }.sorted { $0.count > $1.count }
        guard !names.isEmpty else { return text }
        let alternatives = names.map(NSRegularExpression.escapedPattern(for:)).joined(separator: "|")
        let pattern = "(?<![\\p{L}\\p{N}_])(?:\(alternatives))(?![\\p{L}\\p{N}_])"
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return text }
        let result = NSMutableString(string: text)
        for match in regex.matches(in: text, range: NSRange(text.startIndex..., in: text)).reversed() {
            let shown = (text as NSString).substring(with: match.range)
            if let replacement = replacements[shown] { result.replaceCharacters(in: match.range, with: replacement) }
        }
        return result as String
    }
}
