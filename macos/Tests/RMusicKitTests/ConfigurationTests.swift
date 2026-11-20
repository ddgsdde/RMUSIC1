import Foundation
import Testing
@testable import RMusicKit

@Test func configurationPatchKeepsAPITypesAndExcludesReadOnlyResources() throws {
    var draft = ConfigurationDraft(values: [
        "managed_memory_bytes": .number(64 * 1_048_576), "background_concurrency": .number(2),
        "playback_cache_mode": .string("full"), "cache_enabled": .bool(true), "resources": .object(["used_bytes": .number(99)]),
        "cache_forced_off": .bool(true), "provider_request_limits": .object(["search_ms": .number(150), "resolve_ms": .number(100)])
    ])
    draft.set("provider_request_limits.search_ms", .number(500))
    let fields = CoreConfiguration.resources.filter { ["playback_cache_mode", "managed_memory_bytes", "background_concurrency"].contains($0.key) }
    let result = try draft.payload(fields)
    #expect(result.object.count == 3)
    #expect(result["playback_cache_mode"].string == "full")
    #expect(!result.object.keys.contains("cache_enabled"))
    #expect(result["managed_memory_bytes"].number == 67_108_864)
    #expect(result["resources"].object.isEmpty)
    #expect(draft.get("provider_request_limits.resolve_ms").number == 100)
    #expect(draft.get("provider_request_limits.search_ms").number == 500)
}
@Test func invalidSettingsCannotReachCore() throws {
    var draft = ConfigurationDraft(values: ["background_concurrency": .number(1.5)])
    let concurrency = CoreConfiguration.resources.filter { $0.key == "background_concurrency" }
    #expect(throws: RuntimeFailure.self) { try draft.payload(concurrency) }
    draft.set("background_concurrency", .number(17))
    #expect(throws: RuntimeFailure.self) { try draft.payload(concurrency) }
    draft.set("catalog_scrape_enabled", .string("false"))
    #expect(throws: RuntimeFailure.self) { try draft.payload([CoreConfiguration.catalogScrape[0]]) }
    draft.set("stream_quality", .string("lossless"))
    #expect(throws: RuntimeFailure.self) { try draft.payload([CoreConfiguration.general[3]]) }
}

@MainActor private final class DelayedPageRuntime: RuntimeTransport {
    var pending: [CheckedContinuation<JSONValue, Never>] = []
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        if body["path"].text == "/slow" { return await withCheckedContinuation { pending.append($0) } }
        return .object(["page": body["path"]])
    }
    func release() { pending.removeFirst().resume(returning: .object(["page": .string("/slow")])) }
    func shutdown() async {}
}
@Test @MainActor func obsoleteFeatureResponseCannotOverwriteNewPageOrSession() async throws {
    let runtime = DelayedPageRuntime(), store = MusicStore(runtime: runtime), resource = ResourceStore()
    func waitForRequest() async throws {
        let deadline = ContinuousClock.now + .seconds(3)
        while runtime.pending.isEmpty, ContinuousClock.now < deadline { await Task.yield() }
        try #require(runtime.pending.count == 1)
    }
    store.connected = true
    let older = Task { await resource.load("/slow", store: store) }
    try await waitForRequest()
    #expect(await resource.load("/new", store: store))
    runtime.release()
    #expect(!(await older.value))
    #expect(resource.value["page"].text == "/new")
    let oldSession = Task { await resource.load("/slow", store: store) }
    try await waitForRequest()
    await store.disconnect()
    runtime.release()
    #expect(!(await oldSession.value))
    #expect(await resource.load("/next-session", store: store))
    #expect(resource.value["page"].text == "/next-session")
}
