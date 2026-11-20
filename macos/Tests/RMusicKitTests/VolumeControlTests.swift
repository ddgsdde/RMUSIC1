import Foundation
import Testing
@testable import RMusicKit

@Test @MainActor func volumeDragKeepsOnlyTheNewestPendingValueAndReleasesItsDraft() async throws {
    let state = VolumeControlState()
    let gate = AsyncStream<Void>.makeStream()
    var sent: [Double] = []
    state.editing = true
    state.submit(0.1) { value in
        sent.append(value)
        if sent.count == 1 { for await _ in gate.stream { break } }
    }
    while sent.isEmpty { await Task.yield() }
    for value in [0.2, 0.3, 0.4, 0.9] { state.submit(value) { _ in Issue.record("A second writer was created") } }
    state.finishEditing(); gate.continuation.yield(()); gate.continuation.finish()
    for _ in 0..<100 { if !state.writing { break }; try await Task.sleep(for: .milliseconds(5)) }
    #expect(sent == [0.1, 0.9]); #expect(!state.writing && state.draft == nil)
    state.submit(.nan) { _ in Issue.record("Invalid volume sent") }
    #expect(!state.writing)
    state.reset()
}
