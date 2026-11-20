import Foundation
import OSLog

/// Drain child diagnostics off the UI thread and keep signed media addresses private.
final class RuntimeDiagnostics: @unchecked Sendable {
    private let lock = NSLock()
    private var pending = Data()
    private let logger = Logger(subsystem: "com.rmusic.mac", category: "AudioRuntime")
    private static let address = try! NSRegularExpression(pattern: #"[a-zA-Z][a-zA-Z0-9+.-]*://[^\s\"<>]+"#)

    static func redact(_ text: String) -> String {
        address.stringByReplacingMatches(in: text, range: NSRange(text.startIndex..., in: text), withTemplate: "<地址已隐藏>")
    }

    func receive(_ data: Data) {
        lock.lock()
        defer { lock.unlock() }
        pending.append(data)
        while let end = pending.firstIndex(of: 10) {
            let line = Self.redact(String(decoding: pending[..<end].prefix(8192), as: UTF8.self))
            pending.removeSubrange(...end)
            if !line.isEmpty { logger.notice("\(line, privacy: .public)") }
        }
        if pending.isEmpty || pending.count > 8192 { pending = Data() }
        else { pending = pending.withUnsafeBytes { Data($0) } }
    }
}
