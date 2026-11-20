import Foundation

struct CatalogReference: Identifiable { let id: String; let name: String; let detail: String }
extension MusicStore {
    func catalogArtistReferences(_ track: Track) async throws -> [CatalogReference] {
        let links = try await api("/library/links", method: "POST", body: .object(["resolve_credits": .bool(true), "track_id": .string(track.id), "artist": .string(track.artist), "album": .string(track.album), "source": .string(track.source ?? ""), "provider_data": track.provider_data ?? .object([:])]))
        return links["artists"].array.compactMap { value in guard let id = value["id"].string else { return nil }; return CatalogReference(id: id, name: value["name"].text, detail: track.source ?? "") }
    }
    func catalogReferences(_ track: Track, album: Bool, artistIndex: Int) async throws -> [CatalogReference] {
        let links = try await api("/library/links", method: "POST", body: .object([
            "resolve_credits": .bool(!album), "track_id": .string(track.id), "artist": .string(track.artist), "album": .string(track.album),
            "album_artist": .string(track.album_artist ?? track.artist), "source": .string(track.source ?? ""),
            "provider_data": track.provider_data ?? .object([:])
        ]))
        if !links["resolved"].boolean {
            let name = album ? track.album : track.creditNames[artistIndex]
            let page = try await api(apiQuery("/library/\(album ? "albums" : "artists")", ["q": name, "limit": "200"]))
            return page["items"].array.filter { item in
                album ? item["title"].text == name && (track.album_artist == nil || item["artist"].string == track.album_artist) : item["name"].text == name
            }.map { CatalogReference(id: $0["id"].text, name: $0[album ? "title" : "name"].text, detail: $0["artist"].string ?? $0["source"].string ?? String($0["id"].text.prefix(8))) }
        }
        let credits = links["artists"].array
        let entries = album ? [links["album"]] : track.creditNames.count == 1 && credits.count > 1 ? credits : credits.filter { $0["name"].text == track.creditNames[artistIndex] }
        return entries.compactMap { value in guard let id = value["id"].string else { return nil }; return CatalogReference(id: id, name: value[album ? "title" : "name"].text, detail: track.source ?? "") }
    }
    func openCatalogReference(_ reference: CatalogReference, track: Track, album: Bool) async {
        let owner = sessionID
        await perform {
            if album { albumID = reference.id; albumName = reference.name; artistID = nil; artistName = nil }
            else { artistID = reference.id; artistName = reference.name; albumID = nil; albumName = nil }
            page = 0; catalogAnchor = track; libraryQueryOverride = ""
            detailTrack = nil; showLibrary = true
            try await loadLibrary()
            guard sessionID == owner else { return }
            detailTrack = nil; showLibrary = true
        }
    }
}
