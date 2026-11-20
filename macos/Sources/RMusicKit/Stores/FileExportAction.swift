import Foundation
import Observation

@MainActor @Observable final class FileExportAction {
  var busy = false
  var error: String?
  @ObservationIgnored private var task: Task<Void, Never>?
  @ObservationIgnored private var revision = UUID()
  func save(_ path: String, name: String, store: MusicStore) {
    guard !busy else { return }
    busy = true
    error = nil
    let owner = store.sessionID
    let request = UUID()
    revision = request
    task = Task { [weak self] in
      defer {
        if self?.revision == request {
          self?.busy = false
          self?.task = nil
        }
      }
      do {
        try Task.checkCancellation()
        guard owner == store.sessionID else { throw CancellationError() }
        try await CoreFileService.save(path, name: name, store: store)
      } catch is CancellationError {} catch {
        if self?.revision == request, owner == store.sessionID {
          self?.error = error.localizedDescription
          if (error as? RuntimeFailure)?.status == 401 { await store.handle(error) }
        }
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
