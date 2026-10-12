import SwiftUI
import UIKit
import QmsgKit

extension Color {
    /// The same canvas behind content and the native glass bars.
    static var appCanvas: Color { Color(uiColor: .secondarySystemBackground) }
}

/// Which tab is showing, and where Settings is, so any screen can send the
/// user to an account that needs them.
@MainActor @Observable
final class AppRouter {
    enum Tab: Hashable { case chats, mail, settings }
    var tab: Tab = .chats
    /// Account names pushed in Settings.
    var settingsPath: [String] = []
    var addingAccount = false

    func show(account name: String) {
        settingsPath = [name]
        tab = .settings
    }

    func addAccount() {
        settingsPath = []
        tab = .settings
        addingAccount = true
    }
}

/// The look of a chat network, picked from its type or account name.
struct NetworkStyle {
    let symbol: String
    let color: Color

    init(_ key: String?) {
        let key = key?.lowercased() ?? ""
        switch true {
        case key.contains("discord"): (symbol, color) = ("gamecontroller.fill", .indigo)
        case key.contains("slack"): (symbol, color) = ("number", .purple)
        case key.contains("signal"): (symbol, color) = ("lock.fill", .blue)
        case key.contains("whatsapp"): (symbol, color) = ("phone.fill", .green)
        case key.contains("telegram"): (symbol, color) = ("paperplane.fill", .cyan)
        case key.contains("mail"), key.contains("imap"): (symbol, color) = ("envelope.fill", .orange)
        case key.contains("matrix"): (symbol, color) = ("square.grid.3x3.fill", .teal)
        default: (symbol, color) = ("bubble.left.fill", .gray)
        }
    }
}

struct NetworkBadge: View {
    let style: NetworkStyle
    var size: CGFloat = 30

