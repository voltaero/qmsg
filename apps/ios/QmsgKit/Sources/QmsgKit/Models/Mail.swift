import Foundation

/// What a mail folder is for, from its IMAP special-use attributes, or the
/// usual names when the server doesn't label it.
public enum MailboxRole: Int, Comparable, Sendable {
    case inbox, tab, starred, important, drafts, sent, archive, all, junk, trash, other

    public static func < (a: Self, b: Self) -> Bool { a.rawValue < b.rawValue }

    public init(_ channel: Channel) {
        let attributes = Set((channel.mailbox?.attributes ?? []).map { $0.lowercased() })
        let name = channel.mailboxName.lowercased()
        if channel.id.caseInsensitiveCompare("INBOX") == .orderedSame { self = .inbox }
        // Gmail's other inbox tabs, such as Promotions.
        else if attributes.contains("$gmailtab") { self = .tab }
        else if attributes.contains("\\flagged") { self = .starred }
        else if attributes.contains("\\important") { self = .important }
        else if attributes.contains("\\drafts") { self = .drafts }
        else if attributes.contains("\\sent") { self = .sent }
        else if attributes.contains("\\archive") { self = .archive }
        else if attributes.contains("\\all") { self = .all }
        else if attributes.contains("\\junk") { self = .junk }
        else if attributes.contains("\\trash") { self = .trash }
        else {
            switch name {
            case "drafts", "draft": self = .drafts
            case "sent", "sent mail", "sent items", "sent messages": self = .sent
            case "archive": self = .archive
            case "junk", "spam", "junk e-mail", "junk email": self = .junk
            case "trash", "deleted items", "deleted messages", "bin": self = .trash
            default: self = .other
            }
        }
    }

    /// Folders whose mail is the account's own, so lists show recipients.
    public var showsRecipients: Bool { self == .sent || self == .drafts }
}

extension Channel {
    /// A mail folder rather than a chat.
    public var isMailbox: Bool { mailbox != nil || kind == .custom("mailbox") }

    /// The folder's own name, without its parents: "Sent Mail" for "[Gmail]/Sent Mail".
    public var mailboxName: String {
        if id.caseInsensitiveCompare("INBOX") == .orderedSame { return "Inbox" }
        guard let delimiter = mailbox?.delimiter, !delimiter.isEmpty,
              let last = name.components(separatedBy: delimiter).last(where: { !$0.isEmpty }) else { return name }
        return last
    }
}

extension Scope {
    /// Mail folders in the order Mail shows them: Inbox and special folders
    /// first, then the rest by name.
    public var mailboxes: [Channel] {
        channels.values.filter(\.isMailbox).sorted {
            (MailboxRole($0), $0.name.localizedLowercase, $0.id) < (MailboxRole($1), $1.name.localizedLowercase, $1.id)
        }
    }
}

extension EmailAddress {
    /// The name if the sender gave one, otherwise the address.
    public var displayName: String {
        let name = name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return name.isEmpty ? address : name
    }
}

extension Email {
    /// The HTML body as plain text, for previews of mail without a text part.
    public var htmlText: String? {
        guard let html else { return nil }
        var text = html
        for pattern in ["(?is)<(head|style|script|title)[^>]*>.*?</\\1\\s*>", "(?s)<!--.*?-->"] {
            text = text.replacingOccurrences(of: pattern, with: " ", options: .regularExpression)
        }
        text = text.replacingOccurrences(of: "(?i)<br\\s*/?>|</(p|div|li|tr|h[1-6])\\s*>", with: "\n", options: .regularExpression)
        text = text.replacingOccurrences(of: "<[^>]*>", with: " ", options: .regularExpression)
        for (entity, value) in [("&nbsp;", " "), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&apos;", "'"), ("&zwnj;", ""), ("&amp;", "&")] {
            text = text.replacingOccurrences(of: entity, with: value, options: .caseInsensitive)
        }
        text = text.replacingOccurrences(of: "[ \t\u{00A0}\u{200C}\u{034F}]+", with: " ", options: .regularExpression)
        text = text.replacingOccurrences(of: " ?\\n[ \\n]*", with: "\n", options: .regularExpression)
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

extension Message {
    public var mailNeedsBody: Bool {
        content.contains { if case .custom(let kind, _) = $0 { return kind == "email-summary" }; return false }
    }

    /// IMAP message bodies are immutable for an identity; refreshing flags
    /// and headers must not discard a body already fetched for that identity.
    func keepingMailBody(from previous: Message?) -> Message {
        guard mailNeedsBody, let previous, previous.email != nil, !previous.mailNeedsBody else { return self }
        var result = self
        result.content = previous.content
        result.email?.html = previous.email?.html
        result.email?.contentIDs = previous.email?.contentIDs ?? [:]
        return result
    }
    /// The plain text body of a mail, or its HTML as text.
    public var mailBody: String {
        let text = content.compactMap(\.text).joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? (email?.htmlText ?? "") : text
    }

    /// Files attached to a mail, without the images its HTML shows inline.
    public var mailAttachments: [Media] {
        let inline = Set(inlineImages.keys)
        return content.enumerated().compactMap { index, part in inline.contains(index) ? nil : part.media }
    }

    /// Images the HTML body shows through `cid:` links, by content index.
    public var inlineImages: [Int: String] {
        guard let email, let html = email.html?.lowercased() else { return [:] }
        var result: [Int: String] = [:]
        for (key, cid) in email.contentIDs {
            guard let index = Int(key), content.indices.contains(index), case .image = content[index],
                  !cid.isEmpty, html.contains("cid:" + cid.lowercased()) else { continue }
            result[index] = cid
        }
        return result
    }
}
