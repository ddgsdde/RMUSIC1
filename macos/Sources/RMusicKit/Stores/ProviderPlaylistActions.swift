import Foundation

@MainActor
func collectProviderPlaylistIDs(_ provider: String, playlist: String, store: MusicStore, progress: (Int, Int) -> Void) async throws -> [JSONValue] {
    let session = store.sessionID
    var ids: [JSONValue] = [], page = 1, expected: Int?
    while true {
        try Task.checkCancellation()
        guard session == store.sessionID else { throw CancellationError() }
        let result = try await store.api("/providers/\(provider.pathComponentEncoded)/playlists/\(playlist.pathComponentEncoded)/tracks?page=\(page)")
        try Task.checkCancellation()
        guard session == store.sessionID else { throw CancellationError() }
        guard result["playlist_id"].string == playlist, case .number(let total) = result["total"], total >= 0, total <= 10_000, total.rounded() == total else { throw RuntimeFailure(message: "歌单信息不完整或超过一万首，未改变播放队列。") }
        if let expected, expected != Int(total) { throw RuntimeFailure(message: "读取期间云端歌单发生变化，请刷新后重试。") }
        let declared = Int(total)
        expected = declared
        guard case .array(let items) = result["items"] else { throw RuntimeFailure(message: "音源未提供曲目分页。") }
        guard items.allSatisfy({ $0["id"].string?.isEmpty == false }), ids.count + items.count <= declared else { throw RuntimeFailure(message: "歌单曲目分页无效，未改变播放队列。") }
        ids.append(contentsOf: items.map { $0["id"] }); progress(ids.count, declared)
        let cursor = try ProviderPagination(page: page, response: result)
        if let next = cursor.next {
            guard !items.isEmpty, page < 100, ids.count < declared else { throw RuntimeFailure(message: "音源分页没有完整结束，未改变播放队列。") }; page = next
        } else {
            guard ids.count == declared else { throw RuntimeFailure(message: "音源返回的歌曲数量不完整，未改变播放队列。") }
            return ids
        }
    }
}
