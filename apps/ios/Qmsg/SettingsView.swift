import SwiftUI
import QmsgKit

/// Accounts first, as in Mail's settings, then the server and this device.
struct SettingsView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var model
    @Environment(AppRouter.self) private var router
    @State private var removing: String?
    @State private var busy: String?
    @State private var error: String?
    @State private var confirmUnpair = false
    @State private var unpairing = false

    var body: some View {
        @Bindable var router = router
        NavigationStack(path: $router.settingsPath) {
            List {
                accounts
                Section {
                    HStack(spacing: 12) {
                        Image(systemName: "server.rack").font(.title3).foregroundStyle(.tint).frame(width: 30)
                        VStack(alignment: .leading, spacing: 3) {
                            if let endpoint = store.account?.endpoint {
                                Text(endpoint.host).textSelection(.enabled)
                            }
                            serverStatus.font(.caption)
                        }
                        Spacer(minLength: 0)
                        if store.link != .live && store.link != .connecting {
                            Button("Reconnect") { store.resume() }.buttonStyle(.bordered).controlSize(.small)
                        }
                    }
                    NavigationLink { DevicesView() } label: {
                        Label("Paired Devices", systemImage: "iphone")
                    }
                } header: { Text("Server") }
                Section {
                    Button("Unpair This Device", role: .destructive) { confirmUnpair = true }.disabled(unpairing)
                } footer: { Text("You'll need a new pairing code from your server to connect again.") }
            }
            .navigationTitle("Settings")
            .refreshable { await model.load(store) }
            .navigationDestination(for: String.self) { AccountDetailView(name: $0) }
            .sheet(isPresented: $router.addingAccount) {
                NavigationStack {
                    AddAccountView { name in
                        router.addingAccount = false
                        router.settingsPath = [name]
                    }
                }
            }
            .confirmationDialog("Remove \(removing.map { model.label(for: $0) } ?? "account")?", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }), titleVisibility: .visible) {
                if let name = removing {
                    Button("Remove Account", role: .destructive) { Task { await remove(name) } }
                }
            } message: {
                Text("Its chats disappear from qmsg and its saved sign-in is deleted from your server. Nothing is deleted on the network itself.")
            }
            .confirmationDialog("Unpair this device?", isPresented: $confirmUnpair, titleVisibility: .visible) {
                Button("Unpair", role: .destructive) {
                    unpairing = true
                    Task { await store.unpair(); unpairing = false }
                }
            }
        }
    }

    @ViewBuilder
    private var accounts: some View {
        let items = model.items(store)
        Section {
            if let error = error ?? model.error {
                ErrorNote(message: error) { self.error = nil; model.error = nil }
            }
            if items.isEmpty && model.loading { ProgressView().frame(maxWidth: .infinity) }
            ForEach(items) { item in
                NavigationLink(value: item.name) { AccountRow(item: item, busy: busy == item.name) }
                    .swipeActions(edge: .trailing) {
                        if item.managed != nil {
                            Button("Remove", systemImage: "trash", role: .destructive) { removing = item.name }
                        }
                    }
                    .swipeActions(edge: .leading) {
                        if let managed = item.managed {
                            Button(managed.enabled ? "Turn Off" : "Turn On", systemImage: managed.enabled ? "pause.circle" : "play.circle") {
                                Task { await setEnabled(!managed.enabled, managed) }
                            }.tint(managed.enabled ? .gray : .green)
                        }
                    }
            }
            Button { router.addingAccount = true } label: {
                Label("Add Account", systemImage: "plus")
            }
            .disabled(model.types.isEmpty)
        } header: {
            Text("Accounts")
        } footer: {
            if model.catalog != nil && model.types.isEmpty {
                Text("Your server has no chat networks installed.")
            } else if items.isEmpty && !model.loading {
                Text("Add Discord, mail or another network to see its chats in qmsg.")
            }
        }
    }

    private func setEnabled(_ enabled: Bool, _ managed: ManagedProvider) async {
        busy = managed.name
        defer { busy = nil }
        do {
            try await store.configureProvider(name: managed.name, type: managed.type, settings: managed.settings, enabled: enabled, label: managed.label)
            await model.load(store)
        } catch { self.error = QmsgStore.describe(error) }
    }

    private func remove(_ name: String) async {
        busy = name
        defer { busy = nil }
        do {
            try await store.removeProvider(name: name)
            await model.load(store)
        } catch { self.error = QmsgStore.describe(error) }
    }

    @ViewBuilder
    private var serverStatus: some View {
        switch store.link {
        case .live: Label("Connected", systemImage: "circle.fill").foregroundStyle(.green)
        case .connecting: Label("Connecting…", systemImage: "circle.dotted").foregroundStyle(.secondary)
        case .idle: Label("Offline", systemImage: "circle").foregroundStyle(.secondary)
        case .retrying(let error): Label(error, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange).lineLimit(2)
        }
    }
}

/// The devices paired with the server; any of them can be revoked.
private struct DevicesView: View {
    @Environment(QmsgStore.self) private var store
    @State private var devices: [PairedDevice] = []
    @State private var error: String?
    @State private var loading = false
    @State private var busy = false
    @State private var revoke: PairedDevice?

    var body: some View {
        List {
            Section {
                if loading && devices.isEmpty { ProgressView().frame(maxWidth: .infinity) }
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
            } footer: {
                Text("Swipe a device to revoke its access.")
            }
        }
        .navigationTitle("Paired Devices")
        .navigationBarTitleDisplayMode(.inline)
        .refreshable { await load() }
        .task { await load() }
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
}
