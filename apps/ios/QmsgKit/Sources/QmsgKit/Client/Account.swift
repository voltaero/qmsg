import Foundation
import Synchronization

#if canImport(Security)
import Security
#endif

/// A paired backend: where it is, how to trust it, and this device's token.
public struct Account: Codable, Hashable, Sendable {
    public var endpoint: ServerEndpoint
    public var deviceID: String
    public var token: String
    public var expiresAtMs: UInt64
    /// A rotation secret saved before calling `Rotate`. If the reply was
    /// lost, `<deviceID>.<pendingSecret>` may already be the live token.
    public var pendingSecret: String?

    public init(endpoint: ServerEndpoint, deviceID: String, token: String, expiresAtMs: UInt64, pendingSecret: String? = nil) {
        self.endpoint = endpoint
        self.deviceID = deviceID
        self.token = token
        self.expiresAtMs = expiresAtMs
        self.pendingSecret = pendingSecret
    }

    public var expiresAt: Date { Date(timeIntervalSince1970: TimeInterval(expiresAtMs) / 1000) }

    /// The token a pending rotation proposed.
    public var proposedToken: String? { pendingSecret.map { "\(deviceID).\($0)" } }
}

public protocol AccountStore: Sendable {
    func load() throws -> Account?
    func save(_ account: Account) throws
    func delete() throws
}

/// Keeps the account in memory, for tests and previews.
public final class MemoryAccountStore: AccountStore {
    private let account: Mutex<Account?>

    public init(_ account: Account? = nil) { self.account = Mutex(account) }

    public func load() throws -> Account? { account.withLock { $0 } }
    public func save(_ account: Account) throws { self.account.withLock { $0 = account } }
    public func delete() throws { account.withLock { $0 = nil } }
}

#if canImport(Security)
/// Keeps the account, token included, in the keychain. It never leaves the
/// device: no iCloud sync, no backups to other devices.
public final class KeychainAccountStore: AccountStore {
    private let service: String
    private let key = "account"

    public init(service: String = "app.qmsg.account") { self.service = service }

    private var query: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
        ]
    }

    public func load() throws -> Account? {
        var query = query
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = item as? Data else { throw KeychainError(status: status) }
        return try JSONDecoder().decode(Account.self, from: data)
    }

    public func save(_ account: Account) throws {
        let data = try JSONEncoder().encode(account)
        let attributes: [String: Any] = [
            kSecValueData as String: data,
            // Readable in the background after the first unlock, so the
            // event stream can resume; never synced or restored elsewhere.
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        var status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            status = SecItemAdd(query.merging(attributes) { $1 } as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw KeychainError(status: status) }
    }

    public func delete() throws {
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw KeychainError(status: status) }
    }
}

public struct KeychainError: Error, CustomStringConvertible {
    public var status: OSStatus
    public var description: String {
        (SecCopyErrorMessageString(status, nil) as String?) ?? "Keychain error \(status)"
    }
}
#endif
