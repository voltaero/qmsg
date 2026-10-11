import SwiftUI
import QmsgKit

struct SettingsView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var devices: [PairedDevice] = []
    @State private var error: String?
    @State private var loading = false
    @State private var busy = false
    @State private var revoke: PairedDevice?
    @State private var confirmUnpair = false

    var body: some View {
        Form {
            Section("Server") {
                if let endpoint = store.account?.endpoint {
                    Text("\(endpoint.host):\(endpoint.port)").textSelection(.enabled)
                }
                ConnectionLabel(link: store.link)
            }
            Section("Paired devices") {
                if loading { ProgressView() }
                ForEach(devices) { device in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(device.name)
                            if device.id == store.account?.deviceID { Text("This device").font(.caption).foregroundStyle(.secondary) }
                        }
                        Spacer()
                        Button("Revoke", role: .destructive) { revoke = device }.disabled(busy)
                    }
                }
                Button("Refresh devices") { Task { await load() } }.disabled(loading || busy)
                if let error { Text(error).foregroundStyle(.red) }
            }
            Section {
                Button("Unpair this device", role: .destructive) { confirmUnpair = true }.disabled(busy)
            } footer: { Text("You will need a new pairing code to connect again.") }
        }
        .navigationTitle("Settings")
        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        .task { await load() }
        .confirmationDialog("Unpair this device?", isPresented: $confirmUnpair, titleVisibility: .visible) {
            Button("Unpair", role: .destructive) {
                busy = true
                Task { await store.unpair(); busy = false; dismiss() }
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
                            if store.isPaired { await load() } else { dismiss() }
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
}
