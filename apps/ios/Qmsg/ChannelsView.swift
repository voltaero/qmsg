import SwiftUI
import QmsgKit

struct ChannelsView: View {
    @Environment(QmsgStore.self) private var store
    @State private var selected: ChannelKey?
    @State private var settings = false
    @State private var search = ""

    var body: some View {
        NavigationSplitView {
            List(selection: $selected) {
                if store.link != .live {
                    Section { ConnectionLabel(link: store.link) }
                }
                ForEach(store.sortedProviders) { provider in
                    Section {
                        NavigationLink {
                            ProviderView(providerName: provider.name)
                        } label: {
                            HStack {
                                Label(provider.name, systemImage: "person.crop.circle")
                                Spacer()
                                Text(provider.status.label).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        channelRows(provider.standalone, provider: provider.name, organization: nil)
                    } header: { Text(provider.name) }
                    ForEach(provider.sortedOrganizations) { organization in
                        Section(organization.name) {
                            channelRows(organization.scope, provider: provider.name, organization: organization.id)
                        }
                    }
                }
            }
            .overlay {
                if store.providers.isEmpty {
                    ContentUnavailableView("Waiting for accounts", systemImage: "bubble.left.and.bubble.right", description: Text("Your server will send your accounts and channels when it connects."))
                }
            }
            .navigationTitle("Chats")
            .searchable(text: $search, prompt: "Find a channel")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Settings", systemImage: "gearshape") { settings = true }
                }
            }
        } detail: {
            if let selected {
                ChatView(key: selected).id(selected)
            } else {
                ContentUnavailableView("Choose a chat", systemImage: "bubble.left.and.bubble.right", description: Text("Pick a channel to read and send messages."))
            }
        }
        .sheet(isPresented: $settings) { NavigationStack { SettingsView() } }
    }

    @ViewBuilder
    private func channelRows(_ scope: Scope, provider: String, organization: String?) -> some View {
        ForEach(scope.channelTree().filter { search.isEmpty || scope.title(of: $0.channel).localizedCaseInsensitiveContains(search) }) { node in
            let channel = node.channel
            let key = ChannelKey(provider: provider, channel: ChannelRef(organization: organization, channel: channel.id))
            if channel.isContainer {
                Text(scope.title(of: channel)).font(.caption.bold()).foregroundStyle(.secondary)
                    .padding(.leading, CGFloat(node.depth) * 12)
            } else {
                NavigationLink(value: key) {
                    HStack(spacing: 12) {
                        Image(systemName: channel.kind == .direct || channel.kind == .group ? "person.2" : "number")
                            .foregroundStyle(.secondary)
                        VStack(alignment: .leading, spacing: 3) {
                            Text(scope.title(of: channel))
                            if let last = store.activity[key]?.last {
                                Text(last.content.compactMap(\.text).joined(separator: " "))
                                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }
                        }
                        Spacer()
                        if let unread = store.activity[key]?.unread, unread > 0 {
                            Text("\(unread)").font(.caption.bold()).foregroundStyle(.white)
                                .padding(6).background(.tint, in: Capsule())
                        }
                    }.padding(.leading, CGFloat(node.depth) * 12)
                }
            }
        }
    }
}

struct ConnectionLabel: View {
    let link: QmsgStore.Link
    var body: some View {
        switch link {
        case .live: Label("Connected", systemImage: "checkmark.circle").foregroundStyle(.green)
        case .idle: Label("Offline", systemImage: "wifi.slash").foregroundStyle(.secondary)
        case .connecting: HStack { ProgressView(); Text("Connecting…") }
        case .retrying(let error):
            VStack(alignment: .leading) {
                Label("Reconnecting…", systemImage: "wifi.exclamationmark")
                Text(error).font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}
