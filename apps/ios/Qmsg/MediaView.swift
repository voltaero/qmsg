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
    /// Small images show inline without a tap.
    private var showsInline: Bool { isImage && (!media.metadata.spoiler || revealed) && (media.size ?? 0) <= 8_000_000 }

    /// Decoded images, so a row that is rebuilt while scrolling shows its
    /// image at once instead of a placeholder of another size.
    private static let images = NSCache<NSString, UIImage>()

    /// The image, if its bytes are already here.
    private var image: UIImage? {
        guard isImage else { return nil }
        let key = MediaLoader.key(media).map { $0 as NSString }
        if let key, let hit = Self.images.object(forKey: key) { return hit }
        guard let bytes = data ?? store.media.cached(media), let image = UIImage(data: bytes) else { return nil }
        if let key { Self.images.setObject(image, forKey: key) }
        return image
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if media.metadata.spoiler && !revealed {
                Button { revealed = true } label: {
                    Label("Spoiler — tap to show", systemImage: "eye.slash")
                        .font(.footnote).padding(10)
                        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10))
                }.buttonStyle(.plain)
            } else if showsInline || image != nil {
                inlineImage
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
        .task(id: revealed) {
            if showsInline, image == nil { await fetch() }
        }
        .onDisappear {
            if let file { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
            file = nil
            preview = nil
        }
    }

    /// The image in a frame of its final shape, so loading it doesn't
    /// change the row's height when the provider gave the size.
    private var inlineImage: some View {
        let image = image
        let ratio = media.aspectRatio.map { CGFloat($0) }
            ?? image.map { $0.size.width / max($0.size.height, 1) }
            ?? 4 / 3
        return Button { Task { await open() } } label: {
            Color.clear
                .aspectRatio(ratio, contentMode: .fit)
                .frame(maxWidth: 260, maxHeight: 280)
                .overlay {
                    if let image {
                        Image(uiImage: image).resizable().scaledToFill()
                    } else {
                        Rectangle().fill(.quaternary)
                            .overlay {
                                if loading { ProgressView() }
                                else if error != nil { Image(systemName: "photo").font(.title2).foregroundStyle(.secondary) }
                            }
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        }
        .buttonStyle(.plain)
        .disabled(loading)
        .accessibilityLabel(media.metadata.description ?? media.name ?? "Image")
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
