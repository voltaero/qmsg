import Foundation

#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

public enum MediaError: Error, CustomStringConvertible, Sendable {
    case expired
    case unsupportedURL
    case tooLarge

    public var description: String {
        switch self {
        case .expired: "This link has expired; reload the conversation."
        case .unsupportedURL: "This file can't be downloaded."
        case .tooLarge: "This file is too large to show."
        }
    }
}

/// Downloads media and keeps recent files in memory.
///
/// Blob handles are read through the app API, then released, since they
/// expire anyway. URLs are public and fetched without any credentials.
@MainActor
public final class MediaLoader {
    private var cache: [String: Data] = [:]
    private var order: [String] = []
    private var bytes = 0
    private var inFlight: [String: Task<Data, any Error>] = [:]
    private let budget = 96 * 1024 * 1024
    public nonisolated static let maxDownload = 64 * 1024 * 1024

    init() {}

    func clear() {
        cache = [:]
        order = []
        bytes = 0
        inFlight = [:]
    }

    func data(for media: Media, client: QmsgClient?) async throws -> Data {
        let key: String
        let load: @Sendable () async throws -> Data
        switch media.source {
        case .bytes(let raw):
            return Data(raw)
        case .blob(let id):
            guard let client else { throw StoreError.notConnected }
            key = Self.key(media)!
            load = {
                let data = try await client.readBlob(id: id, maxBytes: Self.maxDownload)
                Task { try? await client.releaseBlob(id: id) }
                return data
            }
        case .url(let string):
            guard let url = URL(string: string), ["https", "http"].contains(url.scheme?.lowercased()) else {
                throw MediaError.unsupportedURL
            }
            if let expiry = media.metadata.expiresAtMs, expiry < Clock.nowMs() { throw MediaError.expired }
            key = Self.key(media)!
            load = {
                var request = URLRequest(url: url)
                // Never send cookies or credentials to provider URLs.
                request.httpShouldHandleCookies = false
                let (data, response) = try await URLSession.shared.data(for: request)
                if let http = response as? HTTPURLResponse, !(200..<300).contains(http.statusCode) {
                    throw http.statusCode == 403 || http.statusCode == 410 ? MediaError.expired : MediaError.unsupportedURL
                }
                guard data.count <= Self.maxDownload else { throw MediaError.tooLarge }
                return data
            }
        }
        if let hit = cache[key] {
            touch(key)
            return hit
        }
        if let running = inFlight[key] { return try await running.value }
        let task = Task { try await load() }
        inFlight[key] = task
        defer { inFlight[key] = nil }
        let data = try await task.value
        store(key, data)
        return data
    }

    /// The cached bytes for a media item, without loading.
    public func cached(_ media: Media) -> Data? {
        if case .bytes(let raw) = media.source { return Data(raw) }
        return Self.key(media).flatMap { cache[$0] }
    }

    /// What a media item is cached under; nil for inline bytes.
    public nonisolated static func key(_ media: Media) -> String? {
        switch media.source {
        case .bytes: nil
        case .blob(let id): "blob:\(id)"
        case .url(let url): "url:\(url)"
        }
    }

    private func touch(_ key: String) {
        order.removeAll { $0 == key }
        order.append(key)
    }

    private func store(_ key: String, _ data: Data) {
        guard data.count < budget / 4 else { return }
        if let old = cache[key] { bytes -= old.count }
        cache[key] = data
        bytes += data.count
        touch(key)
        while bytes > budget, let oldest = order.first {
            order.removeFirst()
            bytes -= cache.removeValue(forKey: oldest)?.count ?? 0
        }
    }
}
