import SwiftUI
import QmsgKit

struct ChatMembersView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let key: ChannelKey
    @State private var search = ""

    private var scope: Scope? { store.providers[key.provider]?.scope(key.channel.organization) }
    private var channel: Channel? { scope?.channels[key.channel.channel] }
    private var people: [User] {
        guard let scope, let channel else { return [] }
        return scope.members(in: channel).filter {
            search.isEmpty || $0.name.localizedCaseInsensitiveContains(search) || ($0.username?.localizedCaseInsensitiveContains(search) ?? false)
        }
    }

    var body: some View {
        List {
            Section {
                ForEach(people) { user in
                    HStack(spacing: 12) {
                        IconImage(media: user.avatar, size: 36, shape: Circle()) { InitialsAvatar(name: user.name, size: 36) }
                        VStack(alignment: .leading, spacing: 2) {
                            Text(user.name + (user.id == scope?.me ? " (You)" : ""))
                            if let username = user.username { Text(username).font(.caption).foregroundStyle(.secondary) }
                        }
                    }
                }
            } footer: {
                if channel?.members == nil { Text("Only members available to qmsg are shown. Channel access may differ.") }
            }
        }
        .overlay {
            if people.isEmpty {
                if search.isEmpty { ContentUnavailableView("No members available", systemImage: "person.2") }
                else { ContentUnavailableView.search(text: search) }
            }
        }
        .searchable(text: $search, prompt: "Find a member")
        .navigationTitle(channel?.members == nil ? "Server members" : "Members")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
    }
}
