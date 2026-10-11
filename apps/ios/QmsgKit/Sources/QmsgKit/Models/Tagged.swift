import Foundation

/// Any JSON object key. Used for Serde's externally tagged enums and for
/// struct-like variant payloads.
struct AnyKey: CodingKey, Hashable {
    var stringValue: String
    var intValue: Int? { nil }
    init(_ string: String) { stringValue = string }
    init?(stringValue: String) { self.stringValue = stringValue }
    init?(intValue: Int) { nil }
}

/// One decoded Serde enum: a unit variant is a bare string such as `"Ready"`,
/// any other variant is an object with exactly one key, such as
/// `{"Failed":"reason"}`.
struct Tag {
    let name: String
    private let container: KeyedDecodingContainer<AnyKey>?

    init(from decoder: Decoder) throws {
        if let single = try? decoder.singleValueContainer(), let name = try? single.decode(String.self) {
            self.name = name
            container = nil
            return
        }
        let container = try decoder.container(keyedBy: AnyKey.self)
        guard container.allKeys.count == 1, let key = container.allKeys.first else {
            throw DecodingError.dataCorrupted(.init(
                codingPath: decoder.codingPath,
                debugDescription: "expected a string or an object with one key"
            ))
        }
        name = key.stringValue
        self.container = container
    }

    /// The variant's payload.
    func value<T: Decodable>(_ type: T.Type = T.self) throws -> T {
        guard let container else {
            throw DecodingError.dataCorrupted(.init(
                codingPath: [], debugDescription: "variant \(name) has no payload"
            ))
        }
        return try container.decode(T.self, forKey: AnyKey(name))
    }

    /// The fields of a struct-like variant.
    func fields() throws -> KeyedDecodingContainer<AnyKey> {
        guard let container else {
            throw DecodingError.dataCorrupted(.init(
                codingPath: [], debugDescription: "variant \(name) has no fields"
            ))
        }
        return try container.nestedContainer(keyedBy: AnyKey.self, forKey: AnyKey(name))
    }

    func unknown(_ type: Any.Type) -> DecodingError {
        .dataCorrupted(.init(codingPath: [], debugDescription: "unknown \(type) variant \(name)"))
    }
}

extension KeyedDecodingContainer where Key == AnyKey {
    func callAsFunction<T: Decodable>(_ key: String, _ type: T.Type = T.self) throws -> T {
        try decode(T.self, forKey: AnyKey(key))
    }

    func optional<T: Decodable>(_ key: String, _ type: T.Type = T.self) throws -> T? {
        try decodeIfPresent(T.self, forKey: AnyKey(key))
    }
}

extension Encoder {
    func unit(_ name: String) throws {
        var container = singleValueContainer()
        try container.encode(name)
    }

    func variant<T: Encodable>(_ name: String, _ value: T) throws {
        var container = container(keyedBy: AnyKey.self)
        try container.encode(value, forKey: AnyKey(name))
    }

    /// A struct-like variant; `fill` encodes its fields.
    func variant(_ name: String, fields fill: (inout KeyedEncodingContainer<AnyKey>) throws -> Void) throws {
        var outer = container(keyedBy: AnyKey.self)
        var inner = outer.nestedContainer(keyedBy: AnyKey.self, forKey: AnyKey(name))
        try fill(&inner)
    }
}

extension KeyedEncodingContainer where Key == AnyKey {
    mutating func set<T: Encodable>(_ key: String, _ value: T) throws {
        try encode(value, forKey: AnyKey(key))
    }
}

/// Arbitrary JSON, for provider-defined data the app only shows.
public enum JSON: Codable, Hashable, Sendable {
    case null
    case bool(Bool)
    case int(Int64)
    case uint(UInt64)
    case double(Double)
    case string(String)
    case array([JSON])
    case object([String: JSON])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let v = try? c.decode(Bool.self) { self = .bool(v) }
        else if let v = try? c.decode(Int64.self) { self = .int(v) }
        else if let v = try? c.decode(UInt64.self) { self = .uint(v) }
        else if let v = try? c.decode(Double.self) { self = .double(v) }
        else if let v = try? c.decode(String.self) { self = .string(v) }
        else if let v = try? c.decode([JSON].self) { self = .array(v) }
        else { self = .object(try c.decode([String: JSON].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let v): try c.encode(v)
        case .int(let v): try c.encode(v)
        case .uint(let v): try c.encode(v)
        case .double(let v): try c.encode(v)
        case .string(let v): try c.encode(v)
        case .array(let v): try c.encode(v)
        case .object(let v): try c.encode(v)
        }
    }
}

public enum QmsgJSON {
    public static func decode<T: Decodable>(_ type: T.Type, from string: String) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(string.utf8))
    }

    public static func encode<T: Encodable>(_ value: T) throws -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return String(decoding: try encoder.encode(value), as: UTF8.self)
    }
}
