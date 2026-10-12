import SwiftUI
import QmsgKit

/// Pick someone to message: tapping a person opens the chat, as in other
/// messaging apps. "New Group" picks several first.
struct NewChatView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(AccountsModel.self) private var accounts
    @Environment(\.dismiss) private var dismiss
    let opened: (ChannelKey) -> Void
    @State private var provider = ""
    @State private var query = ""
    /// Where to look for people: "" for the account's own contacts, or a server.
    @State private var organization = ""
    @State private var function: Function?
    @State private var contacts: [CompletionItem] = []
    @State private var selected: [CompletionItem] = []
    @State private var group = false
    @State private var loading = false
    /// The person being opened, or "" for a group.
    @State private var opening: String?
    @State private var error: String?

    private var context: FunctionContext { organization.isEmpty ? .provider : .organization(organization) }
    /// Mail accounts write from the Mail tab instead.
    private var chatAccounts: [ProviderState] {
        store.sortedProviders.filter { !accounts.isMail($0) }
    }
    private var membersInput: FunctionInput? {
        function?.inputs.first { $0.id == StandardInput.members }
    }
    /// The network takes more than one person per conversation.
    private var allowsGroups: Bool {
        if case .list = membersInput?.valueType { return true }
        return false
    }
    private var busy: Bool { opening != nil }

    var body: some View {
        List {
            if chatAccounts.count > 1 || !(store.providers[provider]?.sortedOrganizations.isEmpty ?? true) {
                Section { sourcePickers }
            }
            if group {
                Section {
                    if selected.isEmpty {
                        Text("Choose people below.").foregroundStyle(.secondary)
                    }
                    ForEach(selected, id: \.value) { contact in
                        HStack {
                            InitialsAvatar(name: contact.label, size: 28)
                            Text(contact.label)
                            Spacer()
                            Button("Remove \(contact.label)", systemImage: "xmark.circle.fill") {
                                selected.removeAll { $0.value == contact.value }
                            }
                            .labelStyle(.iconOnly).foregroundStyle(.secondary).buttonStyle(.plain)
                        }
                    }
                } header: { Text("To") }
            } else if allowsGroups {
                Section {
                    Button { group = true } label: {
                        Label("New Group", systemImage: "person.2.fill")
                    }
                    .disabled(busy)
                }
            }
            Section {
                if loading && contacts.isEmpty { ProgressView().frame(maxWidth: .infinity) }
                ForEach(contacts, id: \.value) { contact in
                    contactRow(contact)
                }
                if !loading && contacts.isEmpty {
                    Text(function == nil && !provider.isEmpty ? "This account can't start new chats here." : query.isEmpty ? "No people to suggest." : "No one matches “\(query)”.")
                        .foregroundStyle(.secondary)
                }
            } header: {
                Text(query.isEmpty ? "Suggested" : "People")
            }
            if let error { Section { ErrorNote(message: error) { self.error = nil } } }
        }
        .navigationTitle(group ? "New Group" : "New Message")
        .navigationBarTitleDisplayMode(.inline)
        .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Name or username")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") {
                    if group { group = false; selected = [] } else { dismiss() }
                }
            }
            if group {
                ToolbarItem(placement: .confirmationAction) {
                    Button { Task { await start(selected) } } label: {
                        if opening == "" { ProgressView() } else { Text("Create").bold() }
                    }
                    .disabled(selected.isEmpty || busy)
                }
            }
        }
        .onAppear { if provider.isEmpty { provider = chatAccounts.first?.name ?? "" } }
        .onChange(of: provider) { _, _ in
            // Networks without a contact list of their own find people in a server.
            organization = store.providers[provider]?.standalone.users.isEmpty == true ? (store.providers[provider]?.sortedOrganizations.first?.id ?? "") : ""
            selected = []; query = ""; function = nil; contacts = []; group = false
        }
        .onChange(of: organization) { _, _ in selected = []; function = nil; contacts = [] }
        .task(id: provider + "\n" + organization + "\n" + query) { await search() }
    }

    @ViewBuilder
    private var sourcePickers: some View {
        if chatAccounts.count > 1 {
            Picker("Account", selection: $provider) {
                ForEach(chatAccounts) { account in
                    Text(accounts.label(for: account.name)).tag(account.name)
                }
            }
            .disabled(busy)
        }
        if let account = store.providers[provider], !account.sortedOrganizations.isEmpty {
            Picker("Find people in", selection: $organization) {
                Text("Contacts").tag("")
                ForEach(account.sortedOrganizations) { server in Text(server.name).tag(server.id) }
            }
            .disabled(busy)
        }
    }

    private func contactRow(_ contact: CompletionItem) -> some View {
        let picked = selected.contains { $0.value == contact.value }
        return Button {
            if group {
                if picked { selected.removeAll { $0.value == contact.value } } else { selected.append(contact) }
            } else {
                Task { await start([contact]) }
            }
        } label: {
            HStack(spacing: 12) {
                InitialsAvatar(name: contact.label, size: 36)
                VStack(alignment: .leading, spacing: 1) {
                    Text(contact.label).foregroundStyle(.primary)
                    if let detail = contact.description, !detail.isEmpty {
                        Text(detail).font(.caption).foregroundStyle(.secondary)
                    }
                }
                Spacer()
                if group {
                    Image(systemName: picked ? "checkmark.circle.fill" : "circle")
                        .foregroundStyle(picked ? AnyShapeStyle(.tint) : AnyShapeStyle(.tertiary)).font(.title3)
                } else if opening == contact.value.textValue {
                    ProgressView()
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(busy)
    }

    private func search() async {
        guard !provider.isEmpty else { return }
        loading = true
        error = nil
        defer { loading = false }
        do {
            try await Task.sleep(for: .milliseconds(200))
            if function == nil { function = try await store.functions(context, provider: provider).first { $0.action == .openChannel } }
            guard let function, let input = membersInput else { contacts = []; return }
            let items = try await store.complete(function, input: input, context: context, arguments: [:], query: query, provider: provider)
            try Task.checkCancellation()
            contacts = items.filter { $0.value.textValue != store.providers[provider]?.scope(organization.isEmpty ? nil : organization)?.me }
        } catch is CancellationError { }
        catch { self.error = QmsgStore.describe(error) }
    }

    private func start(_ people: [CompletionItem]) async {
        let members = people.compactMap { $0.value.textValue }
        guard let function, !members.isEmpty else { return }
        opening = group ? "" : members[0]
        error = nil
        defer { opening = nil }
        do {
            let arguments: Arguments = [StandardInput.members: .list(members.map { .text($0) })]
            if let input = membersInput, let helper = input.verification {
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
