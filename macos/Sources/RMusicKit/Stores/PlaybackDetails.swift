import Foundation
import Observation

@MainActor @Observable
final class PlaybackDetails {
    var lines: [LyricLine] = []
    var document: JSONValue = .null
    var documentID = UUID()
    @ObservationIgnored private var rendering: Task<Void, Never>?
    var lyricError: String?
    var loading = false
    var trackID: String?
    var outputError: String?
    var recovery: PlaybackRecovery?
    @ObservationIgnored private var task: Task<Void, Never>?
    @ObservationIgnored private var cachedDocuments: [String: JSONValue] = [:]
    @ObservationIgnored private var pendingDocuments: [String: Task<JSONValue, Error>] = [:]
    @ObservationIgnored private var cacheOwner: UUID?
    @ObservationIgnored private var prefetchTask: Task<Void, Never>?
    @ObservationIgnored private var nextID: String?
    private func lyricDocument(_ track: Track, store: MusicStore) async throws -> JSONValue {
        let owner = store.sessionID
        if cacheOwner != owner { pendingDocuments.values.forEach { $0.cancel() }; pendingDocuments = [:]; cachedDocuments = [:]; cacheOwner = owner }
        if let value = cachedDocuments[track.id] { return value }
        if let pending = pendingDocuments[track.id] { return try await pending.value }
        let pending = Task { try await store.api("/library/tracks/\(track.id.pathComponentEncoded)/lyrics") }
        pendingDocuments[track.id] = pending
        do {
            let value = try await pending.value
            guard cacheOwner == owner, store.sessionID == owner else { throw CancellationError() }
            pendingDocuments.removeValue(forKey: track.id); cachedDocuments[track.id] = value
            if cachedDocuments.count > 32, let old = cachedDocuments.keys.first(where: { $0 != track.id && $0 != trackID }) { cachedDocuments.removeValue(forKey: old) }
            return value
        } catch { if cacheOwner == owner { pendingDocuments.removeValue(forKey: track.id) }; throw error }
    }
    func prefetch(_ zone: Zone?, store: MusicStore) {
        let next = zone.flatMap { zone -> Track? in
            guard zone.shuffle != true, zone.repeat != "one", zone.queue.indices.contains(zone.queue_index + 1) else { return nil }
            return zone.queue[zone.queue_index + 1]
        }
        guard next?.id != nextID else { return }
        prefetchTask?.cancel(); nextID = next?.id
        guard let next, let current = zone?.track else { return }
        prefetchTask = Task { [weak self, weak store] in
            guard let self, let store else { return }
            do { _ = try await lyricDocument(current, store: store); try Task.checkCancellation(); _ = try await lyricDocument(next, store: store) } catch { }
        }
    }
    func reset() { prefetchTask?.cancel(); nextID = nil; pendingDocuments.values.forEach { $0.cancel() }; pendingDocuments = [:]; cachedDocuments = [:]; cacheOwner = nil; task?.cancel(); rendering?.cancel(); document = .null; documentID = UUID(); trackID = nil; lines = []; lyricError = nil; outputError = nil; recovery = nil; loading = false }
    func update(_ track: Track?, store: MusicStore) {
        guard track?.id != trackID else { return }
        task?.cancel(); rendering?.cancel(); document = .null; documentID = UUID(); trackID = track?.id; lines = []; lyricError = nil
        guard let track else { loading = false; return }
        let epoch = store.sessionID
        task = Task { [weak self, weak store] in
            guard let self, let store else { return }
            loading = true
            defer { if trackID == track.id { loading = false } }
            do {
                let document = try await lyricDocument(track, store: store)
                try Task.checkCancellation()
                guard epoch == store.sessionID, trackID == track.id else { return }
                let language = UserDefaults.standard.string(forKey: "lyricLanguage") ?? ""
                let parsed = try await Self.parse(document, language: language)
                try Task.checkCancellation(); guard epoch == store.sessionID, trackID == track.id else { return }
                self.document = document; documentID = UUID(); lines = parsed
            } catch is CancellationError {} catch { if epoch == store.sessionID, trackID == track.id { lyricError = error.localizedDescription } }
        }
    }
    func selectLanguage(_ language: String) {
        rendering?.cancel(); let document = document, owner = documentID
        rendering = Task { [weak self] in
            do { let parsed = try await Self.parse(document, language: language); try Task.checkCancellation(); guard owner == self?.documentID else { return }; self?.lines = parsed }
            catch is CancellationError {} catch { if owner == self?.documentID { self?.lyricError = error.localizedDescription } }
        }
    }
    nonisolated private static func parse(_ document: JSONValue, language: String) async throws -> [LyricLine] {
        let worker = Task.detached(priority: .utility) { try Task.checkCancellation(); return try renderLyricDocument(document, language: language) }
        return try await withTaskCancellationHandler { try await worker.value } onCancel: { worker.cancel() }
    }
    func retry(_ store: MusicStore) { if let trackID { cachedDocuments.removeValue(forKey: trackID) }; trackID = nil; update(store.zone?.track, store: store) }
    func active(at position: Double) -> Int? { lines.last { $0.time >= 0 && $0.time <= position }?.id }
    func refreshStatus(_ store: MusicStore) async throws {
        let epoch = store.sessionID
        let report = try await store.runtimeCall("status")
        guard epoch == store.sessionID else { return }
        let output = report["items"].array.first { $0["id"].string == store.zone?.endpoint_id }
        recovery = PlaybackRecovery.resolve(zone: store.zone, report: output?["report"] ?? .null)
        outputError = recovery?.kind == .buffering ? nil : recovery?.message
    }
}
