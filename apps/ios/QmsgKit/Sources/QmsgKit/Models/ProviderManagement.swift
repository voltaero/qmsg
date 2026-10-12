import Foundation

public struct ProviderCatalog: Codable, Hashable, Sendable {
    public var types: [ProviderType]
    public var accounts: [ManagedProvider]
}
public struct ProviderType: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var label: String
    public var fields: [ProviderField]
    public var defaults: [String: String]
}
public struct ProviderField: Codable, Hashable, Sendable, Identifiable {
    public var id: String
    public var label: String
    public var required: Bool
    public var secret: Bool
}
public struct ManagedProvider: Codable, Hashable, Sendable, Identifiable {
    public var name: String
    public var label: String
    public var type: String
    public var enabled: Bool
    public var settings: [String: String]
    public var id: String { name }
}
