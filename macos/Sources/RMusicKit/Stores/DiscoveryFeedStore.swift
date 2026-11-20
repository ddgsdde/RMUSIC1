import Foundation
import Observation

/// Recommendations stay visible while Core completes cloud checks or model ranking.
@MainActor @Observable
final class DiscoveryFeedStore {
    private(set) var recommendations = ResourceStore()
    private var loadedAt: Date?
    private var owner: UUID?
    private var request = UUID()
    private(set) var updating = false
    private(set) var items: [Track] = []
    struct Shelf: Identifiable {
        let id: String
        let title: String
        let tracks: [Track]
    }
    private(set) var shelves: [Shelf] = []

    func reset() {
        recommendations = ResourceStore()
        request = UUID(); loadedAt = nil; owner = nil; updating = false; items = []; shelves = []
    }
    func load(_ store: MusicStore, refresh: Bool = false) async {
        let session = store.sessionID
        let path = "/recommendations/discover?limit=24"
        if owner != session {
            reset()
            owner = session
            let saved = store.pageSnapshots.value(path, owner: session) ?? .null
            recommendations.value = saved
            present(saved)
        }
        if !refresh, !recommendations.value.object.isEmpty, let loadedAt, Date.now.timeIntervalSince(loadedAt) < 30 { return }
        let id = UUID()
        request = id
        let resource = recommendations
        updating = true
        defer { if request == id { updating = false } }
        let quick = path + "&quick=true" + (refresh ? "&refresh=true" : "")
        guard await resource.load(quick, store: store, identity: path, snapshot: path, reportConnectionFailure: false),
              !Task.isCancelled, request == id, session == store.sessionID else { return }
        present(resource.value)
        if resource.value["refreshing"].boolean {
            guard await resource.load(path, store: store, identity: path, snapshot: path, reportConnectionFailure: false),
                  !Task.isCancelled, request == id, session == store.sessionID else { return }
            present(resource.value)
        }
        loadedAt = .now
    }

    private func present(_ result: JSONValue) {
        var ranked = (try? result["items"].decode([Track].self)) ?? []
        let sections: [Shelf] = result["sections"].array.compactMap { section in
            guard let id = section["id"].string, let tracks = try? section["items"].decode([Track].self), !tracks.isEmpty else { return nil }
            return Shelf(id: id, title: section["title"].text, tracks: tracks)
        }
        var ids = Set(ranked.map(\.id))
        var recordings = Set(ranked.compactMap(\.recording_id).filter { !$0.isEmpty })
        var insertion = 3
        for track in sections.first(where: { $0.id == "cloud" })?.tracks ?? [] {
            guard !ids.contains(track.id) else { continue }
            if let recording = track.recording_id, !recording.isEmpty, !recordings.insert(recording).inserted { continue }
            ids.insert(track.id)
            ranked.insert(track, at: min(insertion, ranked.count))
            insertion += 4
        }
        items = ranked
        shelves = sections.filter { $0.id != "cloud" }
    }
}
