import SwiftUI
import QmsgKit

/// The server's account setup, merged with what the live connection reports.
@MainActor @Observable
final class AccountsModel {
    private(set) var catalog: ProviderCatalog?
    private(set) var loading = false
    var error: String?

    struct Item: Identifiable {
        var name: String
        var type: ProviderType?
        var managed: ManagedProvider?
        var live: ProviderState?
        var id: String { name }
        /// The editable display name, falling back to the stable account name.
        var label: String {
            let label = managed?.label.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return label.isEmpty ? name : label
        }
        var enabled: Bool { managed?.enabled ?? true }
        var health: AccountHealth { AccountHealth(status: live?.status, enabled: enabled) }
        var style: NetworkStyle { NetworkStyle(managed?.type ?? name) }
        var typeLabel: String { type?.label ?? managed?.type ?? "Account" }
    }

    var types: [ProviderType] { catalog?.types ?? [] }

    func items(_ store: QmsgStore) -> [Item] {
        var items: [Item] = (catalog?.accounts ?? []).map { account in
            Item(name: account.name, type: types.first { $0.id == account.type }, managed: account, live: store.providers[account.name])
        }
        // Accounts the server runs but the catalog didn't list (or hasn't loaded yet).
        for provider in store.sortedProviders where !items.contains(where: { $0.name == provider.name }) {
            items.append(Item(name: provider.name, live: provider))
        }
        return items.sorted { ($0.label.localizedLowercase, $0.name) < ($1.label.localizedLowercase, $1.name) }
    }

    func item(_ name: String, _ store: QmsgStore) -> Item? {
        items(store).first { $0.name == name }
    }

    func style(for provider: String) -> NetworkStyle {
        NetworkStyle(catalog?.accounts.first { $0.name == provider }?.type ?? provider)
    }

    /// The network type id, like "discord", when the server has said.
    func type(for provider: String) -> String? {
        catalog?.accounts.first { $0.name == provider }?.type
    }

    func label(for provider: String) -> String {
        let label = catalog?.accounts.first { $0.name == provider }?.label ?? ""
        return label.isEmpty ? provider : label
    }

    func load(_ store: QmsgStore) async {
        loading = true
        defer { loading = false }
        do {
            catalog = try await store.providerCatalog()
            error = nil
        } catch {
            self.error = QmsgStore.describe(error)
        }
    }
}

struct AccountsView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var model
    @Environment(AppRouter.self) private var router
    @State private var adding = false
    @State private var removing: String?
    @State private var busy: String?
    @State private var error: String?

    var body: some View {
        @Bindable var router = router
        NavigationStack(path: $router.accountPath) {
            List {
                if let error = error ?? model.error {
                    Section { ErrorNote(message: error) { self.error = nil; model.error = nil } }
                }
                let items = model.items(store)
                Section {
                    ForEach(items) { item in
                        NavigationLink(value: item.name) { AccountRow(item: item, busy: busy == item.name) }
                            .swipeActions(edge: .trailing) {
                                if item.managed != nil {
                                    Button("Remove", systemImage: "trash", role: .destructive) { removing = item.name }
                                }
                            }
                            .swipeActions(edge: .leading) {
                                if let managed = item.managed {
                                    Button(managed.enabled ? "Turn off" : "Turn on", systemImage: managed.enabled ? "pause.circle" : "play.circle") {
                                        Task { await setEnabled(!managed.enabled, managed) }
                                    }.tint(managed.enabled ? .gray : .green)
                                }
                            }
                    }
                } footer: {
                    if !items.isEmpty { Text("Swipe an account to turn it off or remove it.") }
                }
                Section {
                    Button { adding = true } label: { Label("Add account", systemImage: "plus.circle.fill") }
                        .disabled(model.types.isEmpty)
                } footer: {
                    if model.catalog != nil && model.types.isEmpty {
                        Text("Your server has no chat networks installed.")
                    }
                }
            }
            .overlay {
                if model.items(store).isEmpty {
                    if model.loading {
                        ProgressView()
                    } else {
                        ContentUnavailableView {
                            Label("No accounts yet", systemImage: "person.crop.circle.badge.plus")
                        } description: {
                            Text("Choose a chat network to see its chats here.")
                        } actions: {
                            Button("Add account") { adding = true }.buttonStyle(.borderedProminent).disabled(model.types.isEmpty)
                        }
                    }
                }
            }
            .navigationTitle("Accounts")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button("Add account", systemImage: "plus") { adding = true }.disabled(model.types.isEmpty)
                }
            }
            .refreshable { await model.load(store) }
            .navigationDestination(for: String.self) { AccountDetailView(name: $0) }
            .sheet(isPresented: $adding) {
                NavigationStack {
                    AddAccountView { name in
                        adding = false
                        router.accountPath = [name]
                    }
                }
            }
            .confirmationDialog("Remove \(removing.map { model.label(for: $0) } ?? "account")?", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }), titleVisibility: .visible) {
                if let name = removing {
                    Button("Remove account", role: .destructive) { Task { await remove(name) } }
                }
            } message: {
                Text("Its chats disappear from qmsg and its saved sign-in is deleted from your server. Nothing is deleted on the network itself.")
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
}

