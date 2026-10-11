import SwiftUI
import QmsgKit

struct NewChatView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var accounts
    @Environment(\.dismiss) private var dismiss
    let opened: (ChannelKey) -> Void
    @State private var provider = ""
    @State private var query = ""
    @State private var organization = ""
    private var context: FunctionContext { organization.isEmpty ? .provider : .organization(organization) }
    /// Mail accounts write from their folders instead.
    private var chatAccounts: [ProviderState] {
        store.sortedProviders.filter { provider in
            !provider.organizations.values.contains { $0.scope.channels.values.contains(where: \.isMailbox) }
        }
    }
    @State private var function: Function?
    @State private var contacts: [CompletionItem] = []
    @State private var selected: [CompletionItem] = []
    @State private var loading = false
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        List {
            Section {
                Picker("Account", selection: $provider) {
                    ForEach(chatAccounts) { account in Text(accounts.label(for: account.name)).tag(account.name) }
                }.disabled(busy)
                if let account = store.providers[provider], !account.sortedOrganizations.isEmpty {
                    Picker("Contacts in", selection: $organization) {
                        Text("Direct messages").tag("")
                        ForEach(account.sortedOrganizations) { group in Text(group.name).tag(group.id) }
                    }.disabled(busy)
                }
            }
            if !selected.isEmpty {
                Section("To") {
                    ForEach(selected, id: \.value) { contact in
                        Button { selected.removeAll { $0.value == contact.value } } label: {
                            HStack { Text(contact.label); Spacer(); Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary) }
                        }.disabled(busy)
                    }
                }
            }
            Section("Contacts") {
                if loading { ProgressView() }
                ForEach(contacts, id: \.value) { contact in
                    Button {
                        if !selected.contains(where: { $0.value == contact.value }) { selected.append(contact) }
                        query = ""
                    } label: {
                        HStack {
                            InitialsAvatar(name: contact.label, size: 36)
                            VStack(alignment: .leading) {
                                Text(contact.label)
                                if let detail = contact.description { Text(detail).font(.caption).foregroundStyle(.secondary) }
                            }
                            Spacer()
                            if selected.contains(where: { $0.value == contact.value }) { Image(systemName: "checkmark") }
                        }
                    }.disabled(busy)
                }
                if !loading && contacts.isEmpty {
                    Text(function == nil ? "This account cannot start a new chat here." : "No contacts found.").foregroundStyle(.secondary)
                }
            }
            if let error { Section { ErrorNote(message: error) { self.error = nil } } }
        }
        .navigationTitle("New chat")
        .searchable(text: $query, prompt: "Search contacts")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            ToolbarItem(placement: .confirmationAction) {
                Button { Task { await start() } } label: { if busy { ProgressView() } else { Text("Chat") } }
                    .disabled(selected.isEmpty || busy || loading)
            }
        }
        .onAppear { if provider.isEmpty { provider = chatAccounts.first?.name ?? "" } }
        .onChange(of: provider) { _, _ in
            organization = store.providers[provider]?.standalone.users.isEmpty == true ? (store.providers[provider]?.sortedOrganizations.first?.id ?? "") : ""
            selected = []; query = ""; function = nil; contacts = []
        }
        .onChange(of: organization) { _, _ in selected = []; function = nil; contacts = [] }
        .task(id: provider + "\n" + organization + "\n" + query) { await search() }
    }

    private func search() async {
        guard !provider.isEmpty else { return }
        loading = true
        error = nil
        defer { loading = false }
        do {
            try await Task.sleep(for: .milliseconds(200))
            if function == nil { function = try await store.functions(context, provider: provider).first { $0.action == .openChannel } }
            guard let function, let input = function.inputs.first(where: { $0.id == StandardInput.members }) else { return }
            let items = try await store.complete(function, input: input, context: context, arguments: [:], query: query, provider: provider)
            try Task.checkCancellation()
            contacts = items.filter { $0.value.textValue != store.providers[provider]?.scope(organization.isEmpty ? nil : organization)?.me }
        } catch is CancellationError { }
        catch { self.error = QmsgStore.describe(error) }
    }

    private func start() async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            let members = selected.compactMap { $0.value.textValue }
            guard let function else { return }
            let arguments: Arguments = [StandardInput.members: .list(members.map { .text($0) })]
            if let input = function.inputs.first(where: { $0.id == StandardInput.members }), let helper = input.verification {
                for member in members {
                    let request = VerificationRequest(call: .init(function: helper, context: context, arguments: arguments), input: .init(function: function.id, input: input.id), value: .text(member))
                    let response = try await store.execute(.verify(request), provider: provider)
                    if case .verified(.invalid(let issues)) = try response.result.get() {
                        throw StoreError.unsupported(issues.map(\.message).joined(separator: "\n"))
                    }
                }
            }
            opened(try await store.openChannel(provider: provider, organization: organization.isEmpty ? nil : organization, members: members))
        } catch { self.error = QmsgStore.describe(error) }
    }
}
