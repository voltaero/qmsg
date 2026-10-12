import Foundation
import GRPCCore
import GRPCNIOTransportHTTP2
import NIOCore
import NIOSSL
import Synchronization

/// One event from `Watch`, with the cursor to resume after it.
public enum WatchEvent: Sendable {
    case snapshot(Snapshot)
    case update(ProviderEvent?, epoch: String, sequence: UInt64)
    case heartbeat(epoch: String, sequence: UInt64)
}

/// A journaled action's identity. Keep it, unchanged, across retries of the
/// same request.
public struct ActionOperation: Codable, Hashable, Sendable {
    public var id: String
    public var createdAtMs: UInt64

    public init(id: String = UUID().uuidString.lowercased(), createdAtMs: UInt64 = Clock.nowMs()) {
        self.id = id
        self.createdAtMs = createdAtMs
    }
}

public enum Clock {
    public static func nowMs() -> UInt64 { UInt64(Date().timeIntervalSince1970 * 1000) }
}

public struct ExecuteResult: Sendable {
    public var result: Result<Reply, AppError>
    public var epoch: String
    /// The event position after events queued before the provider answered.
    public var observedSequence: UInt64
}

public struct PairedDevice: Hashable, Sendable, Identifiable {
    public var id: String
    public var name: String
    public var expiresAtMs: UInt64
}

public struct Credential: Hashable, Sendable {
    public var deviceID: String
    public var token: String
    public var expiresAtMs: UInt64
}

/// The app API over one HTTP/2 connection to a paired backend.
public final class QmsgClient: Sendable {
    public typealias Transport = HTTP2ClientTransport.Posix

    public let endpoint: ServerEndpoint
    private let grpc: GRPCClient<Transport>
    private let app: Qmsg_App_V1_App.Client<Transport>
    private let token: Mutex<String?>

    /// Responses can be up to 8 MiB, such as a large history page.
    private static let maxResponse = 8 * 1024 * 1024
    static let chunk = 256 * 1024

    public init(endpoint: ServerEndpoint, token: String? = nil) throws {
        self.endpoint = endpoint
        self.token = Mutex(token)
        var config = Transport.Config.defaults
        config.connection.keepalive = .init(time: .seconds(30), timeout: .seconds(10), allowWithoutCalls: false)
        config.backoff = .init(initial: .milliseconds(500), max: .seconds(15), multiplier: 1.6, jitter: 0.2)
        let target: any ResolvableTarget = switch ServerEndpoint.ipBytes(endpoint.host)?.count {
        case 4: .ipv4(address: endpoint.host, port: endpoint.port)
        case 16: .ipv6(address: endpoint.host, port: endpoint.port)
        default: .dns(host: endpoint.host, port: endpoint.port)
        }
        let transport = try Transport(target: target, transportSecurity: Self.security(endpoint), config: config)
        grpc = GRPCClient(transport: transport)
        app = Qmsg_App_V1_App.Client(wrapping: grpc)
        let grpc = grpc
        Task.detached { try? await grpc.runConnections() }
    }

    private static func security(_ endpoint: ServerEndpoint) -> Transport.TransportSecurity {
        guard endpoint.secure else { return .plaintext }
        guard let pinned = endpoint.certificateSHA256 else {
            // A public certificate, such as a tunnel's, uses platform validation.
            return .tls
        }
        let host = endpoint.host
        return .tls { tls in
            tls.serverCertificateVerification = .fullVerification
            tls.customVerificationCallback = { certificates, promise in
                guard let leaf = certificates.first, let der = try? leaf.toDERBytes(),
                      ServerEndpoint.verify(leaf: der, pinned: pinned, host: host)
                else { return promise.succeed(.failed) }
                promise.succeed(.certificateVerified(VerificationMetadata(nil)))
            }
        }
    }

    deinit { grpc.beginGracefulShutdown() }

    public func close() { grpc.beginGracefulShutdown() }

    public func setToken(_ token: String?) { self.token.withLock { $0 = token } }