private struct AccountRow: View {
    let item: AccountsModel.Item
    let busy: Bool

    var body: some View {
        HStack(spacing: 12) {
            NetworkBadge(style: item.style, size: 36)
                .opacity(item.enabled ? 1 : 0.45)
            VStack(alignment: .leading, spacing: 3) {
                Text(item.label).font(.body.weight(.medium))
                HStack(spacing: 6) {
                    Text(item.typeLabel)
                    Text("·")
                    StatusDot(health: item.health)
                }.font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            if busy { ProgressView() }
            else if item.health.needsUser {
                Image(systemName: "exclamationmark.circle.fill").foregroundStyle(item.health.color)
                    .accessibilityLabel(item.health.title)
            }
        }
        .padding(.vertical, 2)
    }
}

// MARK: - Account detail

struct AccountDetailView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let name: String
    @State private var functions: [Function] = []
    @State private var busy = false
    @State private var error: String?
    @State private var confirmSignOut = false
    @State private var confirmRemove = false
    @State private var editing = false

    var body: some View {
        let item = model.item(name, store)
        List {
            if let item {
                header(item)
                if let error { Section { ErrorNote(message: error) { self.error = nil } } }
                if let step = item.live?.status.loginStep { signIn(step) }
                if let managed = item.managed {
                    Section {
                        Toggle("Enabled", isOn: Binding(get: { managed.enabled }, set: { value in Task { await setEnabled(value, managed) } }))
                            .disabled(busy)
                    } footer: {
                        Text("A turned-off account stays saved but doesn't sync, notify or send.")
                    }
                    if item.type != nil {
                        Section {
                            Button { editing = true } label: {
                                Label("Edit account", systemImage: "pencil")
                            }.disabled(busy)
                        }
                    }
                }
                Section {
                    if item.live?.status.isReady == true, functions.contains(where: { $0.action == .logout }) {
                        Button("Sign out", systemImage: "rectangle.portrait.and.arrow.right") { confirmSignOut = true }.disabled(busy)
                    }
                    if item.managed != nil {
                        Button("Remove account", systemImage: "trash", role: .destructive) { confirmRemove = true }.disabled(busy)
                    }
                }
            } else {
                ContentUnavailableView("Account removed", systemImage: "person.crop.circle.badge.xmark", description: Text("This account is no longer on your server."))
            }
        }
        .navigationTitle(item?.label ?? name)
        .navigationBarTitleDisplayMode(.inline)
        .task(id: store.providers[name]?.status) { await loadFunctions() }
        .sheet(isPresented: $editing) {
            if let item, let managed = item.managed, let type = item.type {
                NavigationStack { AccountFormView(type: type, existing: managed) { _ in editing = false } }
            }
        }
        .confirmationDialog("Sign out of \(item?.label ?? name)?", isPresented: $confirmSignOut, titleVisibility: .visible) {
            Button("Sign out", role: .destructive) {
                if let function = functions.first(where: { $0.action == .logout }) { Task { await run(function) } }
            }
        } message: { Text("You'll need to sign in again to see these chats.") }
        .confirmationDialog("Remove \(item?.label ?? name)?", isPresented: $confirmRemove, titleVisibility: .visible) {
            Button("Remove account", role: .destructive) { Task { await remove() } }
        } message: {
            Text("Its chats disappear from qmsg and its saved sign-in is deleted from your server. Nothing is deleted on the network itself.")
        }
    }

