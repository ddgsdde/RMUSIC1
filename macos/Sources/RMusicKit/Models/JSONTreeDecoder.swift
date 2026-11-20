import Foundation

/// Decode the existing IPC tree directly. Large queues retain their metadata through COW,
/// instead of encoding a second JSON body and allocating the same tree again.
struct JSONTreeDecoder: Decoder, SingleValueDecodingContainer {
    let value: JSONValue
    var codingPath: [any CodingKey] = []
    var userInfo: [CodingUserInfoKey: Any] { [:] }
    func mismatch(_ type: Any.Type) -> DecodingError {
        .typeMismatch(type, .init(codingPath: codingPath, debugDescription: "JSON value has a different type"))
    }
    func child(_ value: JSONValue, _ key: any CodingKey) -> Self { .init(value: value, codingPath: codingPath + [key]) }
    func container<Key: CodingKey>(keyedBy type: Key.Type) throws -> KeyedDecodingContainer<Key> {
        guard case .object(let fields) = value else { throw mismatch([String:JSONValue].self) }
        return KeyedDecodingContainer(Fields(owner: self, fields: fields))
    }
    func unkeyedContainer() throws -> any UnkeyedDecodingContainer {
        guard case .array(let items) = value else { throw mismatch([JSONValue].self) }
        return Items(owner: self, items: items)
    }
    func singleValueContainer() throws -> any SingleValueDecodingContainer { self }
    func decodeNil() -> Bool { if case .null = value { return true };return false }
    func decode(_ type: Bool.Type) throws -> Bool { guard case .bool(let v) = value else { throw mismatch(type) };return v }
    func decode(_ type: String.Type) throws -> String { guard case .string(let v) = value else { throw mismatch(type) };return v }
    func decode(_ type: Double.Type) throws -> Double { guard case .number(let v) = value, v.isFinite else { throw mismatch(type) };return v }
    func decode(_ type: Float.Type) throws -> Float { let v = Float(try decode(Double.self));guard v.isFinite else { throw mismatch(type) };return v }
    private func integer<T: FixedWidthInteger>(_ type: T.Type) throws -> T {
        guard case .number(let v) = value, let n = T(exactly: v) else { throw mismatch(type) };return n
    }
    func decode(_ type: Int.Type) throws -> Int { try integer(type) }
    func decode(_ type: Int8.Type) throws -> Int8 { try integer(type) }
    func decode(_ type: Int16.Type) throws -> Int16 { try integer(type) }
    func decode(_ type: Int32.Type) throws -> Int32 { try integer(type) }
    func decode(_ type: Int64.Type) throws -> Int64 { try integer(type) }
    func decode(_ type: UInt.Type) throws -> UInt { try integer(type) }
    func decode(_ type: UInt8.Type) throws -> UInt8 { try integer(type) }
    func decode(_ type: UInt16.Type) throws -> UInt16 { try integer(type) }
    func decode(_ type: UInt32.Type) throws -> UInt32 { try integer(type) }
    func decode(_ type: UInt64.Type) throws -> UInt64 { try integer(type) }
    func decode<T: Decodable>(_ type: T.Type) throws -> T {
        if type == JSONValue.self { return value as! T }
        if type == Data.self {
            guard let data = Data(base64Encoded: try decode(String.self)) else { throw mismatch(type) };return data as! T
        }
        if type == URL.self {
            guard let url = URL(string: try decode(String.self)) else { throw mismatch(type) };return url as! T
        }
        return try T(from: self)
    }
    private struct Key: CodingKey {
        let stringValue: String
        var intValue: Int? { Int(stringValue) }
        init(stringValue: String) { self.stringValue = stringValue }
        init(intValue: Int) { stringValue = String(intValue) }
    }
    private struct Fields<K: CodingKey>: KeyedDecodingContainerProtocol {
        let owner: JSONTreeDecoder
        let fields: [String:JSONValue]
        var codingPath: [any CodingKey] { owner.codingPath }
        var allKeys: [K] { fields.keys.compactMap(K.init(stringValue:)) }
        func contains(_ key: K) -> Bool { fields[key.stringValue] != nil }
        func child(_ key: K) throws -> JSONTreeDecoder {
            guard let value = fields[key.stringValue] else { throw DecodingError.keyNotFound(key,.init(codingPath:codingPath,debugDescription:"Missing JSON key")) }
            return owner.child(value,key)
        }
        func decodeNil(forKey key: K) throws -> Bool { try child(key).decodeNil() }
        func decode<T: Decodable>(_ type: T.Type, forKey key: K) throws -> T { try child(key).decode(type) }
        func decode(_ type: Bool.Type, forKey key: K) throws -> Bool { try child(key).decode(type) }
        func decode(_ type: String.Type, forKey key: K) throws -> String { try child(key).decode(type) }
        func decode(_ type: Double.Type, forKey key: K) throws -> Double { try child(key).decode(type) }
        func decode(_ type: Float.Type, forKey key: K) throws -> Float { try child(key).decode(type) }
        func decode(_ type: Int.Type, forKey key: K) throws -> Int { try child(key).decode(type) }
        func decode(_ type: Int8.Type, forKey key: K) throws -> Int8 { try child(key).decode(type) }
        func decode(_ type: Int16.Type, forKey key: K) throws -> Int16 { try child(key).decode(type) }
        func decode(_ type: Int32.Type, forKey key: K) throws -> Int32 { try child(key).decode(type) }
        func decode(_ type: Int64.Type, forKey key: K) throws -> Int64 { try child(key).decode(type) }
        func decode(_ type: UInt.Type, forKey key: K) throws -> UInt { try child(key).decode(type) }
        func decode(_ type: UInt8.Type, forKey key: K) throws -> UInt8 { try child(key).decode(type) }
        func decode(_ type: UInt16.Type, forKey key: K) throws -> UInt16 { try child(key).decode(type) }
        func decode(_ type: UInt32.Type, forKey key: K) throws -> UInt32 { try child(key).decode(type) }
        func decode(_ type: UInt64.Type, forKey key: K) throws -> UInt64 { try child(key).decode(type) }
        func nestedContainer<N: CodingKey>(keyedBy type: N.Type, forKey key: K) throws -> KeyedDecodingContainer<N> { try child(key).container(keyedBy:type) }
        func nestedUnkeyedContainer(forKey key: K) throws -> any UnkeyedDecodingContainer { try child(key).unkeyedContainer() }
        func superDecoder() throws -> any Decoder { owner.child(fields["super"] ?? .null, JSONTreeDecoder.Key(stringValue:"super")) }
        func superDecoder(forKey key: K) throws -> any Decoder { try child(key) }
    }
    private struct Items: UnkeyedDecodingContainer {
        let owner: JSONTreeDecoder
        let items: [JSONValue]
        var currentIndex = 0
        var codingPath: [any CodingKey] { owner.codingPath }
        var count: Int? { items.count }
        var isAtEnd: Bool { currentIndex >= items.count }
        func child() throws -> JSONTreeDecoder {
            guard !isAtEnd else { throw DecodingError.valueNotFound(JSONValue.self,.init(codingPath:codingPath,debugDescription:"End of JSON array")) }
            return owner.child(items[currentIndex],Key(intValue:currentIndex))
        }
        mutating func decodeNil() throws -> Bool { if try child().decodeNil() { currentIndex += 1;return true };return false }
        mutating func decode<T: Decodable>(_ type: T.Type) throws -> T { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: Bool.Type) throws -> Bool { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: String.Type) throws -> String { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: Double.Type) throws -> Double { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: Float.Type) throws -> Float { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: Int.Type) throws -> Int { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: Int8.Type) throws -> Int8 { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: Int16.Type) throws -> Int16 { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: Int32.Type) throws -> Int32 { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: Int64.Type) throws -> Int64 { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: UInt.Type) throws -> UInt { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: UInt8.Type) throws -> UInt8 { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: UInt16.Type) throws -> UInt16 { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: UInt32.Type) throws -> UInt32 { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func decode(_ type: UInt64.Type) throws -> UInt64 { let value=try child().decode(type);currentIndex += 1;return value }
        mutating func nestedContainer<N: CodingKey>(keyedBy type: N.Type) throws -> KeyedDecodingContainer<N> { let value=try child().container(keyedBy:type);currentIndex += 1;return value }
        mutating func nestedUnkeyedContainer() throws -> any UnkeyedDecodingContainer { let value=try child().unkeyedContainer();currentIndex += 1;return value }
        mutating func superDecoder() throws -> any Decoder { let value=try child();currentIndex += 1;return value }
    }
}
