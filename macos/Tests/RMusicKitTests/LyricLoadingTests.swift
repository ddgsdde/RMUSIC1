import Foundation
import Testing
@testable import RMusicKit

@MainActor private final class LyricLoadingRuntime: RuntimeTransport {
    var calls: [(String, CheckedContinuation<JSONValue, Never>)] = []
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        if operation == "request" { return await withCheckedContinuation { calls.append((body["path"].text, $0)) } }
        return .null
    }
    func shutdown() async {}
    func finish(_ index: Int, _ text: String) {
        calls[index].1.resume(returning: .object(["format": .string("lrc"), "text": .string("[00:01]" + text), "offset_ms": .number(0)]))
    }
}
@Test @MainActor func lyricLoadingPrefetchesOneSuccessorAndDiscardsLateDisplayUpdates() async throws {
    let runtime = LyricLoadingRuntime(), store = MusicStore(runtime: runtime)
    let a = Track(id: "a", title: "A", artist: "QA", album: "", duration: 20, format: "flac", sample_rate: 44100, bit_depth: 16)
    let b = Track(id: "b", title: "B", artist: "QA", album: "", duration: 20, format: "flac", sample_rate: 48000, bit_depth: 16)
    let zone = Zone(id: "zone", name: "QA", endpoint_id: "fake", status: "playing", track: a, position: 0, volume: 1, muted: false, playback_id: "p", queue_index: 0, queue: [a, b])
    func waitUntil(_ predicate: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(2)
        while !predicate(), ContinuousClock.now < deadline { await Task.yield() }
        try #require(predicate())
    }
    store.details.update(a, store: store); store.details.prefetch(zone, store: store)
    try await waitUntil { runtime.calls.count == 1 }; #expect(runtime.calls[0].0.contains("/a/lyrics"))
    // Change before A finishes. A's delayed response must not replace B.
    store.details.update(b, store: store)
    try await waitUntil { runtime.calls.count == 2 }; runtime.finish(1, "Beta"); runtime.finish(0, "Alpha")
    try await waitUntil { store.details.document["text"].text.contains("Beta") && !store.details.loading }
    #expect(store.details.trackID == "b")
    store.details.update(a, store: store)
    try await waitUntil { store.details.document["text"].text.contains("Alpha") && !store.details.loading }
    #expect(runtime.calls.count == 2)
    store.details.reset(); store.details.update(a, store: store)
    try await waitUntil { runtime.calls.count == 3 }; runtime.finish(2, "New Alpha")
    try await waitUntil { !store.details.loading }; #expect(store.details.document["text"].text.contains("New Alpha"))
    await store.shutdown()
}
@Test @MainActor func pageSnapshotsRestoreDataWithoutSharingAccountsOrSecretSettings() {
    let snapshots = PageSnapshots(), owner = UUID()
    let document = JSONValue.object(["title": .string("Saved page")])
    snapshots.save(document, path: "/library/home", owner: owner)
    #expect(snapshots.value("/library/home", owner: owner)?["title"].text == "Saved page")
    snapshots.save(document, path: "/auth/sessions", owner: owner)
    #expect(snapshots.value("/auth/sessions", owner: owner) == nil)
    #expect(snapshots.value("/library/home", owner: UUID()) == nil)
    for n in 0..<40 { snapshots.save(document, path: "/library/albums?offset=\(n)", owner: owner) }
    #expect(snapshots.value("/library/albums?offset=0", owner: owner) == nil)
    #expect(snapshots.value("/library/albums?offset=39", owner: owner) != nil)
}
