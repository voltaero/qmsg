import SwiftUI
import UIKit
import QuickLook
// The Linux toolchain needs the SwiftUI overlay imported explicitly.
import _QuickLook_SwiftUI
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

    private var isImage: Bool { media.mime?.hasPrefix("image/") == true }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if media.metadata.spoiler && !revealed {
                Button { revealed = true } label: {
                    Label("Spoiler — tap to show", systemImage: "eye.slash")
                        .font(.footnote).padding(10)
                        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10))
                }.buttonStyle(.plain)
            } else if let data, isImage, let image = UIImage(data: data) {
                Button { Task { await open() } } label: {
                    Image(uiImage: image).resizable().scaledToFit().frame(maxWidth: 260, maxHeight: 280)
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                }
                .buttonStyle(.plain)
                .accessibilityLabel(media.metadata.description ?? media.name ?? "Image")
            } else {
                Button { Task { await open() } } label: {
                    HStack(spacing: 10) {
                        Image(systemName: isImage ? "photo" : "doc.fill").font(.title3)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(media.name ?? "Attachment").lineLimit(1)
                            if let size = media.size {
                                Text(ByteCountFormatter.string(fromByteCount: Int64(size), countStyle: .file)).font(.caption2).opacity(0.7)
                            }
                        }
                        if loading { ProgressView().controlSize(.small) }
                    }
                    .padding(10)
                    .background(.black.opacity(0.06), in: RoundedRectangle(cornerRadius: 10))
                }
                .buttonStyle(.plain)
                .disabled(loading)
            }
            if let file, !isImage { ShareLink(item: file).font(.caption) }
            if let error { Text(error).font(.caption).foregroundStyle(.red) }
        }
        .quickLookPreview($preview)
        .task {
            // Small images show inline without a tap.
            if isImage, data == nil, !media.metadata.spoiler, (media.size ?? 0) <= 8_000_000 { await fetch() }
        }
        .onDisappear {
            if let file { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
            file = nil
            preview = nil
        }
    }

    private func open() async {
        if file == nil { await fetch() }
        preview = file
    }

    private func fetch() async {
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
        } catch { self.error = QmsgStore.describe(error) }
    }
}
