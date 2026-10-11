import SwiftUI
import QmsgKit

enum MessageTool: String, Identifiable {
    case edit, react
    var id: String { rawValue }
}

struct MessageToolsView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let key: ChannelKey
    let message: Message
    let tool: MessageTool
    @State private var text = ""
    @State private var function: Function?
    @State private var reactions: [CompletionItem] = []
    @State private var loading = true
    @State private var busy = false
    @State private var error: String?
    private var context: FunctionContext { .message(channel: key.channel, id: message.id) }

    var body: some View {
        Form {
            if loading { ProgressView() }
            if !loading && function != nil {
                if tool == .edit {
                    TextField("Message", text: $text, axis: .vertical).lineLimit(4...12).disabled(busy)
                } else {
                    if reactions.isEmpty {
                        Text("No reactions available.").foregroundStyle(.secondary)
                    } else {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 52))], spacing: 10) {
                            ForEach(reactions, id: \.value) { reaction in
                                Button { Task { await run([StandardInput.reaction: reaction.value]) } } label: {
                                    Text(reaction.value.textValue ?? reaction.label).font(.largeTitle)
                                        .frame(width: 52, height: 52)
                                        .background(Color(uiColor: .secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                                }
                                .buttonStyle(.plain)
                                .accessibilityLabel(reaction.label)
                                .disabled(busy)
                            }
                        }.padding(.vertical, 6)
                    }
                }
            }
            if let error { ErrorNote(message: error) { self.error = nil } }
            if busy { ProgressView() }
        }
        .navigationTitle(tool == .edit ? "Edit message" : "React")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
            if tool == .edit {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") { Task { await run([StandardInput.content: .content([.text(text)] + message.content.filter { $0.text == nil })]) } }
                        .disabled(function == nil || busy || text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
        .task {
            text = message.content.compactMap(\.text).joined(separator: "\n")
            defer { loading = false }
            do {
                function = try await store.functions(context, provider: key.provider).first { $0.action == (tool == .edit ? .editMessage : .addReaction) }
                guard let function else { throw StoreError.unsupported(tool == .edit ? "This message cannot be edited." : "This message does not support reactions.") }
                if tool == .react, let input = function.inputs.first(where: { $0.id == StandardInput.reaction }) {
                    reactions = try await store.complete(function, input: input, context: context, arguments: [:], query: "", provider: key.provider)
                }
            } catch { self.error = QmsgStore.describe(error) }
        }
    }

    private func run(_ arguments: Arguments) async {
        guard let function else { return }
        busy = true
        error = nil
        defer { busy = false }
        do {
            _ = try await store.call(function, context: context, arguments: arguments, provider: key.provider).get()
            dismiss()
        } catch { self.error = QmsgStore.describe(error) }
    }
}