    private func header(_ item: AccountsModel.Item) -> some View {
        Section {
            HStack(spacing: 14) {
                NetworkBadge(style: item.style, size: 52)
                VStack(alignment: .leading, spacing: 4) {
                    Text(item.label).font(.title3.bold())
                    Text(item.typeLabel).foregroundStyle(.secondary)
                }
            }.padding(.vertical, 4)
            HStack {
                Text("Status")
                Spacer()
                StatusDot(health: item.health).font(.body)
            }
            if let detail = item.health.detail, item.live?.status.loginStep == nil {
                Text(detail).font(.footnote).foregroundStyle(item.health.color).textSelection(.enabled)
            }
        }
    }

    @ViewBuilder
    private func signIn(_ step: LoginStep) -> some View {
        Section {
            if let google = functions.first(where: { $0.label == "Sign in with Google" && $0.action == .login && $0.inputs.isEmpty }) {
                Text("Choose your Google account to connect your mail.")
                NavigationLink("Sign in with Google") {
                    GoogleSignInView(provider: name, function: google)
                }
            } else {
                LoginStepView(step: step)
                if let function = functions.first(where: { $0.id == step.function || $0.action == .login }) {
                    if function.inputs.isEmpty {
                        Button { Task { await run(function) } } label: {
                            HStack { Text("Sign in").bold(); Spacer(); if busy { ProgressView() } }
                        }.disabled(busy)
                    } else {
                        NavigationLink("Continue sign in") { SignInForm(provider: name, context: .provider, function: function) }
                    }
                }
            }
        } header: {
            Text("Sign in")
        }
    }

    private func loadFunctions() async {
        guard store.providers[name] != nil else { functions = []; return }
        do { functions = try await store.functions(.provider, provider: name) }
        catch { self.error = QmsgStore.describe(error) }
    }

    private func run(_ function: Function) async {
        busy = true
        error = nil
        defer { busy = false }
        do { _ = try await store.call(function, context: .provider, arguments: [:], provider: name, timeout: .seconds(600)).get() }
        catch { self.error = QmsgStore.describe(error) }
    }

    private func setEnabled(_ enabled: Bool, _ managed: ManagedProvider) async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            try await store.configureProvider(name: managed.name, type: managed.type, settings: managed.settings, enabled: enabled, label: managed.label)
            await model.load(store)
        } catch { self.error = QmsgStore.describe(error) }
    }

    private func remove() async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            try await store.removeProvider(name: name)
            await model.load(store)
            dismiss()
        } catch { self.error = QmsgStore.describe(error) }
    }
}

// MARK: - Adding and editing

struct AddAccountView: View {
    @Environment(AccountsModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let created: (String) -> Void

    var body: some View {
        List {
            Section {
                ForEach(model.types) { type in
                    NavigationLink {
                        AccountFormView(type: type, existing: nil, done: created)
                    } label: {
                        HStack(spacing: 12) {
                            NetworkBadge(style: NetworkStyle(type.id), size: 34)
                            Text(type.label)
                        }
                    }
                }
            } footer: {
                Text("Only networks installed on your server are listed. You can add more than one account of the same network.")
            }
        }
        .navigationTitle("Add account")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
    }
}

/// Creates an account of a type, or edits an existing one. The display
/// label can change any time; the stable account name is derived once.
struct AccountFormView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let type: ProviderType
    let existing: ManagedProvider?
    let done: (String) -> Void
    @State private var label = ""
    @State private var values: [String: String] = [:]
    @State private var busy = false
    @State private var error: String?

