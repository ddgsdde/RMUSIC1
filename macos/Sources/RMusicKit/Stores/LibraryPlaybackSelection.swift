import Foundation

/// Collect IDs only; a failed, cancelled or changing directory never replaces the queue.
@MainActor func collectLibraryPlaybackIDs(_ path: String, store: MusicStore) async throws -> [String] {
    let owner = store.sessionID
    guard var url = URLComponents(string: path) else { throw RuntimeFailure(message: "曲库地址无效。") }
    let filters = (url.queryItems ?? []).filter { !["offset", "limit"].contains($0.name) }
    var ids: [String] = [], seen = Set<String>(), expected: Int?
    while true {
        try Task.checkCancellation()
        guard owner == store.sessionID else { throw CancellationError() }
        url.queryItems = filters + [URLQueryItem(name: "limit", value: "200"), URLQueryItem(name: "offset", value: String(ids.count))]
        let page = try await store.api(url.string ?? path).decode(TrackPage.self)
        guard owner == store.sessionID else { throw CancellationError() }
        guard (0...10_000).contains(page.total), expected == nil || expected == page.total,
              ids.count + page.items.count <= page.total,
              page.items.allSatisfy({ !$0.id.isEmpty && seen.insert($0.id).inserted }) else {
            throw RuntimeFailure(message: "曲库在读取期间发生变化，或超过一万首，请缩小筛选后重试。播放队列未改变。")
        }
        expected = page.total
        ids.append(contentsOf: page.items.map(\.id))
        if ids.count == page.total { return ids }
        guard !page.items.isEmpty else { throw RuntimeFailure(message: "曲库分页不完整，播放队列未改变。") }
    }
}

extension MusicStore {
    func playLibraryQuery(_ path: String) async throws -> JSONValue {
        let owner = sessionID
        guard let zone else { throw RuntimeFailure(message: "先启用并选择声音设备。") }
        let ids = try await collectLibraryPlaybackIDs(path, store: self)
        try Task.checkCancellation()
        guard owner == sessionID, self.zone?.id == zone.id else { throw CancellationError() }
        guard !ids.isEmpty else { throw RuntimeFailure(message: "当前筛选没有可播放的歌曲。") }
        try await requireUngrouped(zone.id)
        _ = try await api("/zones/\(zone.id.pathComponentEncoded)/queue", method: "PUT", body: .object(["track_ids": .array(ids.map(JSONValue.string))]))
        _ = try await api("/zones/\(zone.id.pathComponentEncoded)/play", method: "POST", body: .object(["queue_index": .number(0)]))
        try await refreshZones()
        return .null
    }
}
