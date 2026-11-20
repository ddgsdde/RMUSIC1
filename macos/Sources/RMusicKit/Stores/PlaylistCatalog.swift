import Foundation
import Observation

struct PlaylistEntry: Identifiable {
    let value: JSONValue
    let cloud: Bool
    var id: String { "\(cloud):\(value["source"].text):\(value["account_context"].text):\(value["id"].text)" }
    var provider: String { cloud ? value["source"].text : value["sync"]["provider"].text }
    var artwork: Track? {
        guard let source = value["cover_url"].string, !source.isEmpty else { return nil }
        // A cloud directory cover is independent of locally imported track IDs.
        return Track(id: "playlist:\(id)", title: value["name"].text, artist: "", album: "", duration: 0, format: "", sample_rate: 0, bit_depth: 0, cover_url: source)
    }
}

@MainActor @Observable
final class PlaylistCatalog {
    var local: [JSONValue] = []
    var cloud: [JSONValue] = []
    var providers: [JSONValue] = []
    var loading = false
    var error: String?
    @ObservationIgnored private var revision = UUID()
    @ObservationIgnored private var owner: UUID?
    func load(_ store: MusicStore) async {
        let session = store.sessionID, request = UUID(); revision = request
        if owner != session { local = []; cloud = []; providers = []; error = nil; owner = session }
        loading = true
        defer { if revision == request { loading = false } }
        do {
            let lists = try await store.api("/playlists")
            let directory: JSONValue = store.user["role"].text == "external" ? .object(["items": .array([])]) : try await store.api("/playlist-directory")
            let plugins = try await store.api("/plugins")
            try Task.checkCancellation()
            guard session == store.sessionID, revision == request else { return }
            local = lists["items"].array; cloud = directory["items"].array
            providers = store.user["role"].text == "external" ? [] : plugins["items"].array.filter { $0["enabled"].boolean && $0["capabilities"].array.contains { ["playlists", "favorites"].contains($0.text) } }
            error = nil
        } catch is CancellationError {} catch {
            guard session == store.sessionID, revision == request else { return }
            self.error = error.localizedDescription
            await store.handle(error)
        }
    }
    func name(_ id: String) -> String { providers.first { $0["id"].text == id }?["name"].text ?? (id.isEmpty ? "本地歌单" : id) }
    func entries(source: String) -> [PlaylistEntry] {
        Self.entries(local: local, cloud: cloud, source: source)
    }
    static func entries(local: [JSONValue], cloud: [JSONValue], source: String) -> [PlaylistEntry] {
        let own = local.filter { source == "all" || source == "local" || $0["sync"]["provider"].text == source }
        let remote = cloud.filter { item in
            (source == "all" || source == item["source"].text) && !local.contains { bound in
                bound["sync"]["provider"].text == item["source"].text && bound["sync"]["playlist_id"].text == item["id"].text && bound["sync"]["account_context"].text == item["account_context"].text
            }
        }
        return own.map { PlaylistEntry(value: $0, cloud: false) } + remote.map { PlaylistEntry(value: $0, cloud: true) }
    }
}

extension MusicStore {
    func playPlaylist(_ id: String) async {
        await perform {
            let session = sessionID
            guard let zone else { throw RuntimeFailure(message: "先选择播放音区。") }
            try await requireUngrouped(zone.id)
            let playlist = try await api("/playlists/\(id.pathComponentEncoded)")
            try Task.checkCancellation()
            guard session == sessionID, zone.id == selectedZoneID else { return }
            let ids = playlist["track_ids"].array
            guard !ids.isEmpty else { throw RuntimeFailure(message: "歌单中还没有歌曲。") }
            guard ids.count <= 10_000 else { throw RuntimeFailure(message: "播放列表不能超过一万首。") }
            _ = try await api("/zones/\(zone.id.pathComponentEncoded)/queue", method: "PUT", body: .object(["track_ids": .array(ids)]))
            try Task.checkCancellation()
            guard session == sessionID else { return }
            _ = try await api("/zones/\(zone.id.pathComponentEncoded)/play", method: "POST", body: .object(["queue_index": .number(0)]))
            try await refreshZones()
        }
    }
}
