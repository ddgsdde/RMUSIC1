import Foundation
import Darwin
import CoreFoundation

/// Decodes on FileHandle's background reader, applying pipe backpressure instead of queuing
/// arbitrarily many Data blocks onto the UI actor. The lock never touches an audio callback.
final class RuntimeResponseReader: @unchecked Sendable {
    private let lock = NSLock()
    private var buffer = Data()
    // Relative to buffer.startIndex: bytes before this offset contain no unconsumed newline.
    private var scanned = 0
    private var failed = false
    func read(_ data: Data) -> Result<[RuntimeResponse], Error> {
        lock.lock(); defer { lock.unlock() }
        guard !failed else { return .success([]) }
        buffer.append(data)
        var responses: [RuntimeResponse] = []
        do {
            var consumed = 0
            while let end = buffer.withUnsafeBytes({ (bytes: UnsafeRawBufferPointer) -> Int? in
                guard scanned < bytes.count, let base = bytes.baseAddress,
                      let newline = memchr(base.advanced(by: scanned), 10, bytes.count - scanned) else { return nil }
                return base.distance(to: UnsafeRawPointer(newline))
            }) {
                guard end - consumed <= 8 * 1024 * 1024 else { throw RuntimeFailure(message: "原生响应过大。") }
                let line = buffer[(buffer.startIndex + consumed)..<(buffer.startIndex + end)]
                responses.append(try autoreleasepool {
                    // Parse bytes once, without throwing type probes for every field. Drain
                    // Foundation temporaries on this long-lived FileHandle reader queue.
                    let tree = try JSONValue(wireObject: JSONSerialization.jsonObject(with: line))
                    return try tree.decode(RuntimeResponse.self)
                })
                consumed = end + 1
                scanned = consumed
            }
            if consumed == buffer.count {
                buffer = Data()
            } else if consumed > 0 {
                // removeFirst only advances Data's slice; repeated appends can retain
                // every consumed reply in one ever-growing backing allocation.
                buffer = buffer.withUnsafeBytes { bytes in
                    Data(bytes: bytes.baseAddress!.advanced(by: consumed), count: bytes.count - consumed)
                }
            }
            scanned = buffer.count
            guard buffer.count <= 8 * 1024 * 1024 else { throw RuntimeFailure(message: "原生响应过大。") }
            return .success(responses)
        } catch {
            failed = true; buffer = Data(); scanned = 0
            return .failure(RuntimeFailure(message: "原生协议解析失败或响应过大。"))
        }
    }
}

private extension JSONValue {
    init(wireObject value: Any) throws {
        if value is NSNull { self = .null }
        else if let number = value as? NSNumber {
            self = CFGetTypeID(number) == CFBooleanGetTypeID() ? .bool(number.boolValue) : .number(number.doubleValue)
        } else if let string = value as? String { self = .string(string) }
        else if let array = value as? [Any] { self = .array(try array.map { try Self(wireObject: $0) }) }
        else if let object = value as? [String: Any] { self = .object(try object.mapValues { try Self(wireObject: $0) }) }
        else { throw RuntimeFailure(message: "原生响应包含无效 JSON 类型。") }
    }
}
