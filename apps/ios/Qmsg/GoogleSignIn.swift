import AuthenticationServices
import SwiftUI
import QmsgKit

/// Google runs in the system authentication browser, with its callback bound to this app.
@MainActor
final class GoogleAuthentication: NSObject, ASWebAuthenticationPresentationContextProviding {
    private var session: ASWebAuthenticationSession?
    private var continuation: CheckedContinuation<URL, Error>?
    private var sessionID = UUID()

    func open(_ url: URL, callbackScheme: String) async throws -> URL {
        guard let window = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene })
            .flatMap(\.windows).first(where: \.isKeyWindow), !window.isHidden else {
            throw StoreError.unsupported("Open qmsg to sign in.")
        }
        return try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            let id = UUID()
            self.sessionID = id
            let session = ASWebAuthenticationSession(url: url, callbackURLScheme: callbackScheme) { callback, error in
                Task { @MainActor in
                    guard self.sessionID == id else { return }
                    let waiting = self.continuation
                    self.continuation = nil
                    self.session = nil
                    if let callback { waiting?.resume(returning: callback) }
                    else { waiting?.resume(throwing: error ?? CancellationError()) }
                }
            }
            session.presentationContextProvider = self
            self.session = session
            if !session.start() {
                self.continuation = nil
                self.session = nil
                continuation.resume(throwing: StoreError.unsupported("Couldn't open Google sign-in."))
            }
        }
    }

    func cancel() {
        sessionID = UUID()
        session?.cancel()
        session = nil
        continuation?.resume(throwing: CancellationError())
        continuation = nil
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).first(where: \.isKeyWindow) ?? ASPresentationAnchor()
    }
}

struct GoogleSignInView: View {
    @Environment(QmsgStore.self) private var store
    let provider: String
    let function: Function
    @State private var browser = GoogleAuthentication()
    @State private var busy = false
    @State private var done = false
    @State private var error: String?

    var body: some View {
        Form {
            Section {
                Text("Choose your Google account and allow qmsg to access your mail.")
                if let error { ErrorNote(message: error) { self.error = nil } }
                if done { Label("Signed in", systemImage: "checkmark.circle.fill").foregroundStyle(.green) }
                Button {
                    Task { await signIn() }
                } label: {
                    HStack { Text("Sign in with Google"); Spacer(); if busy { ProgressView() } }
                }.disabled(busy || done)
            }
        }
        .navigationTitle("Gmail")
        .onDisappear { browser.cancel() }
    }

    private func signIn() async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            let reply = try await store.call(function, context: .provider, arguments: [:], provider: provider).get()
            guard let raw = reply["url"]?.textValue, let url = URL(string: raw),
                  url.scheme == "https", url.host == "accounts.google.com",
                  let expectedState = reply["state"]?.textValue,
                  let request = URLComponents(url: url, resolvingAgainstBaseURL: false),
                  let redirect = request.queryItems?.first(where: { $0.name == "redirect_uri" })?.value,
                  let target = URL(string: redirect), let scheme = target.scheme,
                  scheme.hasPrefix("com.googleusercontent.apps."),
                  let types = Bundle.main.object(forInfoDictionaryKey: "CFBundleURLTypes") as? [[String: Any]],
                  types.contains(where: { ($0["CFBundleURLSchemes"] as? [String])?.contains(scheme) == true }) else {
                throw StoreError.unsupported("This Google client isn't configured for this qmsg app.")
            }
            let callback = try await browser.open(url, callbackScheme: scheme)
            guard callback.scheme == scheme, callback.host == target.host, callback.path == target.path,
                  let response = URLComponents(url: callback, resolvingAgainstBaseURL: false),
                  response.queryItems?.filter({ $0.name == "state" }).count == 1,
                  response.queryItems?.first(where: { $0.name == "state" })?.value == expectedState else {
                throw StoreError.unsupported("Google returned an invalid sign-in response. Try again.")
            }
            if response.queryItems?.contains(where: { $0.name == "error" }) == true { throw CancellationError() }
            guard response.queryItems?.filter({ $0.name == "code" }).count == 1,
                  let code = response.queryItems?.first(where: { $0.name == "code" })?.value, !code.isEmpty else {
                throw StoreError.unsupported("Google didn't finish signing in. Try again.")
            }
            guard let finish = try await store.functions(.provider, provider: provider).first(where: { $0.id == "finish_login" }) else {
                throw StoreError.unsupported("Sign-in expired. Try again.")
            }
            _ = try await store.call(finish, context: .provider, arguments: ["code": .text(code), "state": .text(expectedState)], provider: provider).get()
            done = true
        } catch {
            if !(error is CancellationError) && (error as? ASWebAuthenticationSessionError)?.code != .canceledLogin {
                self.error = QmsgStore.describe(error)
            }
            if let cancel = try? await store.functions(.provider, provider: provider).first(where: { $0.id == "cancel_login" }) {
                _ = try? await store.call(cancel, context: .provider, arguments: [:], provider: provider)
            }
        }
    }
}