    private func auth(_ token: String? = nil) -> Metadata {
        guard let token = token ?? self.token.withLock({ $0 }) else { return [:] }
        return ["authorization": "Bearer \(token)"]
    }

    private func options(timeout: Duration? = .seconds(30)) -> CallOptions {
        var options = CallOptions.defaults
        options.timeout = timeout
        options.maxResponseMessageBytes = Self.maxResponse
        return options
    }

    // MARK: Devices

    /// Redeems a one-time pairing code. Needs no token.
    public func pair(code: String, deviceName: String) async throws -> Credential {
        var request = Qmsg_App_V1_PairRequest()
        request.code = code
        request.deviceName = deviceName
        return Credential(try await app.pair(request, options: options()))
    }

    /// Replaces the token. Pass a persisted secret so a lost reply can be
    /// recovered; see `Account.pendingSecret`.
    public func rotate(secret: String) async throws -> Credential {
        var request = Qmsg_App_V1_RotateRequest()
        request.replacementSecret = secret
        return Credential(try await app.rotate(request, metadata: auth(), options: options()))
    }

    public func devices() async throws -> [PairedDevice] {
        try await app.devices(Qmsg_App_V1_Empty(), metadata: auth(), options: options()).devices.map {
            PairedDevice(id: $0.id, name: $0.name, expiresAtMs: $0.expiresAtMs)
        }
    }

    public func revoke(deviceID: String) async throws {
        var request = Qmsg_App_V1_RevokeRequest()
        request.deviceID = deviceID
        _ = try await app.revoke(request, metadata: auth(), options: options())
    }

    // MARK: Accounts

    public func providerCatalog() async throws -> ProviderCatalog {
        let data = try await app.providerCatalog(Qmsg_App_V1_Empty(), metadata: auth(), options: options())
        return try QmsgJSON.decode(ProviderCatalog.self, from: data.json)
    }

    public func configureProvider(name: String, type: String, settings: [String: String], enabled: Bool, label: String? = nil) async throws {
        var request = Qmsg_App_V1_ProviderConfiguration()
        request.name = name
        request.type = type
        request.label = label ?? ""
        request.enabled = enabled
        request.settingsJson = try QmsgJSON.encode(settings)
        _ = try await app.configureProvider(request, metadata: auth(), options: options())
    }

    public func removeProvider(name: String) async throws {
        var request = Qmsg_App_V1_ProviderName()
        request.name = name
        _ = try await app.removeProvider(request, metadata: auth(), options: options())
    }

    // MARK: State

    /// The current directory. `token` overrides the client's, to probe a
    /// rotated token.
    public func snapshot(token: String? = nil) async throws -> Snapshot {
        let data = try await app.snapshot(Qmsg_App_V1_Empty(), metadata: auth(token), options: options())
        return try QmsgJSON.decode(Snapshot.self, from: data.json)
    }