    private var trimmedLabel: String { label.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var labelProblem: String? {
        if trimmedLabel.isEmpty { return "Enter a name." }
        let clash = model.items(store).contains {
            $0.name != existing?.name && $0.label.caseInsensitiveCompare(trimmedLabel) == .orderedSame
        }
        return clash ? "You already have an account called “\(trimmedLabel)”." : nil
    }

    private var missing: [ProviderField] {
        type.fields.filter { field in
            guard field.required else { return false }
            // A saved secret isn't sent back, so blank keeps it.
            if existing != nil && field.secret { return false }
            return (values[field.id] ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    var body: some View {
        Form {
            Section {
                TextField("Name", text: $label)
                    .autocorrectionDisabled()
            } header: {
                Text("Name")
            } footer: {
                if let labelProblem, !trimmedLabel.isEmpty { Text(labelProblem).foregroundStyle(.red) }
                else { Text("Shown in your chat list, like “Work” or “Personal”.") }
            }
            if !type.fields.isEmpty {
                Section {
                    ForEach(type.fields) { field in
                        fieldView(field)
                    }
                } header: {
                    Text("Connection")
                } footer: {
                    if existing != nil && type.fields.contains(where: \.secret) {
                        Text("Leave a password or token blank to keep the saved one.")
                    }
                }
            }
            if existing == nil {
                Section {
                    Text("After adding, you'll be asked to sign in if \(type.label) needs it.")
                        .font(.footnote).foregroundStyle(.secondary)
                }
            }
            if let error { Section { ErrorNote(message: error) { self.error = nil } } }
        }
        .disabled(busy)
        .navigationTitle(existing == nil ? "New \(type.label) account" : "Edit account")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if existing != nil {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button { Task { await save() } } label: {
                    if busy { ProgressView() } else { Text(existing == nil ? "Add" : "Save").bold() }
                }
                .disabled(busy || labelProblem != nil || !missing.isEmpty)
            }
        }
        .onAppear {
            guard label.isEmpty else { return }
            if let existing {
                label = existing.label.isEmpty ? existing.name : existing.label
                values = existing.settings
            } else {
                label = suggestedLabel()
                values = type.defaults
            }
        }
    }

    @ViewBuilder
    private func fieldView(_ field: ProviderField) -> some View {
        let binding = Binding(get: { values[field.id] ?? "" }, set: { values[field.id] = $0 })
        let prompt = field.secret && existing != nil ? "Saved" : (field.required ? "Required" : "Optional")
        LabeledContent(field.label) {
            Group {
                if field.secret {
                    SecureField(field.label, text: binding, prompt: Text(prompt))
                } else {
                    TextField(field.label, text: binding, prompt: Text(prompt))
                }
            }
            .multilineTextAlignment(.trailing)
            .textInputAutocapitalization(.never).autocorrectionDisabled()
        }
    }

    private func suggestedLabel() -> String {
        let base = type.label
        let taken = Set(model.items(store).map { $0.label.lowercased() })
        if !taken.contains(base.lowercased()) { return base }
        var n = 2
        while taken.contains("\(base) \(n)".lowercased()) { n += 1 }
        return "\(base) \(n)"
    }

    /// A stable, unique account name like "discord-work", made once at creation.
    private func newName() -> String {
        let characters: [Character] = trimmedLabel.lowercased().unicodeScalars
            .map { $0.isASCII && CharacterSet.alphanumerics.contains($0) ? Character($0) : "-" }
        let slug: String = String(characters).split(separator: "-").joined(separator: "-")
        let typeSlug = type.id.lowercased()
        var base = slug.isEmpty || slug == typeSlug ? typeSlug : (slug.hasPrefix(typeSlug) ? slug : "\(typeSlug)-\(slug)")
        base = String(base.prefix(40))
        let taken = Set(model.items(store).map { $0.name.lowercased() })
        if !taken.contains(base) { return base }
        var n = 2
        while taken.contains("\(base)-\(n)") { n += 1 }
        return "\(base)-\(n)"
    }

    private func save() async {
        busy = true
        error = nil
        defer { busy = false }
        var settings: [String: String] = [:]
        for field in type.fields {
            let value = (values[field.id] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            if !value.isEmpty { settings[field.id] = value }
        }
        let accountName = existing?.name ?? newName()
        do {
            try await store.configureProvider(name: accountName, type: type.id, settings: settings, enabled: existing?.enabled ?? true, label: trimmedLabel)
            await model.load(store)
            done(accountName)
        } catch { self.error = QmsgStore.describe(error) }
    }
}
