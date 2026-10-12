import Crypto
import Foundation
import NIOSSL
import SwiftASN1
import X509

#if canImport(Darwin)
import Darwin
#elseif canImport(Glibc)
import Glibc
#elseif canImport(Musl)
import Musl
#endif

/// How the app reaches and trusts one qmsg backend. Saved with the device
/// token; it only ever comes from a scanned pairing payload.
public struct ServerEndpoint: Codable, Hashable, Sendable {
    public var host: String
    public var port: Int
    public var secure: Bool
    /// The pinned certificate from the QR code. Without one, a secure
    /// endpoint uses the platform's normal certificate validation, as for a
    /// tunnel with a public certificate.
    public var certificatePEM: String?
    public var certificateSHA256: String?

    public var displayName: String { "\(host):\(port)" }

    /// Validates a scanned payload: version, endpoint scheme and shape,
    /// certificate fingerprint and expiry.
    public init(payload: PairingPayload, now: Date = Date()) throws(PairingError) {
        guard payload.version == 1 else { throw .unsupportedVersion(payload.version) }
        guard payload.expiresAt > now else { throw .expired }
        guard let url = URLComponents(string: payload.endpoint),
              let scheme = url.scheme?.lowercased(),
              let rawHost = url.host, !rawHost.isEmpty,
              url.user == nil, url.password == nil, url.query == nil, url.fragment == nil,
              url.path.isEmpty || url.path == "/"
        else { throw .invalidEndpoint(payload.endpoint) }
        // URLComponents keeps the brackets of an IPv6 literal.
        let host = rawHost.hasPrefix("[") && rawHost.hasSuffix("]") ? String(rawHost.dropFirst().dropLast()) : rawHost
        switch scheme {
        case "https": secure = true
        case "http":
            // Plaintext is only for a local secure tunnel.
            guard Self.isLoopback(host) else { throw .insecureEndpoint }
            secure = false
        default: throw .invalidEndpoint(payload.endpoint)
        }
        self.host = host
        port = url.port ?? (secure ? 443 : 80)
        if let pem = payload.certificatePEM {
            guard let expected = payload.certificateSHA256?.lowercased() else { throw .fingerprintMismatch }
            let actual: String
            do { actual = try Self.fingerprint(pem: pem) } catch { throw .invalidCertificate }
            guard actual == expected else { throw .fingerprintMismatch }
            certificatePEM = pem
            certificateSHA256 = actual
        } else {
            certificatePEM = nil
            certificateSHA256 = nil
        }
    }

    public init(host: String, port: Int, secure: Bool, certificatePEM: String? = nil, certificateSHA256: String? = nil) {
        self.host = host
        self.port = port
        self.secure = secure
        self.certificatePEM = certificatePEM
        self.certificateSHA256 = certificateSHA256
    }

    /// Lowercase hex SHA-256 of the first certificate's DER encoding.
    public static func fingerprint(pem: String) throws -> String {
        let certificate = try NIOSSLCertificate(bytes: Array(pem.utf8), format: .pem)
        return hex(SHA256.hash(data: try certificate.toDERBytes()))
    }

    static func hex(_ digest: some Sequence<UInt8>) -> String {
        digest.map { String(format: "%02x", $0) }.joined()
    }

    static func isLoopback(_ host: String) -> Bool {
        if host.lowercased() == "localhost" { return true }
        if let v4 = ipBytes(host), v4.count == 4 { return v4[0] == 127 }
        if let v6 = ipBytes(host), v6.count == 16 { return v6 == Array(repeating: 0, count: 15) + [1] }
        return false
    }

    /// The bytes of an IPv4 or IPv6 literal, or nil for a host name.
    static func ipBytes(_ host: String) -> [UInt8]? {
        var v4 = in_addr()
        if inet_pton(AF_INET, host, &v4) == 1 {
            return withUnsafeBytes(of: &v4) { Array($0) }
        }
        var v6 = in6_addr()
        if inet_pton(AF_INET6, host, &v6) == 1 {
            return withUnsafeBytes(of: &v6) { Array($0) }
        }
        return nil
    }

    /// Accepts only the pinned certificate, and only when it names `host`.
    static func verify(leaf der: [UInt8], pinned: String, host: String) -> Bool {
        guard hex(SHA256.hash(data: der)) == pinned,
              let certificate = try? Certificate(derEncoded: der),
              let names = try? certificate.extensions.subjectAlternativeNames
        else { return false }
        let ip = ipBytes(host)
        return names.contains { name in
            switch name {
            case .dnsName(let dns): ip == nil && dns.lowercased() == host.lowercased()
            case .ipAddress(let octets): ip == Array(octets.bytes)
            default: false
            }
        }
    }
}

public enum PairingError: Error, Equatable, Sendable, CustomStringConvertible {
    case notAPairingCode
    case unsupportedVersion(Int)
    case expired
    case invalidEndpoint(String)
    case insecureEndpoint
    case invalidCertificate
    case fingerprintMismatch

    public var description: String {
        switch self {
        case .notAPairingCode: "That isn't a qmsg pairing code."
        case .unsupportedVersion(let v): "This pairing code is from a newer qmsg (version \(v)). Update the app."
        case .expired: "This pairing code has expired. Run `qmsg-orchestrator pair` again."
        case .invalidEndpoint(let e): "The server address “\(e)” isn't valid."
        case .insecureEndpoint: "The server must use HTTPS unless it is reached through a local tunnel."
        case .invalidCertificate: "The server certificate in this code can't be read."
        case .fingerprintMismatch: "The server certificate doesn't match its fingerprint."
        }
    }

    /// Parses the text of a scanned QR code.
    public static func payload(from text: String) throws(PairingError) -> PairingPayload {
        do {
            return try JSONDecoder().decode(PairingPayload.self, from: Data(text.utf8))
        } catch {
            throw .notAPairingCode
        }
    }
}