    /// Events after `(epoch, sequence)`, or a snapshot first when the server
    /// can't resume from there. Runs until cancelled or the stream fails.
    public func watch(epoch: String, after sequence: UInt64) -> AsyncThrowingStream<WatchEvent, any Error> {
        var request = Qmsg_App_V1_WatchRequest()
        request.epoch = epoch
        request.afterSequence = sequence
        let metadata = auth()
        let options = options(timeout: nil)
        let app = app
        let message = request
        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    try await app.watch(message, metadata: metadata, options: options) { response in
                        for try await event in response.messages {
                            continuation.yield(try Self.decode(event))
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    static func decode(_ event: Qmsg_App_V1_Event) throws -> WatchEvent {
        switch event.body {
        case .snapshot(let data):
            var snapshot = try QmsgJSON.decode(Snapshot.self, from: data.json)
            snapshot.epoch = event.epoch
            snapshot.sequence = event.sequence
            return .snapshot(snapshot)
        case .update(let data):
            // An event this app doesn't understand still advances the cursor.
            let update = try? QmsgJSON.decode(ProviderEvent.self, from: data.json)
            return .update(update, epoch: event.epoch, sequence: event.sequence)
        case .heartbeat, nil:
            return .heartbeat(epoch: event.epoch, sequence: event.sequence)
        }
    }

    // MARK: Commands

    /// Runs a command. Actions need an `operation`, reused unchanged when
    /// retrying the same request.
    public func execute(
        _ command: Command,
        provider: ProviderID,
        operation: ActionOperation? = nil,
        timeout: Duration = .seconds(60)
    ) async throws -> ExecuteResult {
        var request = Qmsg_App_V1_ExecuteRequest()
        request.provider = .init(provider)
        request.commandJson = try QmsgJSON.encode(command)
        if let operation {
            request.operationID = operation.id
            request.createdAtMs = operation.createdAtMs
        }
        let ms = timeout.components.seconds * 1000 + timeout.components.attoseconds / 1_000_000_000_000_000
        request.timeoutMs = UInt64(min(max(ms, 100), 900_000))
        // Give the server time to answer its own timeout first.
        let outcome = try await app.execute(request, metadata: auth(), options: options(timeout: timeout + .seconds(15)))
        let result: Result<Reply, AppError>
        switch outcome.result {
        case .reply(let data): result = .success(try QmsgJSON.decode(Reply.self, from: data.json))
        case .errorJson(let json): result = .failure(try QmsgJSON.decode(AppError.self, from: json))
        case nil: throw RPCError(code: .internalError, message: "empty outcome")
        }
        return ExecuteResult(result: result, epoch: outcome.epoch, observedSequence: outcome.observedSequence)
    }

    // MARK: Media

    /// Uploads a file for one provider instance. Returns media with a Blob
    /// source to put in a send or function call.
    public func upload(provider: ProviderID, name: String, mime: String, data: Data) async throws -> Media {
        let first = data.prefix(Self.chunk)
        let token = auth()
        let response = try await app.upload(metadata: token, options: options(timeout: .seconds(130))) { writer in
            var head = Qmsg_App_V1_UploadChunk()
            head.provider = .init(provider)
            head.name = name
            head.mime = mime
            head.size = UInt64(data.count)
            head.data = Data(first)
            try await writer.write(head)
            var offset = first.count
            while offset < data.count {
                var chunk = Qmsg_App_V1_UploadChunk()
                chunk.data = data.subdata(in: offset..<min(offset + Self.chunk, data.count))
                try await writer.write(chunk)
                offset += Self.chunk
            }
        }
        return try QmsgJSON.decode(Media.self, from: response.json)
    }

    /// Reads a whole blob handle.
    public func readBlob(id: String, maxBytes: Int = 100 * 1024 * 1024) async throws -> Data {
        var data = Data()
        while true {
            var request = Qmsg_App_V1_ReadBlobRequest()
            request.id = id
            request.offset = UInt64(data.count)
            request.length = UInt32(Self.chunk)
            let chunk = try await app.readBlob(request, metadata: auth(), options: options(timeout: .seconds(70))).data
            data.append(chunk)
            if chunk.count < Self.chunk { return data }
            if data.count > maxBytes { throw RPCError(code: .resourceExhausted, message: "file too large") }
        }
    }

    public func releaseBlob(id: String) async throws {
        var request = Qmsg_App_V1_ReleaseBlobRequest()
        request.id = id
        _ = try await app.releaseBlob(request, metadata: auth(), options: options())
    }
}

extension Credential {
    init(_ wire: Qmsg_App_V1_Credential) {
        self.init(deviceID: wire.deviceID, token: wire.token, expiresAtMs: wire.expiresAtMs)
    }
}

extension Qmsg_App_V1_Provider {
    init(_ id: ProviderID) {
        self.init()
        name = id.name
        instance = id.instance
    }
}

extension Error {
    /// The gRPC status code, if this is a gRPC error.
    public var rpcCode: RPCError.Code? { (self as? RPCError)?.code }

    /// Whether the token was rejected: expired, rotated or revoked.
    public var isUnauthenticated: Bool { rpcCode == .unauthenticated }
}
