import SwiftUI
import WebKit
import Security
import CryptoKit
import QmsgKit

/// The server's login viewer uses the same certificate trusted during pairing.
struct LoginBrowser: UIViewRepresentable {
    let url: URL
    let endpoint: ServerEndpoint?

    func makeCoordinator() -> Coordinator { Coordinator(endpoint: endpoint) }
    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.navigationDelegate = context.coordinator
        view.load(URLRequest(url: url))
        return view
    }
    func updateUIView(_ view: WKWebView, context: Context) { }

    final class Coordinator: NSObject, WKNavigationDelegate {
        let endpoint: ServerEndpoint?
        init(endpoint: ServerEndpoint?) { self.endpoint = endpoint }

        func webView(_ webView: WKWebView, didReceive challenge: URLAuthenticationChallenge,
                     completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
            guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
                  let endpoint, let pin = endpoint.certificateSHA256,
                  challenge.protectionSpace.host == endpoint.host else {
                completionHandler(.performDefaultHandling, nil)
                return
            }
            guard let trust = challenge.protectionSpace.serverTrust,
                  let leaf = (SecTrustCopyCertificateChain(trust) as? [SecCertificate])?.first else {
                completionHandler(.cancelAuthenticationChallenge, nil)
                return
            }
            let data = SecCertificateCopyData(leaf) as Data
            let actual = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
            guard actual == pin.lowercased() else {
                completionHandler(.cancelAuthenticationChallenge, nil)
                return
            }
            SecTrustSetAnchorCertificates(trust, [leaf] as CFArray)
            SecTrustSetAnchorCertificatesOnly(trust, true)
            guard SecTrustEvaluateWithError(trust, nil) else {
                completionHandler(.cancelAuthenticationChallenge, nil)
                return
            }
            completionHandler(.useCredential, URLCredential(trust: trust))
        }
    }
}
