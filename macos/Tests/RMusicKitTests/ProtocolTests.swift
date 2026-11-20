import Foundation
import Testing
@testable import RMusicKit

@Test func wireRoundTripPreservesNestedCorePayload() throws {
    let input = Data(#"{"id":"req","result":{"core_id":"core-a","items":[{"enabled":true,"position":1.25}],"empty":null}}"#.utf8)
    let response = try JSONDecoder().decode(RuntimeResponse.self, from: input)
    #expect(response.id == "req")
    #expect(response.result?["core_id"].string == "core-a")
    #expect(response.result?["items"].array.first?["enabled"].boolean == true)
    let encoded = try JSONEncoder().encode(response.result)
    #expect(try JSONDecoder().decode(JSONValue.self, from: encoded)["core_id"].string == "core-a")
}
@Test func coreProfileKeepsIdentityAcrossLanAndReverseProxy() throws {
    let profile = CoreProfile(address: "http://192.168.1.2:7800", alternatives: [CoreAddress(address: "https://music.example.com/music")], core_id: "same-core")
    let wire = try JSONValue.encoded(profile)
    #expect(wire["core_id"].string == "same-core")
    #expect(wire["alternatives"].array.first?["address"].string == "https://music.example.com/music")
    #expect(wire["password"].string == nil)
}
@Test func formatsUseActualSourceParameters() throws {
    let value = Data(#"{"id":"track","title":"夜航","artist":"测试","album":"验收","duration":6,"format":"flac","sample_rate":96000,"bit_depth":24}"#.utf8)
    let track = try JSONDecoder().decode(Track.self, from: value)
    #expect(track.sample_rate == 96000)
    #expect(track.quality.contains("24 bit"))
    #expect(timeLabel(.nan) == "0:00")
    #expect(timeLabel(61.2) == "1:01")
}

@MainActor private final class ReportRuntime: RuntimeTransport {
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        let path = body["path"].string ?? ""
        let text: String
        if path == "/zones" {
            text = #"{"items":[{"id":"zone","name":"Mac","endpoint_id":"mac-test","status":"playing","track":null,"playback_id":"current","position":1,"volume":1,"muted":false,"queue_index":0,"queue":[]}]}"#
        } else {
            text = #"{"items":[{"id":"mac-test","online":true,"playback":{"core_playback_id":"current","signal_path":{"sample_rate":48000,"bit_depth":24,"channels":2,"exclusive":false,"processing":["CoreAudio shared/system mixing route"]}}}]}"#
        }
        return try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
    }
    func shutdown() async {}
}
@Test @MainActor func actualEndpointReportReplacesPreparingSignalState() async throws {
    let store = MusicStore(runtime: ReportRuntime())
    store.connected = true; store.endpointID = "mac-test"; store.selectedZoneID = "zone"
    try await store.refreshZones()
    #expect(store.signal.contains("48000 Hz"))
    #expect(store.signal.contains("系统共享输出"))
}

@Test func lyricOffsetsDuplicatesAndTranslationsAreAligned() {
    let lines = parseLyrics("[00:01.00]第一行\n[00:02.00][00:03.00]重复段\n[offset:-250]", translation: "[00:00.75]First\n[00:01.76]Repeat")
    #expect(lines.count == 3)
    #expect(lines[0].time == 0.75)
    #expect(lines[0].translation == "First")
    #expect(lines[1].translation == "Repeat")
    #expect(lines[2].time == 2.75)
    let duplicate = parseLyrics("[00:01.00]原文\n[00:01.00]译文")
    #expect(duplicate.count == 1)
    #expect(duplicate[0].translation == "译文")
    #expect(parseLyrics("[00:00.00]歌曲信息\n[00:01.00]正文", translation: "[00:00.00]//\n[00:01.00]Translation")[0].translation == nil)
    #expect(parseLyrics("纯音乐\n没有时间戳").allSatisfy { $0.time == -1 })
}

@MainActor private final class FavoritesRuntime: RuntimeTransport {
    var liked = false
    var requested: [String] = []
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        let path = body["path"].string ?? ""
        requested.append(path)
        if path == "/playlists" { return .object(["items": .array([])]) }
        if path.hasPrefix("/favorites/") { liked = body["json"]["liked"].boolean }
        return .object(["id": .string("favorites-user"), "name": .string("我喜欢"), "track_ids": .array(liked ? [.string("track")] : [])])
    }
    func shutdown() async {}
}
@Test @MainActor func emptyFavoritesDoNotReadNonexistentPlaylistAndWritesUseCore() async throws {
    let runtime = FavoritesRuntime(), store = MusicStore(runtime: FavoritesRuntime())
    let owned = MusicStore(runtime: runtime); owned.connected = true
    try await owned.collection.refresh(owned)
    try await owned.collection.open(owned.collection.favoriteID, store: owned)
    #expect(owned.collection.total == 0)
    #expect(!runtime.requested.contains { $0.contains("/tracks") })
    owned.collection.selectedID = nil
    let track = try JSONDecoder().decode(Track.self, from: Data(#"{"id":"track","title":"测试","artist":"QA","album":"QA","duration":10,"format":"wav","sample_rate":48000,"bit_depth":16}"#.utf8))
    try await owned.collection.like(track, store: owned)
    #expect(runtime.liked)
    #expect(owned.collection.favorites.contains("track"))
    try await owned.collection.like(track, store: owned)
    #expect(!owned.collection.favorites.contains("track"))
    #expect(store.collection.favorites.isEmpty)
}

@Test @MainActor func discoveredCoreSelectionKeepsTheSavedBindingUntilConfirmationAndClearsCredentials() {
    let store = MusicStore(runtime: FavoritesRuntime())
    store.profile = CoreProfile(core_id: "saved")
    store.password = "must-not-follow-another-Core"; store.totp = "123456"
    let core = DiscoveredCore(id: "advertised-core", name: "书房音乐库", addresses: ["http://192.168.1.30:7800/music", "http://10.0.0.30:7800/music"], core_id: nil)
    store.discovery.cores = [core]
    let target = CoreConnectionTarget.choices(profile: CoreProfile(), local: nil, discovered: [core])[0]
    store.selectConnectionTarget(target)
    #expect(store.profile.core_id == "saved" && store.discovery.selectedID == core.id)
    store.adoptConnectionIdentity(CoreIdentity(address: core.addresses[0], core_id: "advertised-core", name: core.name, version: "1", initialized: true))
    #expect(store.connectionBinding.core_id == "advertised-core" && store.profile.core_id == "saved")
    #expect(store.password.isEmpty && store.totp.isEmpty)
    #expect(!store.connected)
}

@MainActor private final class InventoryRuntime: RuntimeTransport {
    var login: JSONValue = .null
    var configured = false
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        if operation == "connect" { login = body; return .object(["core_id": .string("core-inventory"), "address": .string("http://127.0.0.1:7800")]) }
        if operation == "configure_output" { configured = true }
        if operation == "outputs" { return .object(["host_id": .string("mac-test"), "items": .array([])]) }
        if operation == "downloads" { return .object(["items": .array([]), "jobs": .object([:]), "used_bytes": .number(0), "limit_bytes": .number(1024)]) }
        let path = body["path"].string ?? ""
        if path == "/favorites" { return .object(["id": .string("favorites-user"), "name": .string("我喜欢"), "track_ids": .array([])]) }
        return .object(["items": .array([]), "total": .number(0)])
    }
    func shutdown() async {}
}
@Test @MainActor func signingInRegistersInventoryWithoutSelectingOrEnablingAnOutput() async {
    let runtime = InventoryRuntime(), store = MusicStore(runtime: InventoryRuntime())
    let owned = MusicStore(runtime: runtime)
    owned.lanAddress = "http://127.0.0.1:7800"; owned.username = "synthetic"; owned.password = "synthetic-password"
    owned.profile = CoreProfile()
    await owned.connect()
    #expect(owned.connected)
    #expect(runtime.login["output"].string == nil)
    if case .null = runtime.login["output"] {} else { Issue.record("login carries an output selection") }
    #expect(!runtime.configured)
    #expect(owned.outputs.enabled.isEmpty)
    #expect(!owned.canPlay)
    #expect(store.outputs.items.isEmpty)
    await owned.disconnect()
}
