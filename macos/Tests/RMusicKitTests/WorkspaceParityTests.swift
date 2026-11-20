import Foundation
import Testing
@testable import RMusicKit

private func json(_ text: String) throws -> JSONValue { try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8)) }
@Test func cloudPlaylistCoverDoesNotDependOnImportedTrackIDs() throws {
    let value = try json(#"{"id":"cloud-list","source":"qqmusic","name":"Cloud","cover_url":"https://y.gtimg.cn/cover.jpg","track_count":842}"#)
    let entry = PlaylistEntry(value: value, cloud: true)
    #expect(entry.artwork?.cover_url == "https://y.gtimg.cn/cover.jpg")
    #expect(entry.artwork?.id.hasPrefix("playlist:") == true)
    #expect(PlaylistEntry(value: try json(#"{"id":"empty","cover_url":null}"#), cloud: true).artwork == nil)
}
@Test @MainActor func outputLabelsFollowHostAndOutputRenamesAndReleaseOldInventory() throws {
    let labels = OutputLabels()
    let zone = try json(#"{"id":"z","name":"Old zone","endpoint_id":"e","status":"paused","position":0,"volume":1,"muted":false,"queue_index":0,"queue":[]}"#).decode(Zone.self)
    labels.accept([try json(#"{"name":"Living room Mac","outputs":[{"id":"e","name":"USB DAC"}]}"#)])
    #expect(labels.label(zone) == "Living room Mac · USB DAC")
    labels.accept([try json(#"{"name":"Desk Mac","outputs":[{"id":"e","name":"Headphones"}]}"#)])
    #expect(labels.label(zone) == "Desk Mac · Headphones")
    labels.reset(); #expect(labels.label(zone) == "Old zone")
}
@Test func signalTimelineRejectsOldTracksAndPlaybackInstances() throws {
    let zone = try json(#"{"id":"z","name":"BlackHole","endpoint_id":"e","status":"playing","track":{"id":"current","title":"Test","artist":"Artist","album":"Album","duration":30,"format":"flac","sample_rate":96000,"bit_depth":24},"position":1,"volume":1,"muted":false,"playback_id":"instance","queue_index":0,"queue":[]}"#).decode(Zone.self)
    let current = try json(#"{"name":"BlackHole","playback":{"track_id":"current","core_playback_id":"instance","signal_path":{"sample_rate":96000,"bit_depth":24,"channels":2,"exclusive":false,"processing":["CoreAudio PCM decode"]}}}"#)
    #expect(SignalPresentation(zone: zone, endpoint: current, info: .null).hasReport)
    let old = try json(#"{"playback":{"track_id":"old","core_playback_id":"instance","signal_path":{"sample_rate":48000}}}"#)
    #expect(!SignalPresentation(zone: zone, endpoint: old, info: .null).hasReport)
    let stale = try json(#"{"playback":{"track_id":"current","core_playback_id":"previous","signal_path":{"sample_rate":48000}}}"#)
    #expect(!SignalPresentation(zone: zone, endpoint: stale, info: .null).hasReport)
    let wrongInfo = try json(#"{"track_id":"old","format":"mp3","verified":true,"sample_rate":44100,"bit_depth":16}"#)
    let presentation = SignalPresentation(zone: zone, endpoint: current, info: wrongInfo)
    #expect(presentation.stages[0].detail.contains("96"))
    #expect(presentation.stages[0].status != "已校验")
    #expect(presentation.stages.last?.status == "系统混音")
}
@Test @MainActor func cloudDirectoryDoesNotHideDifferentAccountsOrCollectedLists() throws {
    let local = try json(#"[{"id":"local","sync":{"provider":"qqmusic","playlist_id":"same","account_context":"a"}}]"#).array
    let cloud = try json(#"[{"id":"same","source":"qqmusic","account_context":"a"},{"id":"same","source":"qqmusic","account_context":"b"},{"id":"collected","source":"qqmusic","account_context":"a","kind":"collected"}]"#).array
    let all = PlaylistCatalog.entries(local: local, cloud: cloud, source: "all")
    #expect(all.count == 3)
    #expect(all.filter(\.cloud).count == 2)
    #expect(all.contains { $0.value["id"].text == "collected" })
    #expect(PlaylistCatalog.entries(local: local, cloud: cloud, source: "local").count == 1)
    #expect(PlaylistCatalog.entries(local: local, cloud: cloud, source: "other").isEmpty)
}
@Test func cloudPagingContinuesAndRejectsInvalidCursors() throws {
    #expect(try ProviderPagination(page: 1, response: json(#"{"next_page":2}"#)).next == 2)
    #expect(try ProviderPagination(page: 2, response: json(#"{"next_page":null}"#)).next == nil)
    #expect(throws: RuntimeFailure.self) { try ProviderPagination(page: 1, response: json(#"{"next_page":1}"#)) }
    #expect(throws: RuntimeFailure.self) { try ProviderPagination(page: 1, response: json(#"{"next_page":2.5}"#)) }
}
@Test func firstListeningHistoryRequestOmitsInvalidEmptyCursor() {
    #expect(!statsHistoryPath(cursor: "").contains("cursor="))
    #expect(!statsHistoryPath(cursor: nil).contains("cursor="))
    #expect(statsHistoryPath(cursor: "100:session").contains("cursor="))
}

@MainActor private final class PagedProviderRuntime: RuntimeTransport {
    var invalidTotal = false
    var paths: [String] = []
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        let path = body["path"].text; paths.append(path)
        if path == "/playlists" || path == "/playlist-directory" || path == "/plugins" { return .object(["items": .array([])]) }
        let page = URLComponents(string: path)?.queryItems?.first { $0.name == "page" }?.value.flatMap(Int.init) ?? 1
        let first = (page - 1) * 100, last = min(page * 100, 205)
        return .object(["playlist_id": .string("long"), "total": .number(invalidTotal && page == 2 ? 204 : 205), "next_page": page < 3 ? .number(Double(page + 1)) : .null,
                        "items": .array((first..<last).map { .object(["id": .string("example:\($0)")]) })])
    }
    func shutdown() async {}
}
@Test @MainActor func entireCloudPlaylistUsesAllPagesAndNeverCommitsPartialResults() async throws {
    let runtime = PagedProviderRuntime()
    let actual = MusicStore(runtime: runtime)
    let ids = try await collectProviderPlaylistIDs("example", playlist: "long", store: actual) { _, _ in }
    #expect(ids.count == 205)
    #expect(ids.last?.text == "example:204")
    #expect(runtime.paths.count == 3)
    runtime.paths = []; runtime.invalidTotal = true
    await #expect(throws: RuntimeFailure.self) { try await collectProviderPlaylistIDs("example", playlist: "long", store: actual) { _, _ in } }
    #expect(runtime.paths.count == 2)
    #expect(runtime.paths.allSatisfy { !$0.contains("/queue") })
}
@Test @MainActor func playlistCatalogDoesNotRetainItsWindowStore() async {
    let runtime = PagedProviderRuntime()
    var window: MusicStore? = MusicStore(runtime: runtime)
    weak let released = window
    let catalog = PlaylistCatalog()
    await catalog.load(window!)
    window = nil
    #expect(released == nil)
}

@Test @MainActor func remoteVolumeUsesExplicitConfigurationAndNeverAnotherEndpointsReport() throws {
    let store = MusicStore(runtime: PagedProviderRuntime())
    let zone = try json(#"{"id":"zone","name":"Remote","endpoint_id":"remote","status":"paused","track":null,"position":0,"volume":0.3,"muted":false,"queue_index":0,"queue":[]}"#).decode(Zone.self)
    store.zones = [zone]; store.selectedZoneID = zone.id
    #expect(store.volumeMode == .fixed)
    store.signalEndpoint = try json(#"{"id":"remote","output_config":{"volume_control":"software"}}"#)
    #expect(store.volumeMode == .software)
    #expect(store.volumeValue == 0.3)
    store.signalEndpoint = try json(#"{"id":"another","output_config":{"volume_control":"software"}}"#)
    #expect(store.volumeMode == .fixed)
}

@Test func coreConversionIsShownOnlyFromThisPlaybackReport() throws {
    let zone = try json(#"{"id":"z","name":"BlackHole","endpoint_id":"e","status":"playing","track":{"id":"current","title":"Test","artist":"Artist","album":"Album","duration":30,"format":"flac","sample_rate":96000,"bit_depth":24},"position":1,"volume":1,"muted":false,"playback_id":"instance","queue_index":0,"queue":[]}"#).decode(Zone.self)
    let endpoint = try json(#"{"playback":{"track_id":"current","core_playback_id":"instance","signal_path":{"sample_rate":48000,"bit_depth":24,"channels":2,"exclusive":false,"processing":["Core PCM conversion: 48000 Hz / 24 bit / 2 ch","HAL driver: float 32 bit"]}}}"#)
    let presentation = SignalPresentation(zone: zone, endpoint: endpoint, info: .null)
    #expect(presentation.stages.first { $0.name == "传输" }?.status == "Core 转换")
    #expect(!presentation.stages.contains { $0.name == "DSP" || $0.name == "淡入淡出" })
    #expect(presentation.stages.last?.detail.contains("48") == true)
}

@Test func signalPathShowsOnlyEffectiveSettingsAndReportsLoudnessByProcessor() throws {
    var zone = try json(#"{"id":"z","name":"USB","endpoint_id":"e","status":"playing","track":{"id":"t","title":"Test","artist":"","album":"","duration":30,"format":"flac","sample_rate":48000,"bit_depth":24},"position":1,"volume":1,"muted":false,"playback_id":"p","queue_index":0,"queue":[],"dsp":{"enabled":true,"normalize":true,"target_lufs":-14,"preamp_db":0,"bands":[]},"transitions":{"fade_in_ms":80,"fade_out_ms":120}}"#).decode(Zone.self)
    func report(_ stages: [String]) -> SignalPresentation {
        let signal: JSONValue = .object(["sample_rate": .number(48000), "bit_depth": .number(24),
            "channels": .number(2), "processing": .array(stages.map(JSONValue.string))])
        return SignalPresentation(zone: zone, endpoint: .object(["playback": .object(["track_id": .string("t"),
            "core_playback_id": .string("p"), "signal_path": signal])]), info: .null)
    }
    let core = report(["Core DSP · 按音区设置处理", "Normalization pending measured LUFS"])
    #expect(!core.stages.contains { ["DSP", "参数均衡", "增益与余量"].contains($0.name) })
    #expect(core.stages.first { $0.name == "响度均衡" }?.status == "待分析")
    #expect(core.stages.first { $0.name == "淡入淡出" }?.detail.hasPrefix("Core") == true)
    #expect(report(["Core DSP · 按音区设置处理", "Core loudness applied"]).stages.first { $0.name == "响度均衡" }?.status == "已应用")
    zone.track?.integrated_lufs = -19
    zone.id = "travel-e"
    let travel = report(["Shared DSP f64", "Shared fade envelope"])
    #expect(travel.stages.first { $0.name == "响度均衡" }?.detail.hasPrefix("播放设备") == true)
    #expect(travel.stages.first { $0.name == "淡入淡出" }?.status == "已应用")
}
