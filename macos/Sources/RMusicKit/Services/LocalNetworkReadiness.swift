import Foundation
import Network

/// Let the responsible GUI app establish its local-network access before its
/// private runtime starts BSD-socket requests. No credentials are sent here.
@MainActor enum LocalNetworkReadiness {
  static func needsCheck(_ address: String) -> Bool {
    guard let host = URL(string: address)?.host?.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "[]")) else { return false }
    if host == "localhost" || host == "::1" || host.hasPrefix("127.") { return false }
    let parts = host.split(separator: ".").compactMap { Int($0) }
    if parts.count == 4 && parts.allSatisfy({ (0...255).contains($0) }) {
      return parts[0] == 10 || parts[0] == 192 && parts[1] == 168 ||
        parts[0] == 172 && (16...31).contains(parts[1]) || parts[0] == 169 && parts[1] == 254
    }
    return host.hasSuffix(".local") || !host.contains(".") && !host.contains(":") ||
      host.hasPrefix("fc") && host.contains(":") || host.hasPrefix("fd") && host.contains(":") || host.hasPrefix("fe80:")
  }

  static func check(_ address: String) async throws {
    guard needsCheck(address), let url = URL(string: address), let rawHost = url.host,
      let rawPort = UInt16(exactly: url.port ?? (url.scheme == "https" ? 443 : 80)),
      let port = NWEndpoint.Port(rawValue: rawPort) else { return }
    let host = rawHost.trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
    let attempt = LocalNetworkAttempt(host: host, port: port)
    try await withTaskCancellationHandler {
      try Task.checkCancellation()
      try await withCheckedThrowingContinuation { attempt.start($0) }
    } onCancel: { Task { @MainActor in attempt.finish(CancellationError()) } }
  }
}

@MainActor private final class LocalNetworkAttempt {
  let connection: NWConnection
  var continuation: CheckedContinuation<Void, Error>?
  var deadline: Task<Void, Never>?
  var denied = false
  init(host: String, port: NWEndpoint.Port) {
    connection = NWConnection(host: NWEndpoint.Host(host), port: port, using: .tcp)
  }
  func start(_ continuation: CheckedContinuation<Void, Error>) {
    self.continuation = continuation
    connection.stateUpdateHandler = { [weak self] state in
      Task { @MainActor in
        guard let self else { return }
        switch state {
        case .ready: self.finish(nil)
        case .waiting:
          if self.connection.currentPath?.unsatisfiedReason == .localNetworkDenied { self.denied = true }
        case .failed(let error): self.finish(error)
        default: break
        }
      }
    }
    deadline = Task { [weak self] in
      do { try await Task.sleep(for: .seconds(4)) } catch { return }
      guard let self else { return }
      self.finish(RuntimeFailure(message: self.denied
        ? "macOS 未允许本地网络访问。请在系统设置的隐私与安全→本地网络允许 R-Music，再重试。"
        : "局域网 Core 暂不可达，请检查 Wi-Fi、地址或外网备用入口。"))
    }
    connection.start(queue: .global(qos: .utility))
  }
  func finish(_ error: Error?) {
    guard let continuation else { return }
    self.continuation = nil
    deadline?.cancel(); deadline = nil
    connection.stateUpdateHandler = nil; connection.cancel()
    if let error { continuation.resume(throwing: error) } else { continuation.resume() }
  }
}
