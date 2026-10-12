import SwiftUI
import UIKit
import UniformTypeIdentifiers
import QmsgKit

/// The message field: plain, multiline text that also takes pasted images,
/// which a SwiftUI `TextField` drops.
struct ComposerTextView: View {
    @Binding var text: String
    var placeholder = "Message"
    var maxLines = 6
    /// Called with images pasted into the field, instead of inserting them.
    let pasteImages: ([QmsgStore.Attachment]) -> Void

    var body: some View {
        Field(text: $text, maxLines: maxLines, placeholder: placeholder, pasteImages: pasteImages)
            .overlay(alignment: .topLeading) {
                if text.isEmpty {
                    Text(placeholder).foregroundStyle(.tertiary).allowsHitTesting(false).accessibilityHidden(true)
                }
            }
    }

    private struct Field: UIViewRepresentable {
        @Binding var text: String
        let maxLines: Int
        let placeholder: String
        let pasteImages: ([QmsgStore.Attachment]) -> Void

        func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

        func makeUIView(context: Context) -> PastingTextView {
            let view = PastingTextView()
            view.delegate = context.coordinator
            view.font = .preferredFont(forTextStyle: .body)
            view.adjustsFontForContentSizeCategory = true
            view.textColor = .label
            view.backgroundColor = .clear
            view.allowsEditingTextAttributes = false
            view.textContainerInset = .zero
            view.textContainer.lineFragmentPadding = 0
            view.isScrollEnabled = true
            view.showsVerticalScrollIndicator = false
            view.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            view.text = text
            return view
        }

        func updateUIView(_ view: PastingTextView, context: Context) {
            context.coordinator.text = $text
            view.pasteImages = pasteImages
            view.accessibilityLabel = placeholder
            // Don't disturb a keyboard composing marked text.
            if view.markedTextRange == nil, view.text != text {
                view.text = text
            }
        }

        func sizeThatFits(_ proposal: ProposedViewSize, uiView view: PastingTextView, context: Context) -> CGSize? {
            guard let width = proposal.width, width.isFinite, width > 0 else { return nil }
            let font = view.font ?? .preferredFont(forTextStyle: .body)
            let fitting = view.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height
            let lines = max(1, maxLines)
            let range = font.lineHeight...(font.lineHeight * CGFloat(lines)).rounded(.up)
            return CGSize(width: width, height: min(max(fitting, range.lowerBound), range.upperBound))
        }

        final class Coordinator: NSObject, UITextViewDelegate {
            var text: Binding<String>
            init(text: Binding<String>) { self.text = text }

            func textViewDidChange(_ view: UITextView) {
                if text.wrappedValue != view.text { text.wrappedValue = view.text }
            }
        }
    }
}

/// A text view whose paste turns pasteboard images into attachments.
final class PastingTextView: UITextView {
    var pasteImages: (([QmsgStore.Attachment]) -> Void)?

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        // Checking for images doesn't read them, so it doesn't prompt.
        if action == #selector(paste(_:)), pasteImages != nil, UIPasteboard.general.hasImages { return true }
        return super.canPerformAction(action, withSender: sender)
    }

    override func paste(_ sender: Any?) {
        let pasteboard = UIPasteboard.general
        guard let pasteImages, pasteboard.hasImages else { return super.paste(sender) }
        var images: [QmsgStore.Attachment] = []
        var strings: [String] = []
        for item in pasteboard.items {
            // An item with an image is the image, even if it also carries a link.
            if let image = Self.image(in: item, number: images.count + 1) {
                images.append(image)
            } else if let string = item[UTType.utf8PlainText.identifier] as? String ?? item[UTType.plainText.identifier] as? String {
                strings.append(string)
            }
        }
        if !strings.isEmpty { insertText(strings.joined(separator: "\n")) }
        if !images.isEmpty { pasteImages(images) }
    }

    /// The image in a pasteboard item, keeping its original encoding when
    /// the pasteboard has it, so GIFs stay animated. Other formats, such as
    /// HEIC, become JPEG or PNG, as photos from the library do, since few
    /// networks take them.
    static func image(in item: [String: Any], number: Int) -> QmsgStore.Attachment? {
        let types = item.keys.compactMap { UTType($0) }.filter { $0.conforms(to: .image) }
        let preferred = types.sorted { rank($0) < rank($1) }
        for type in preferred {
            let found: (data: Data, type: UTType)?
            switch item[type.identifier] {
            case let data as Data where [UTType.gif, .png, .jpeg].contains(type): found = (data, type)
            case let data as Data: found = UIImage(data: data).flatMap(encode)
            case let image as UIImage: found = encode(image)
            default: found = nil
            }
            guard let found, !found.data.isEmpty, found.data.count <= MediaLoader.maxDownload else { continue }
            return attachment(found.data, type: found.type, number: number)
        }
        return nil
    }

    private static func rank(_ type: UTType) -> Int {
        [UTType.gif, .png, .jpeg, .heic, .webP].firstIndex(of: type) ?? 10
    }

    private static func encode(_ image: UIImage) -> (data: Data, type: UTType)? {
        let opaque = image.cgImage.map { [.none, .noneSkipFirst, .noneSkipLast].contains($0.alphaInfo) } ?? false
        if opaque, let jpeg = image.jpegData(compressionQuality: 0.9) { return (jpeg, .jpeg) }
        return image.pngData().map { ($0, .png) }
    }

    private static func attachment(_ data: Data, type: UTType, number: Int) -> QmsgStore.Attachment {
        let ext = type.preferredFilenameExtension ?? "img"
        let name = number == 1 ? "Pasted image.\(ext)" : "Pasted image \(number).\(ext)"
        return QmsgStore.Attachment(name: name, mime: type.preferredMIMEType ?? "image/\(ext)", data: data)
    }
}
