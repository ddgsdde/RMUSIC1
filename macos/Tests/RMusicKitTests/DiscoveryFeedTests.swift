import Foundation
import Testing
@testable import RMusicKit

@MainActor private final class FeedRuntime: RuntimeTransport {
    var calls: [String] = []
    var fail = false
    var staged = false
    var fixture: JSONValue?
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        let path = body["path"].text
        calls.append(path)
        if path.hasPrefix("/recommendations") {
            try await Task.sleep(for: .milliseconds(30))
            if fail { throw RuntimeFailure(message: "fixture unavailable", status: 500) }
            if let fixture { return fixture }
            let initial = staged && path.contains("&quick=true")
            return .object(["items": .array([.object(["id": .string(initial ? "initial" : "kept")])]), "refreshing": .bool(initial)])
        }
        return .object(["albums": .array([])])
    }
    func shutdown() async {}
}

@Test @MainActor func discoveryPutsCloudShelfTracksInTheSameListWithoutDuplicateRecordings() async throws {
    let runtime = FeedRuntime(), store = MusicStore(runtime: runtime)
    runtime.fixture = try JSONDecoder().decode(JSONValue.self, from: Data("""
    {"items":[{"id":"local","title":"Local","artist":"","album":"","duration":90,"format":"flac","sample_rate":44100,"bit_depth":16,"recording_id":"shared"}],
     "sections":[{"id":"cloud","title":"云端探索","items":[
       {"id":"duplicate","title":"Duplicate","artist":"","album":"","duration":90,"format":"flac","sample_rate":44100,"bit_depth":16,"recording_id":"shared"},
       {"id":"online","title":"Online","artist":"","album":"","duration":90,"format":"flac","sample_rate":44100,"bit_depth":16,"recording_id":"unique"}
     ]}]}
    """.utf8))
    await store.discoveryFeed.load(store)
    #expect(store.discoveryFeed.items.map(\.id) == ["local", "online"])
    #expect(store.discoveryFeed.shelves.isEmpty)
    #expect(runtime.calls == ["/recommendations/discover?limit=24&quick=true"])
    await store.shutdown()
}

@Test @MainActor func discoveryReusesCachedPagesWithoutRequestingHome() async {
    let runtime = FeedRuntime(), store = MusicStore(runtime: runtime)
    await store.discoveryFeed.load(store)
    #expect(runtime.calls.count == 1)
    await store.discoveryFeed.load(store)
    #expect(runtime.calls.count == 1)
    runtime.fail = true
    await store.discoveryFeed.load(store, refresh: true)
    #expect(runtime.calls.count == 2)
    #expect(store.discoveryFeed.recommendations.value["items"].array.first?["id"].text == "kept")
    #expect(store.discoveryFeed.recommendations.error != nil)
    weak var previous = store.discoveryFeed.recommendations
    store.discoveryFeed.reset()
    #expect(previous == nil)
    #expect(store.discoveryFeed.recommendations.value.object.isEmpty)
    await store.shutdown()
}

@Test @MainActor func discoveryPublishesPreliminaryContentWhileCoreFinishesCloudChecks() async {
    let runtime = FeedRuntime(), store = MusicStore(runtime: runtime)
    runtime.staged = true
    let task = Task { await store.discoveryFeed.load(store) }
    while runtime.calls.count < 2 { await Task.yield() }
    #expect(store.discoveryFeed.recommendations.value["items"].array.first?["id"].text == "initial")
    #expect(store.discoveryFeed.updating)
    await task.value
    #expect(store.discoveryFeed.recommendations.value["items"].array.first?["id"].text == "kept")
    #expect(!store.discoveryFeed.updating)
    await store.shutdown()
}

@Test @MainActor func cancelledDiscoveryLoadDoesNotPublishOrRetainItsResponse() async {
    let runtime = FeedRuntime(), store = MusicStore(runtime: runtime)
    let task = Task { await store.discoveryFeed.load(store) }
    while runtime.calls.isEmpty { await Task.yield() }
    task.cancel(); await task.value
    #expect(store.discoveryFeed.recommendations.value.object.isEmpty)
    #expect(!store.discoveryFeed.recommendations.loading)
    await store.discoveryFeed.load(store)
    #expect(store.discoveryFeed.recommendations.value["items"].array.first?["id"].text == "kept")
    await store.shutdown()
}
