import Foundation
import Observation

/// View-owned writes. Navigation cancels the HTTP operation; a changed login cannot accept its result.
@MainActor @Observable
final class WorkflowAction {
  var busy = false
  var error: String?
  var status: Int?
  var notice: String?
  @ObservationIgnored private var task: Task<Void, Never>?
  @ObservationIgnored private var revision = UUID()
  func send(
    _ path: String, method: String = "POST", body: JSONValue = .object([:]),
    preflight: (@MainActor () async throws -> Void)? = nil, store: MusicStore,
    commit: @escaping @MainActor (JSONValue) -> Void = { _ in }
  ) {
    run(
      store: store, preflight: preflight,
      operation: { try await store.api(path, method: method, body: body) }, commit: commit)
  }
  func run(
    store: MusicStore, preflight: (@MainActor () async throws -> Void)? = nil,
    operation: @escaping @MainActor () async throws -> JSONValue,
    commit: @escaping @MainActor (JSONValue) -> Void = { _ in }
  ) {
    guard !busy else { return }
    let owner = store.sessionID
    let request = UUID()
    revision = request
    busy = true
    error = nil
    notice = nil
    status = nil
    task = Task { [weak self] in
      defer {
        if self?.revision == request { self?.busy = false; self?.task = nil }
      }
      do {
        try Task.checkCancellation()
        guard owner == store.sessionID else { throw CancellationError() }
        try await preflight?()
        try Task.checkCancellation()
        guard owner == store.sessionID else { throw CancellationError() }
        let value = try await operation()
        try Task.checkCancellation()
        guard self?.revision == request, owner == store.sessionID else { return }
        self?.notice = "已保存"
        commit(value)
      } catch is CancellationError {} catch {
        guard self?.revision == request, owner == store.sessionID else { return }
        self?.error = error.localizedDescription
        self?.status = (error as? RuntimeFailure)?.status
        if (error as? RuntimeFailure)?.status == 401 { await store.handle(error) }
      }
    }
  }
  func cancel() {
    revision = UUID()
    task?.cancel()
    task = nil
    busy = false
  }
}
