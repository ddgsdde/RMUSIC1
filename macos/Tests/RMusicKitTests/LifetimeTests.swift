import Foundation
import Testing
@testable import RMusicKit

@MainActor private final class IdleRuntime: RuntimeTransport {
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        if operation == "devices" { return .array([]) }
        return .object(["items": .array([])])
    }
    func shutdown() async {}
}

@Test @MainActor func discoveryDoesNotKeepAnAbandonedWindowStoreAlive() async {
    var store: MusicStore? = MusicStore(runtime: IdleRuntime())
    weak var released = store
    await store?.prepare()
    await Task.yield()
    store = nil
    await Task.yield()
    #expect(released == nil, "Discovery polling must not own its window store during the idle interval")
    released?.discovery.stop()
}

@MainActor private final class ReconnectingRuntime: RuntimeTransport {
    var downloadReads = 0
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        if operation == "outputs" { return .object(["host_id": .string("mac-test"), "items": .array([])]) }
        if operation == "downloads" { downloadReads += 1; return .object(["items": .array([]), "jobs": .object([:]), "used_bytes": .number(0), "limit_bytes": .number(1024)]) }
        return .object(["items": .array([])])
    }
    func shutdown() async {}
}

@Test @MainActor func unavailableCoreKeepsSessionAndRecoversWithoutHistoricalConnectionError() async throws {
    let store = MusicStore(runtime: ReconnectingRuntime())
    store.connected = true; store.coreReachable = true
    store.selectedZoneID = "preserved-zone"
    await store.handle(RuntimeFailure(message: "Connection refused", status: 503))
    #expect(store.connected)
    #expect(!store.coreReachable)
    #expect(!store.canPlay)
    #expect(store.selectedZoneID == "preserved-zone")
    #expect(store.connectionIssue != nil)
    try await store.pollOnce()
    #expect(store.coreReachable)
    #expect(store.connectionIssue == nil)
    #expect(store.error == nil)
    await store.handle(RuntimeFailure(message: "Expired session", status: 401))
    #expect(!store.connected)
    #expect(!store.coreReachable)
}

@Test @MainActor func idleDownloadPollingSkipsResortingUntilExplicitRefreshOrActiveJob() async throws {
    let runtime = ReconnectingRuntime()
    let store = MusicStore(runtime: runtime)
    store.connected = true
    for _ in 0..<12 { try await store.downloads.refreshDuringPlaybackPoll(store) }
    #expect(runtime.downloadReads == 1)
    try await store.downloads.refresh(store)
    #expect(runtime.downloadReads == 2)
    store.downloads.jobs["fixture"] = DownloadJob(state: "downloading", error: nil, received: 1, total: 2)
    try await store.downloads.refreshDuringPlaybackPoll(store)
    #expect(runtime.downloadReads == 3)
}
