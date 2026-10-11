import SwiftUI
import UIKit
import CoreImage.CIFilterBuiltins
import QmsgKit

struct ActionsView: View {
    @Environment(QmsgStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let provider: String
    let context: FunctionContext
    var initialContent: [Content] = []
    @State private var functions: [Function] = []
    @State private var loading = true
    @State private var error: String?

    var body: some View {
        List {
            if loading { ProgressView() }
            if let error { Text(error).foregroundStyle(.red) }
            ForEach(functions) { function in
                NavigationLink {
                    FunctionForm(provider: provider, context: context, function: function, initialContent: initialContent)
                } label: {
                    VStack(alignment: .leading) {
                        Text(function.label)
                        if !function.description.isEmpty { Text(function.description).font(.caption).foregroundStyle(.secondary) }
                    }
                }
            }
            if !loading && functions.isEmpty && error == nil { Text("No actions available here.").foregroundStyle(.secondary) }
        }
        .navigationTitle("Actions")
        .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } } }
        .task { await load() }
        .refreshable { await load() }
    }

    private func load() async {
        loading = true
        error = nil
        defer { loading = false }
        do { functions = try await store.functions(context, provider: provider) }
        catch { self.error = QmsgStore.describe(error) }
    }
}

struct FunctionForm: View {
    @Environment(QmsgStore.self) private var store
    let provider: String
    let context: FunctionContext
    let function: Function
    var initialContent: [Content] = []
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
                    if needsJSON(input) { Text("This input takes a structured value. See the server's function schema for its JSON format.") }
                }
            }
            Section {
                if let error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
                if let result { Text(result).textSelection(.enabled) }
                Button {
                    run()
                } label: {
                    HStack { Text(function.label); Spacer(); if busy { ProgressView() } }
                }.disabled(busy)
            }
        }
        .navigationTitle(function.label)
        .onAppear {
            if function.action == .editMessage && fields[StandardInput.content] == nil {
                fields[StandardInput.content] = initialContent.compactMap(\.text).joined(separator: "\n")
            }
        }
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

    private func needsJSON(_ input: FunctionInput) -> Bool {
        switch input.valueType {
        case .text, .integer, .boolean, .content, .list(.text): false
        default: true
        }
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
                do { arguments[input.id] = try JSONDecoder().decode(Value.self, from: Data(raw.utf8)) }
                catch { throw StoreError.unsupported("Check the JSON for \(input.label).") }
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

struct ProviderView: View {
    @Environment(QmsgStore.self) private var store
    let providerName: String

    var body: some View {
        List {
            if let provider = store.providers[providerName] {
                Section("Account") { Text(provider.status.label) }
                if let step = provider.status.loginStep {
                    Section("Log in") {
                        LoginStepView(step: step)
                    }
                }
                NavigationLink("Account actions") { ActionsView(provider: providerName, context: .provider) }
                ForEach(provider.sortedOrganizations) { organization in
                    NavigationLink("\(organization.name) actions") {
                        ActionsView(provider: providerName, context: .organization(organization.id))
                    }
                }
            } else { Text("This account is no longer available.") }
        }.navigationTitle(providerName)
    }
}

private struct LoginStepView: View {
    let step: LoginStep
    var body: some View {
        Text(step.message)
        if let link = step.link, let url = URL(string: link), ["https", "http"].contains(url.scheme?.lowercased()) {
            Link("Open login page", destination: url)
        }
        if let code = step.qrCode {
            QRCodeView(code: code)
            Text(code).font(.caption).textSelection(.enabled)
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
