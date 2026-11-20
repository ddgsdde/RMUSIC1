import Foundation
import Observation

/// One in-flight write and one newest value. Dragging cannot build a network queue.
@MainActor @Observable final class VolumeControlState {
    var draft: Double?
    var editing = false
    private(set) var writing = false
    @ObservationIgnored private var pending: Double?
    @ObservationIgnored private var task: Task<Void, Never>?
    @ObservationIgnored private var revision = UUID()
    func submit(_ value: Double, send: @escaping @MainActor (Double) async -> Void) {
        guard value.isFinite, (0...1).contains(value) else { return }
        draft = value; pending = value
        guard task == nil else { return }
        let owner = revision
        writing = true
        task = Task { [weak self] in
            defer {
                if let self, self.revision == owner { self.writing = false; self.task = nil; if !self.editing { self.draft = nil } }
            }
            while let next = self?.pending, !Task.isCancelled {
                self?.pending = nil
                await send(next)
                do { try await Task.sleep(for: .milliseconds(75)) } catch { return }
            }
        }
    }
    func finishEditing() { editing = false; if !writing { draft = nil } }
    func reset() { revision = UUID(); task?.cancel(); task = nil; pending = nil; draft = nil; writing = false; editing = false }
}
