import Foundation
import GRPCCore
import NIOSSL
import Testing
@testable import QmsgKit

/// Runs against a real orchestrator with the echo provider. Set
/// QMSG_PAIRING to the output of `qmsg-orchestrator pair <config> --json`;
/// see apps/ios/README.md.
@Suite(.enabled(if: ProcessInfo.processInfo.environment["QMSG_PAIRING"] != nil), .serialized)
@MainActor struct LiveTests {
    let pairing = ProcessInfo.processInfo.environment["QMSG_PAIRING"] ?? ""

    @Test func pairWatchSendUploadAndRotate() async throws {
        let store = QmsgStore(accounts: MemoryAccountStore())
        try await store.pair(code: pairing, deviceName: "QmsgKit live test")
        #expect(store.isPaired)

        // The snapshot arrives through Watch, then the echo channel.
        try await until("echo channel") {
            store.providers["echo"]?.status == .ready
                && store.providers["echo"]?.organizations.values.first?.scope.channels.isEmpty == false
        }
        let echo = store.providers["echo"]!
        let organization = echo.organizations.values.first!
        let channel = organization.scope.channels.values.first!
        let key = ChannelKey(provider: "echo", channel: ChannelRef(organization: organization.id, channel: channel.id))
        store.visible = key
        await store.loadLatest(key)
        let conversation = store.conversation(key)
        #expect(conversation.loaded)

        // A text send settles its pending entry and the server echoes it.
        let text = "hello \(UUID().uuidString.prefix(8))"
        store.send(text: text, to: key)
        #expect(conversation.pending.count == 1)
        try await until("echo of the text") {
            conversation.pending.isEmpty && conversation.messages.contains { $0.content.first?.text == "echo: \(text)" }
        }
        let sent = try #require(conversation.messages.first { $0.content.first?.text == text })
        #expect(sent.author == organization.scope.me)

        // A file goes up through Upload, then comes back as a blob to read.
        let bytes = Data((0..<300_000).map { UInt8($0 % 251) })
        store.send(text: "", attachments: [.init(name: "data.bin", mime: "application/octet-stream", data: bytes)], to: key)
        try await until("file message") {
            conversation.pending.isEmpty && conversation.messages.contains { $0.content.contains { $0.media?.name == "data.bin" } }
        }
        let fileMessage = conversation.messages.last { $0.content.contains { $0.media?.name == "data.bin" } }!
        let media = try #require(fileMessage.content.compactMap(\.media).first)
        guard case .blob = media.source else { Issue.record("expected a blob, got \(media.source)"); return }
        #expect(try await store.data(for: media) == bytes)

        // History pages are oldest first and include what was sent.
        await store.loadLatest(key)
        #expect(conversation.messages.contains { $0.id == sent.id })
        #expect(conversation.messages.map(\.sentAt) == conversation.messages.map(\.sentAt).sorted())

        // Function discovery and a journaled action replayed with the same operation.
        let functions = try await store.functions(.message(channel: key.channel, id: sent.id), provider: "echo")
        if let react = functions.first(where: { $0.action == .addReaction }) {
            let operation = ActionOperation()
            let context = FunctionContext.message(channel: key.channel, id: sent.id)
            let first = try await store.call(react, context: context, arguments: ["reaction": .text("👍")], provider: "echo", operation: operation)
            let replay = try await store.call(react, context: context, arguments: ["reaction": .text("👍")], provider: "echo", operation: operation)
            #expect((try? first.get()) == (try? replay.get()))
            try await until("reaction") { conversation.message(sent.id)?.reactions.first?.key == "👍" }
            #expect(conversation.message(sent.id)?.reactions.first?.count == 1)
        }

        // Rotation with a persisted secret yields the proposed token.
        let account = try #require(store.account)
        // A send before resume fails to upload. Its original files must
        // survive, so retry after connecting sends the same bytes.
        let retryStore = QmsgStore(accounts: MemoryAccountStore(account))
        defer { retryStore.suspend() }
        let files: [QmsgStore.Attachment] = [
            .init(name: "retry-\(UUID().uuidString).bin", mime: "application/octet-stream", data: Data([1, 2, 3])),
        ]
        retryStore.send(text: "retried attachment", attachments: files, to: key)
        let retryConversation = retryStore.conversation(key)
        try await until("failed upload") {
            if case .failed = retryConversation.pending.first?.state { return true }
            return false
        }
        let failed = try #require(retryConversation.pending.first)
        #expect(failed.attachments == files)
        retryStore.resume()
        try await until("retry store connects") { retryStore.providers["echo"]?.status == .ready }
        await retryStore.loadLatest(key)
        retryStore.retry(failed, in: key)
        let retryNonce = try #require(retryConversation.pending.last?.id)
        try await until("retried files") {
            retryConversation.pending.isEmpty && retryConversation.messages.contains { message in
                message.nonce == retryNonce
            }
        }
        let retried = try #require(retryConversation.messages.last { message in
            message.nonce == retryNonce
        })
        for file in files {
            let media = try #require(retried.content.compactMap(\.media).first { $0.name == file.name })
            #expect(try await retryStore.data(for: media) == file.data)
        }

        let client = try QmsgClient(endpoint: account.endpoint, token: account.token)
        let secret = QmsgStore.randomSecret()
        let rotated = try await client.rotate(secret: secret)
        #expect(rotated.token == "\(account.deviceID).\(secret)")
        _ = try await client.snapshot(token: rotated.token)
        await #expect(throws: RPCError.self) { _ = try await client.snapshot(token: account.token) }

        // The store recovers from the rejected token by... pairing again; the old one is gone.
        client.setToken(rotated.token)
        let devices = try await client.devices()
        #expect(devices.contains { $0.id == account.deviceID })
        try await client.revoke(deviceID: account.deviceID)
        await #expect(throws: RPCError.self) { _ = try await client.devices() }
        try await until("store notices revocation") { store.needsPairing || store.link != .live }
    }

    @Test func rejectsAWrongCertificatePin() async throws {
        let payload = try PairingError.payload(from: pairing)
        var endpoint = try ServerEndpoint(payload: payload)
        guard endpoint.secure else { return }
        endpoint.certificateSHA256 = String(repeating: "0", count: 64)
        let client = try QmsgClient(endpoint: endpoint)
        await #expect(throws: (any Error).self) { _ = try await client.snapshot() }

        // The pin alone isn't enough: the certificate must also name the host.
        let pem = try #require(payload.certificatePEM)
        let der = try NIOSSLCertificate(bytes: Array(pem.utf8), format: .pem).toDERBytes()
        let pin = try ServerEndpoint.fingerprint(pem: pem)
        #expect(ServerEndpoint.verify(leaf: der, pinned: pin, host: endpoint.host))
        #expect(!ServerEndpoint.verify(leaf: der, pinned: pin, host: "evil.example"))
        #expect(!ServerEndpoint.verify(leaf: der, pinned: pin, host: "10.9.8.7"))
    }

    private func until(_ what: String, timeout: Duration = .seconds(20), _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + timeout
        while !condition() {
            guard ContinuousClock.now < deadline else {
                Issue.record("timed out waiting for \(what)")
                throw CancellationError()
            }
            try await Task.sleep(for: .milliseconds(50))
        }
    }
}
