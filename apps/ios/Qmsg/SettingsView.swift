import SwiftUI
import QmsgKit

struct SettingsView: View {
    @Environment(QmsgStore.self) private var store
    @State private var devices: [PairedDevice] = []
    @State private var error: String?
    @State private var loading = false
    @State private var busy = false
    @State private var revoke: PairedDevice?
    @State private var confirmUnpair = false

    var body: some View {
        Form {
            Section {
                HStack(spacing: 12) {
                    Image(systemName: "server.rack").font(.title3).foregroundStyle(.tint).frame(width: 30)
                    VStack(alignment: .leading, spacing: 3) {
                        if let endpoint = store.account?.endpoint {
                            Text(endpoint.host).textSelection(.enabled)
                        }
                        serverStatus
                    }
                }
                if store.link != .live && store.link != .connecting {
                    Button("Reconnect") { store.resume() }
                }
            } header: { Text("Server") }
            Section {
                if loading && devices.isEmpty { ProgressView() }
                ForEach(devices) { device in
                    HStack(spacing: 12) {
                        Image(systemName: "iphone").foregroundStyle(.secondary).frame(width: 30)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(device.name)
                            if device.id == store.account?.deviceID { Text("This device").font(.caption).foregroundStyle(.secondary) }
                        }
                    }
                    .swipeActions {
                        Button("Revoke", role: .destructive) { revoke = device }.disabled(busy)
                    }
                }
                if let error { ErrorNote(message: error) { self.error = nil } }
            } header: {
                Text("Paired devices")
            } footer: {
                Text("Swipe a device to revoke its access.")
            }
            Section {
                Button("Unpair this device", role: .destructive) { confirmUnpair = true }.disabled(busy)
            } footer: { Text("You will need a new pairing code to connect again.") }
        }
        .navigationTitle("Settings")
        .refreshable { await load() }
        .task { await load() }
        .confirmationDialog("Unpair this device?", isPresented: $confirmUnpair, titleVisibility: .visible) {
            Button("Unpair", role: .destructive) {
                busy = true
                Task { await store.unpair(); busy = false }
            }
        }
        .confirmationDialog("Revoke \(revoke?.name ?? "device")?", isPresented: Binding(get: { revoke != nil }, set: { if !$0 { revoke = nil } }), titleVisibility: .visible) {
            if let device = revoke {
                Button("Revoke", role: .destructive) {
                    busy = true
                    Task {
                        defer { busy = false }
                        do {
                            try await store.revoke(device)
                            if store.isPaired { await load() }
                        } catch { self.error = QmsgStore.describe(error) }
                    }
                }
            }
        }
    }

    private func load() async {
        loading = true
        error = nil
        defer { loading = false }
        do { devices = try await store.devices() }
        catch { self.error = QmsgStore.describe(error) }
    }

    @ViewBuilder
    private var serverStatus: some View {
        switch store.link {
        case .live: Label("Connected", systemImage: "circle.fill").foregroundStyle(.green)
        case .connecting: Label("Connecting…", systemImage: "circle.dotted").foregroundStyle(.secondary)
        case .idle: Label("Offline", systemImage: "circle").foregroundStyle(.secondary)
        case .retrying(let error): Label(error, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange)
        }
    }
}
