import Foundation
import Observation

@MainActor @Observable
final class CollectionStore {
    var playlists: [Playlist] = []
    var favorites: Set<String> = []
    var favoriteID = ""
    var favoritePlaylist: Playlist?
    var sort = "default"
    var selectedPlaylist: Playlist? { selectedID == favoriteID ? favoritePlaylist : playlists.first { $0.id == selectedID } }
    var selectedID: String?
    var tracks: [Track] = []
    var total = 0
    var page = 0
    var loading = false
    var name = ""
    @ObservationIgnored private var revision = UUID()

    func reset() { revision = UUID(); playlists = []; favorites = []; favoriteID = ""; favoritePlaylist = nil; sort = "default"; selectedID = nil; tracks = []; total = 0; page = 0; name = "" }
    func refresh(_ store: MusicStore) async throws {
        let epoch = store.sessionID
        let lists = try await store.api("/playlists").decode(PlaylistPage.self)
        let liked = try await store.api("/favorites").decode(Playlist.self)
        try Task.checkCancellation()
        guard epoch == store.sessionID, store.connected else { return }
        playlists = lists.items.filter { $0.id != liked.id }; favorites = Set(liked.alias_track_ids ?? liked.track_ids); favoriteID = liked.id; favoritePlaylist = liked
    }
    func synchronize(_ store: MusicStore) async throws {
        let id = selectedID, currentPage = page, previous = selectedPlaylist
        try await refresh(store)
        guard let id, id == selectedID, currentPage == page else { return }
        guard let latest = selectedPlaylist else { selectedID = nil; tracks = []; total = 0; name = ""; return }
        name = id == favoriteID ? "我喜欢" : latest.name
        if previous?.track_ids != latest.track_ids || previous?.smart_query != latest.smart_query {
            try await open(id, store: store, page: currentPage)
        }
    }
    func open(_ id: String, store: MusicStore, page: Int = 0) async throws {
        let current = UUID(), epoch = store.sessionID; revision = current
        if id == favoriteID && selectedID != id { sort = "default" }
        if selectedID != id || self.page != page { tracks = []; total = 0 }
        selectedID = id; self.page = page; loading = true
        defer { if revision == current { loading = false } }
        if id == favoriteID && favorites.isEmpty { tracks = []; total = 0; name = "我喜欢"; return }
        let result = try await store.api("/playlists/\(id.pathComponentEncoded)/tracks?limit=100&offset=\(page * 100)&sort=\(sort.pathComponentEncoded)&view=compact").decode(TrackPage.self)
        guard revision == current, epoch == store.sessionID else { return }
        tracks = result.items; total = result.total
        name = id == favoriteID ? "我喜欢" : playlists.first { $0.id == id }?.name ?? "歌单"
    }
    func like(_ track: Track, store: MusicStore) async throws {
        let epoch = store.sessionID
        let liked = !favorites.contains(track.id)
        let result = try await store.api("/favorites/\(track.id.pathComponentEncoded)", method: "PUT", body: .object(["liked": .bool(liked)])).decode(Playlist.self)
        guard epoch == store.sessionID else { return }; favorites = Set(result.alias_track_ids ?? result.track_ids); favoritePlaylist = result
        if liked { store.similarSongs.find(track, store: store, automatic: true) }
        else { store.similarSongs.unliked(track) }
        if selectedID == favoriteID { try await open(favoriteID, store: store, page: page) }
    }
    func create(_ name: String, store: MusicStore) async throws {
        let clean = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return }
        _ = try await store.api("/playlists", method: "POST", body: .object(["name": .string(clean), "track_ids": .array([])]))
        try await refresh(store)
    }
    func rename(_ playlist: Playlist, name: String, store: MusicStore) async throws {
        _ = try await store.api("/playlists/\(playlist.id.pathComponentEncoded)", method: "PUT", body: .object(["name": .string(name)]))
        try await refresh(store)
        if selectedID == playlist.id { self.name = name }
    }
    func delete(_ playlist: Playlist, store: MusicStore) async throws {
        _ = try await store.api("/playlists/\(playlist.id.pathComponentEncoded)", method: "DELETE")
        if selectedID == playlist.id { selectedID = nil; tracks = [] }
        try await refresh(store)
    }
    func change(_ track: Track, playlist: Playlist, remove: Bool = false, store: MusicStore) async throws {
        _ = try await store.api("/playlists/\(playlist.id.pathComponentEncoded)/tracks", method: "POST", body: .object(["track_id": .string(track.id), "remove": .bool(remove)]))
        try await refresh(store)
        if selectedID == playlist.id { try await open(playlist.id, store: store, page: page) }
    }
}