    var body: some View {
        Image(systemName: style.symbol)
            .font(.system(size: size * 0.48, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(style.color.gradient, in: RoundedRectangle(cornerRadius: size * 0.28, style: .continuous))
            .accessibilityHidden(true)
    }
}

/// A round avatar with initials, colored by name.
struct InitialsAvatar: View {
    let name: String
    var size: CGFloat = 44
    var neutral = false

    private static let palette: [Color] = [.blue, .indigo, .purple, .teal, .orange, .green]

    private var initials: String {
        let words = name.split(whereSeparator: { $0 == " " || $0 == "-" || $0 == "_" }).prefix(2)
        let letters = words.compactMap(\.first).map(String.init).joined()
        return letters.isEmpty ? "#" : letters.uppercased()
    }

    private var color: Color {
        let sum = name.unicodeScalars.reduce(0) { $0 &+ Int($1.value) }
        return Self.palette[sum % Self.palette.count]
    }

    var body: some View {
        Text(initials)
            .font(.system(size: size * 0.38, weight: .semibold, design: .rounded))
            .foregroundStyle(neutral ? AnyShapeStyle(.secondary) : AnyShapeStyle(color))
            .frame(width: size, height: size)
            .background(neutral ? AnyShapeStyle(.quaternary) : AnyShapeStyle(color.opacity(0.16)), in: Circle())
            .accessibilityHidden(true)
    }
}

/// A plain, neutral tile for servers and workspaces.
struct ServerTile: View {
    let name: String
    var size: CGFloat = 28

    var body: some View {
        Text(name.first.map { String($0).uppercased() } ?? "#")
            .font(.system(size: size * 0.45, weight: .semibold, design: .rounded))
            .foregroundStyle(.secondary)
            .frame(width: size, height: size)
            .background(.quaternary, in: RoundedRectangle(cornerRadius: size * 0.3, style: .continuous))
            .accessibilityHidden(true)
    }
}

/// How an account is doing, in a word, a color, and an optional detail.
struct AccountHealth: Equatable {
    enum Tone { case good, busy, attention, error, off }
    let title: String
    let detail: String?
    let tone: Tone

    init(status: ProviderStatus?, enabled: Bool) {
        guard enabled else { self.init("Off", nil, .off); return }
        switch status {
        case nil: self.init("Starting", nil, .busy)
        case .connecting: self.init("Connecting", nil, .busy)
        case .syncing: self.init("Syncing", nil, .busy)
        case .ready: self.init("Connected", nil, .good)
        case .loginRequired(let step): self.init("Sign in needed", step.message, .attention)
        case .disconnected(let reason): self.init("Disconnected", reason.isEmpty ? nil : reason, .attention)
        case .failed(let reason): self.init("Not working", reason.isEmpty ? nil : reason, .error)
        }
    }

    private init(_ title: String, _ detail: String?, _ tone: Tone) {
        self.title = title
        self.detail = detail
        self.tone = tone
    }

    var color: Color {
        switch tone {
        case .good: .green
        case .busy: .blue
        case .attention: .orange
        case .error: .red
        case .off: .secondary
        }
    }

    var needsUser: Bool { tone == .attention || tone == .error }
}

struct StatusDot: View {
    let health: AccountHealth
    var body: some View {
        HStack(spacing: 5) {
            Circle().fill(health.color).frame(width: 7, height: 7)
            Text(health.title)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }
}

/// A slim banner for the server connection, shown only while it isn't live.
struct ConnectionBanner: View {
    let link: QmsgStore.Link
    var retry: (() -> Void)?

    var body: some View {
        if link != .live {
            HStack(spacing: 10) {
                switch link {
                case .live, .idle:
                    Image(systemName: "wifi.slash")
                    Text("Offline")
                case .connecting:
                    ProgressView().controlSize(.small)
                    Text("Connecting to your server…")
                case .retrying(let error):
                    Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                    VStack(alignment: .leading, spacing: 1) {
                        Text("Can't reach your server")
                        Text(error).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                Spacer(minLength: 0)
                if let retry, link != .connecting {
                    Button("Retry", action: retry).font(.caption.bold()).buttonStyle(.bordered).controlSize(.mini)
                }
            }
            .font(.footnote)
            .padding(.horizontal, 14).padding(.vertical, 9)
            .frame(maxWidth: .infinity, alignment: .leading)
            // The inset also colors the navigation area above it during startup.
            .background(Color.appCanvas)
        }
    }
}

/// An inline error the user can dismiss.
struct ErrorNote: View {
    let message: String
    var dismiss: (() -> Void)?

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(.red)
            Text(message).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
            if let dismiss {
                Button("Dismiss", systemImage: "xmark", action: dismiss)
                    .labelStyle(.iconOnly).foregroundStyle(.secondary)
            }
        }
        .font(.footnote)
    }
}

/// Discord-style mention tokens (`<@id>`, `<@!id>`, `<#id>`) shown as names.
enum Mentions {
    private static let token = try! Regex("<(@!?|@&|#)([0-9]+)>")

    private static func name(_ kind: Substring, _ id: String, _ scope: Scope?) -> String {
        guard let scope else { return kind.hasPrefix("#") ? "#channel" : "@Unknown user" }
        switch kind {
        case "#": return "#" + (scope.channels[id].map { scope.title(of: $0) } ?? "channel")
        case "@&": return "@role"
        default: return "@" + scope.userName(id)
        }
    }

    static func render(_ text: String, scope: Scope?) -> AttributedString {
        var result = AttributedString()
        var rest = text[...]
        for match in text.matches(of: token) {
            result += AttributedString(text[rest.startIndex..<match.range.lowerBound])
            guard let kind = match.output[1].substring, let id = match.output[2].substring else { continue }
            var mention = AttributedString(name(kind, String(id), scope))
            mention.font = .body.weight(.semibold)
            result += mention
            rest = text[match.range.upperBound...]
        }
        result += AttributedString(rest)
        return result
    }

    static func plain(_ text: String, scope: Scope?) -> String {
        var result = ""
        var rest = text[...]
        for match in text.matches(of: token) {
            result += text[rest.startIndex..<match.range.lowerBound]
            if let kind = match.output[1].substring, let id = match.output[2].substring {
                result += name(kind, String(id), scope)
            }
            rest = text[match.range.upperBound...]
        }
        return result + rest
    }
}

extension Message {
    var plainText: String { content.compactMap(\.text).joined(separator: " ") }

    /// A one-line preview for chat lists, with mentions as names.
    func preview(in scope: Scope?) -> String {
        Mentions.plain(preview, scope: scope)
    }

    /// A one-line preview for chat lists.
    var preview: String {
        let text = plainText.trimmingCharacters(in: .whitespacesAndNewlines)
        if !text.isEmpty { return text }
        if let subject = email?.subject, !subject.isEmpty { return subject }
        let files = content.filter { $0.media != nil }.count
        return files == 1 ? "Attachment" : files > 1 ? "\(files) attachments" : "Message"
    }
}

extension Date {
    /// "14:02" today, "Mon" this week, otherwise a short date.
    var chatListStamp: String {
        let calendar = Calendar.current
        if calendar.isDateInToday(self) { return formatted(date: .omitted, time: .shortened) }
        if calendar.isDateInYesterday(self) { return "Yesterday" }
        if let days = calendar.dateComponents([.day], from: self, to: .now).day, days < 7 {
            return formatted(.dateTime.weekday(.abbreviated))
        }
        return formatted(date: .numeric, time: .omitted)
    }
}

// Keep custom chat controls on the same glass surface as the system controls.
extension View {
    @ViewBuilder
    func chatGlass<S: Shape>(in shape: S, tint: Color? = nil, interactive: Bool = false) -> some View {
        if #available(iOS 26.0, *) {
            glassEffect(.regular.tint(tint).interactive(interactive), in: shape)
        } else {
            background(.regularMaterial, in: shape)
        }
    }
}

/// A server icon or avatar the network provides, over a fallback until it
/// loads or when there is none.
struct IconImage<Fallback: View, S: Shape>: View {
    @Environment(QmsgStore.self) private var store
    let media: Media?
    let size: CGFloat
    let shape: S
    @ViewBuilder let fallback: () -> Fallback
    @State private var image: UIImage?

    /// Small, decoded icons, so lists don't decode them again as rows reappear.
    private static var cache: NSCache<NSString, UIImage> { IconCache.shared }

    private var cached: UIImage? {
        guard let media, let key = MediaLoader.key(media) else { return nil }
        return Self.cache.object(forKey: key as NSString)
    }

    var body: some View {
        Group {
            if let image = image ?? cached {
                Image(uiImage: image).resizable().scaledToFill()
                    .frame(width: size, height: size).clipShape(shape)
            } else {
                fallback()
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
        .task(id: media.flatMap(MediaLoader.key)) { await load() }
    }

    private func load() async {
        image = nil
        guard let media, media.mime.map({ $0.hasPrefix("image/") }) ?? true,
              let key = MediaLoader.key(media) else { image = nil; return }
        if let hit = Self.cache.object(forKey: key as NSString) { image = hit; return }
        guard let data = try? await store.data(for: media), let full = UIImage(data: data) else { return }
        let side = size * 3
        let small = await full.byPreparingThumbnail(ofSize: CGSize(width: side, height: side)) ?? full
        guard !Task.isCancelled else { return }
        Self.cache.setObject(small, forKey: key as NSString)
        image = small
    }
}

private enum IconCache {
    nonisolated(unsafe) static let shared: NSCache<NSString, UIImage> = {
        let cache = NSCache<NSString, UIImage>()
        cache.countLimit = 300
        return cache
    }()
}
