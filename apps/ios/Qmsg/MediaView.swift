import SwiftUI
import UIKit
import QuickLook
import QmsgKit

struct MediaView: View {
    @Environment(QmsgStore.self) private var store
    let media: Media
    @State private var data: Data?
    @State private var loading = false
    @State private var error: String?
    @State private var file: URL?
    @State private var preview: URL?
    @State private var revealed = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if media.metadata.spoiler && !revealed {
                Button("Show spoiler") { revealed = true }
            } else {
                if let data, media.mime?.hasPrefix("image/") == true, let image = UIImage(data: data) {
                    Image(uiImage: image).resizable().scaledToFit().frame(maxHeight: 280)
                        .clipShape(RoundedRectangle(cornerRadius: 10))
                        .accessibilityLabel(media.metadata.description ?? media.name ?? "Image")
                }
                Button {
                    Task { await load() }
                } label: {
                    Label(media.name ?? "Attachment", systemImage: "doc")
                }.disabled(loading)
                if loading { ProgressView() }
                if let file { ShareLink(item: file) }
                if let error { Text(error).font(.caption).foregroundStyle(.red) }
            }
        }
        .quickLookPreview($preview)
        .onDisappear {
            if let file { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
            file = nil
            preview = nil
        }
    }

    private func load() async {
        if let file { preview = file; return }
        loading = true
        error = nil
        defer { loading = false }
        do {
            let bytes = try await store.data(for: media)
            data = bytes
            let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let name = (media.name ?? "attachment") as NSString
            let safeName = name.lastPathComponent
            let url = directory.appendingPathComponent(safeName.isEmpty || safeName == "." || safeName == ".." ? "attachment" : safeName)
            try bytes.write(to: url, options: .atomic)
            file = url
            preview = url
        } catch { self.error = QmsgStore.describe(error) }
    }
}
