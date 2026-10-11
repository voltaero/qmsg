import SwiftUI
import UIKit
import CoreImage.CIFilterBuiltins
import QmsgKit

struct SignInForm: View {
    @Environment(QmsgStore.self) private var store
    let provider: String
    let context: FunctionContext
    let function: Function
    @State private var fields: [String: String] = [:]
    @State private var choices: [String: Value] = [:]
    @State private var flags: [String: Bool] = [:]
    @State private var busy = false
    @State private var result: String?
    @State private var error: String?

    var body: some View {
        Form {
            if !function.description.isEmpty { Section { Text(function.description) } }
            if function.action == .login, let step = store.providers[provider]?.status.loginStep {
                Section("Log in") { LoginStepView(step: step) }
            }
            ForEach(function.inputs) { input in
                Section {
                    inputView(input).disabled(busy)
                } header: {
                    Text(input.label + (input.required ? " *" : ""))
                } footer: {
                    if !input.description.isEmpty { Text(input.description) }
                    if input.valueType == .list(.text) { Text("Enter one item per line.") }

                }
            }
            Section {
                if let error { ErrorNote(message: error) { self.error = nil } }
                if let result { Label(result, systemImage: "checkmark.circle.fill").foregroundStyle(.green).textSelection(.enabled) }
                Button {
                    run()
                } label: {
                    HStack { Text(function.label); Spacer(); if busy { ProgressView() } }
                }.disabled(busy)
            }
        }
        .navigationTitle(function.label)

    }

    @ViewBuilder
    private func inputView(_ input: FunctionInput) -> some View {
        if case .choices(let values) = input.hint {
            Picker(input.label, selection: Binding<Value?>(get: { choices[input.id] }, set: { choices[input.id] = $0 })) {
                Text("Choose…").tag(nil as Value?)
                ForEach(Array(values.enumerated()), id: \.offset) { _, choice in
                    Text(choice.label).tag(Optional(choice.value))
                }
            }
        } else if input.valueType == .boolean {
            Toggle(input.label, isOn: Binding(get: { flags[input.id] ?? false }, set: { flags[input.id] = $0 }))
        } else if input.hint == .secret {
            SecureField(input.label, text: field(input.id))
                .textInputAutocapitalization(.never).autocorrectionDisabled()
        } else {
            TextField(input.label, text: field(input.id), axis: input.hint == .multiline || input.valueType == .content || input.valueType == .list(.text) ? .vertical : .horizontal)
                .lineLimit(1...8).textInputAutocapitalization(.never).autocorrectionDisabled()
                .keyboardType(input.valueType == .integer ? .numbersAndPunctuation : .default)
        }
    }

    private func field(_ id: String) -> Binding<String> {
        Binding(get: { fields[id] ?? "" }, set: { fields[id] = $0 })
    }

    private func arguments() throws -> Arguments {
        var arguments: Arguments = [:]
        for input in function.inputs {
            if let choice = choices[input.id] { arguments[input.id] = choice; continue }
            if input.valueType == .boolean { arguments[input.id] = .boolean(flags[input.id] ?? false); continue }
            let raw = fields[input.id] ?? ""
            if raw.isEmpty {
                if input.required { throw StoreError.unsupported("Enter \(input.label).") }
                continue
            }
            switch input.valueType {
            case .text: arguments[input.id] = .text(raw)
            case .integer:
                guard let number = Int64(raw) else { throw StoreError.unsupported("\(input.label) must be a whole number.") }
                arguments[input.id] = .integer(number)
            case .content: arguments[input.id] = .content([.text(raw)])
            case .list(.text): arguments[input.id] = .list(raw.split(separator: "\n").map { .text(String($0)) })
            default:
                throw StoreError.unsupported("This sign-in method is not supported yet.")
            }
        }
        return arguments
    }

    private func run() {
        error = nil
        result = nil
        do {
            let arguments = try arguments()
            busy = true
            Task {
                defer { busy = false }
                do {
                    let reply = try await store.call(function, context: context, arguments: arguments, provider: provider,
                        timeout: function.action == .login ? .seconds(600) : .seconds(60))
                    result = try reply.get().summary
                } catch { self.error = QmsgStore.describe(error) }
            }
        } catch { self.error = QmsgStore.describe(error) }
    }
}

/// What the network asks for to sign in: a page to open, a QR code to scan, or both.
struct LoginStepView: View {
    @Environment(QmsgStore.self) private var store
    @State private var showingLogin = false
    let step: LoginStep

    var body: some View {
        Label {
            Text(step.message)
        } icon: {
            Image(systemName: "person.badge.key.fill").foregroundStyle(.orange)
        }
        if let link = step.link, let url = URL(string: link), ["https", "http"].contains(url.scheme?.lowercased()) {
            Button { showingLogin = true } label: {
                Label("Open sign-in page", systemImage: "safari")
            }
            .sheet(isPresented: $showingLogin) {
                NavigationStack {
                    LoginBrowser(url: url, endpoint: store.account?.endpoint)
                        .ignoresSafeArea(edges: .bottom)
                        .navigationTitle("Sign in")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { showingLogin = false } } }
                }
            }
        }
        if let code = step.qrCode {
            VStack(spacing: 8) {
                QRCodeView(code: code)
                Text("Scan with the app on your other device.").font(.caption).foregroundStyle(.secondary)
            }.frame(maxWidth: .infinity)
        }
    }
}

private struct QRCodeView: View {
    let code: String
    private var image: UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(code.utf8)
        guard let output = filter.outputImage,
              let cgImage = CIContext().createCGImage(output.transformed(by: CGAffineTransform(scaleX: 8, y: 8)), from: output.extent.applying(CGAffineTransform(scaleX: 8, y: 8))) else { return nil }
        return UIImage(cgImage: cgImage)
    }
    var body: some View {
        if let image {
            Image(uiImage: image).interpolation(.none).resizable().scaledToFit()
                .frame(maxWidth: 260).padding(12).background(.white)
                .accessibilityLabel("Provider login QR code")
        }
    }
}
