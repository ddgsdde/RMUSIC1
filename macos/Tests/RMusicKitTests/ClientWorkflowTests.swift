import Foundation
import Testing
@testable import RMusicKit

@Test func runtimeReaderRetainsFragmentsAndFailsClosedOnMalformedLines() throws {
    let reader = RuntimeResponseReader()
    #expect(try reader.read(Data(#"{"id":"one","result": {"x":"# .utf8)).get().isEmpty)
    let values = try reader.read(Data("1}}\n{\"id\":\"two\",\"result\":true}\n".utf8)).get()
    #expect(values.map(\.id) == ["one", "two"])
    #expect(values[0].result?["x"].integer == 1)
    #expect(throws: RuntimeFailure.self) { try reader.read(Data("invalid\n".utf8)).get() }
    #expect(try reader.read(Data("{\"id\":\"three\"}\n".utf8)).get().isEmpty)
}
@Test func smartRulesRetainIdentityBoundsAndRejectInvertedOrFractionalNumbers() throws {
    var draft = SmartPlaylistDraft(.object(["album_id": .string("verified-album"), "artist_id": .string("verified-artist"), "max_plays": .number(0), "favorites_only": .bool(true)]))
    let result = try draft.rule()
    #expect(result["album_id"].text == "verified-album")
    #expect(result["max_plays"].integer == 0)
    #expect(result["favorites_only"].boolean)
    draft.strings["min_plays"] = "1"
    #expect(throws: RuntimeFailure.self) { try draft.rule() }
    draft.strings["min_plays"] = "0.5"
    #expect(throws: RuntimeFailure.self) { try draft.rule() }
    draft.strings["min_plays"] = "0"; draft.strings["max_results"] = "10001"
    #expect(throws: RuntimeFailure.self) { try draft.rule() }
}

@MainActor private final class CancellableWorkflowRuntime: RuntimeTransport {
    var cancelled = false
    var started = false
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        started = true
        do { try await Task.sleep(for: .seconds(30)); return .object([:]) }
        catch { cancelled = true; throw error }
    }
    func shutdown() async {}
}
@Test @MainActor func navigatingAwayCancelsWritesAndReleasesFeatureState() async throws {
    let runtime = CancellableWorkflowRuntime()
    let own = MusicStore(runtime: runtime), action = WorkflowAction()
    var committed = false
    action.send("/smart-playlists/preview", store: own) { _ in committed = true }
    let started = ContinuousClock.now + .seconds(3)
    while !runtime.started && ContinuousClock.now < started { await Task.yield() }
    action.cancel()
    let cancelled = ContinuousClock.now + .seconds(3)
    while !runtime.cancelled && ContinuousClock.now < cancelled { await Task.yield() }
    #expect(runtime.started && runtime.cancelled)
    #expect(!committed && !action.busy)
}

@MainActor private final class CoreDownloadRuntime: RuntimeTransport {
    var calls: [(String, JSONValue)] = []
    var inconsistent = false
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        calls.append((operation, body))
        if operation == "downloads" { return .object(["items": .array([]), "jobs": .object([:]), "used_bytes": .number(0), "limit_bytes": .number(1024)]) }
        guard operation == "request" else { return .null }
        let offset = URLComponents(string: body["path"].text)?.queryItems?.first { $0.name == "offset" }?.value.flatMap(Int.init) ?? 0
        let tracks = (offset..<min(offset + 100, 205)).map { i in
            JSONValue.object(["id": .string("core-\(i)"), "title": .string("Song"), "artist": .string("Artist"), "album": .string("Album"), "duration": .number(3), "format": .string("flac"), "sample_rate": .number(44100), "bit_depth": .number(16)])
        }
        return .object(["items": .array(tracks), "total": .number(inconsistent && offset > 0 ? 204 : 205)])
    }
    func shutdown() async {}
}

@Test @MainActor func directoryDownloadsKeepCoreIDsAndRejectPartialPlaylistSubmission() async throws {
    let defaults = UserDefaults(suiteName: "rmusic-download-test-\(UUID())")!
    let downloads = DownloadsStore(defaults: defaults), runtime = CoreDownloadRuntime()
    let store = MusicStore(runtime: runtime)
    try await downloads.queue(["core-a", "core-a"], store: store)
    #expect(downloads.pendingIDs == ["core-a"])
    #expect(store.showDownloads && runtime.calls.isEmpty)
    await #expect(throws: RuntimeFailure.self) { try await downloads.queue((0..<1000).map { "track-\($0)" }, store: store) }
    #expect(downloads.pendingIDs == ["core-a"])
    defaults.set("/tmp/rmusic-chosen-directory", forKey: "downloadDirectory")
    try await downloads.queuePlaylist("verified-core-playlist", store: store)
    let submission = try #require(runtime.calls.first { $0.0 == "download_files" })
    #expect(submission.1["directory"].text == "/tmp/rmusic-chosen-directory")
    #expect(Set(submission.1["track_ids"].array.compactMap(\.string)).count == 205)
    #expect(runtime.calls.filter { $0.0 == "request" }.count == 3)
    runtime.calls = []; runtime.inconsistent = true
    await #expect(throws: RuntimeFailure.self) { try await downloads.queuePlaylist("verified-core-playlist", store: store) }
    #expect(runtime.calls.allSatisfy { $0.0 == "request" })
    #expect(runtime.calls.count == 2)
    downloads.reset()
    #expect(downloads.pendingIDs.isEmpty)
    if case .null = downloads.fileDownloads {} else { Issue.record("旧账号的下载记录未释放") }
}

@Test @MainActor func nativeInstallerWorkflowCancellationReleasesOperationAndStore() async throws {
    let runtime = CancellableWorkflowRuntime(), action = WorkflowAction()
    var store: MusicStore? = MusicStore(runtime: runtime)
    weak var released = store
    var cleanup = false, committed = false
    action.run(store: store!, operation: { [owner = store!] in
        defer { cleanup = true }
        return try await owner.runtimeCall("install_plugin")
    }) { _ in committed = true }
    let started = ContinuousClock.now + .seconds(3)
    while !runtime.started && ContinuousClock.now < started { await Task.yield() }
    action.cancel(); store = nil
    let deadline = ContinuousClock.now + .seconds(3)
    while (!cleanup || released != nil) && ContinuousClock.now < deadline { await Task.yield() }
    #expect(runtime.started && runtime.cancelled && cleanup)
    #expect(released == nil && !committed && !action.busy)
}

@Test @MainActor func changingSessionDuringPreflightDoesNotWriteToNewCoreOrLeaveActionBusy() async throws {
    let runtime = CoreDownloadRuntime(), store = MusicStore(runtime: runtime), action = WorkflowAction()
    var ran = false, committed = false
    action.run(store: store, preflight: { await store.disconnect() }, operation: { ran = true; return .null }) { _ in committed = true }
    let deadline = ContinuousClock.now + .seconds(3)
    while action.busy && ContinuousClock.now < deadline { await Task.yield() }
    #expect(!ran && !committed)
    #expect(!action.busy)
    action.cancel()
}
