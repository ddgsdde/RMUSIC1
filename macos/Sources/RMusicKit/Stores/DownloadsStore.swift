import Foundation
import Observation

@MainActor @Observable
final class DownloadsStore {
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private var revision = UUID()
    @ObservationIgnored private var lastRefresh: ContinuousClock.Instant?
    init(defaults: UserDefaults = .standard) { self.defaults = defaults }
    var entries: [CachedTrack] = []
    var fileOffset = 0
    var cacheOffset = 0
    var cacheTotal = 0
    var fileDownloads: JSONValue = .null
    var pendingIDs: Set<String> = []
    var directory: String { defaults.string(forKey: "downloadDirectory") ?? "" }
    var jobs: [String: DownloadJob] = [:]
    var used: UInt64 = 0
    var manualBytes: UInt64 = 0
    var limit: UInt64 = 5 * 1024 * 1024 * 1024
    func reset() { lastRefresh = nil; revision = UUID(); cacheOffset = 0; cacheTotal = 0; fileOffset = 0; fileDownloads = .null; pendingIDs = []; entries = []; jobs = [:]; used = 0; manualBytes = 0 }
    func refreshDuringPlaybackPoll(_ store: MusicStore) async throws {
        let hasActiveJob = jobs.values.contains { $0.state == "queued" || $0.state == "downloading" }
        if !hasActiveJob, fileDownloads["active"].integer == 0, let lastRefresh,
           lastRefresh.duration(to: .now) < .seconds(30) { return }
        try await refresh(store)
    }
    func refresh(_ store: MusicStore) async throws {
        let epoch = store.sessionID, request = UUID(); revision = request
        let result = try await store.runtimeCall("downloads", .object(["export_offset": .number(Double(fileOffset)), "cache_offset": .number(Double(cacheOffset))])).decode(CachePage.self)
        try Task.checkCancellation()
        guard store.connected, epoch == store.sessionID, revision == request else { return }
        lastRefresh = .now
        fileDownloads = result.exports ?? .null; cacheTotal = result.total ?? result.items.count
        manualBytes = result.manual_bytes ?? 0
        entries = result.items.sorted { ($0.title ?? $0.track_id) < ($1.title ?? $1.track_id) }; jobs = result.jobs; used = result.used_bytes; limit = result.limit_bytes
    }
    func start(_ id: String, store: MusicStore) async throws {
        _ = try await store.runtimeCall("download", .object(["track_id": .string(id)])); try await refresh(store)
    }
    func queue(_ ids: [String], store: MusicStore) async throws {
        let ids = Set(ids)
        guard !ids.isEmpty, ids.count <= 1000 else { throw RuntimeFailure(message: "每批请选择 1–1000 首歌曲。") }
        if directory.isEmpty {
            guard pendingIDs.union(ids).count <= 1000 else { throw RuntimeFailure(message: "待下载歌曲超过每批 1000 首，请先完成当前选择。") }
            pendingIDs.formUnion(ids); store.showDownloads = true; return
        }
        let epoch = store.sessionID
        _ = try await store.runtimeCall("download_files", .object(["track_ids": .array(ids.sorted().map(JSONValue.string)), "directory": .string(directory)]))
        guard epoch == store.sessionID else { throw CancellationError() }
        pendingIDs.subtract(ids); try await refresh(store); store.notice = "已加入原文件下载队列。"
    }
    func changeFile(_ id: String, resume: Bool, directory: String? = nil, store: MusicStore) async throws {
        var body: [String: JSONValue] = ["id": .string(id), "resume": .bool(resume)]
        if let directory { body["directory"] = .string(directory) }
        _ = try await store.runtimeCall("change_file_download", .object(body)); try await refresh(store)
    }
    func queuePlaylist(_ id: String, store: MusicStore) async throws {
        let owner = store.sessionID
        var offset = 0, ids: [String] = [], expected: Int?
        while true {
            try Task.checkCancellation()
            let page = try await store.api("/playlists/\(id.pathComponentEncoded)/tracks?limit=100&offset=\(offset)").decode(TrackPage.self)
            guard owner == store.sessionID else { throw CancellationError() }
            guard page.total <= 1000, expected == nil || expected == page.total else { throw RuntimeFailure(message: "歌单超过每批 1000 首，或读取期间发生变化，请分批选择。") }
            expected = page.total; ids.append(contentsOf: page.items.map(\.id)); offset += page.items.count
            if offset >= page.total { break }
            guard !page.items.isEmpty else { throw RuntimeFailure(message: "歌单分页不完整，未提交下载。") }
        }
        guard !ids.isEmpty else { throw RuntimeFailure(message: "歌单暂无可下载歌曲。") }
        try await queue(ids, store: store)
    }
    func cancel(_ id: String, store: MusicStore) async throws {
        _ = try await store.runtimeCall("cancel_download", .object(["track_id": .string(id)])); try await refresh(store)
    }
    func remove(_ entry: CachedTrack, store: MusicStore) async throws {
        _ = try await store.runtimeCall("remove_download", .object(["id": .string(entry.id)])); try await refresh(store)
    }
    func setLimit(_ bytes: UInt64, store: MusicStore) async throws {
        _ = try await store.runtimeCall("cache_limit", .object(["bytes": .number(Double(bytes))])); try await refresh(store)
    }
    func state(_ id: String) -> String {
        if let job = jobs[id], job.state == "downloading" || job.state == "queued" { return job.state == "queued" ? "等待下载" : "正在下载" }
        if entries.contains(where: { $0.track_id == id && $0.complete }) { return "已缓存" }
        if jobs[id]?.state == "paused" || entries.contains(where: { $0.track_id == id }) { return "继续下载" }
        return "下载原文件"
    }
}
