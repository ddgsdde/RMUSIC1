import Foundation
import Observation

/// One bounded page per feature, owned by the view. No polling task retains the window.
@MainActor @Observable
final class ResourceStore {
    var value: JSONValue = .null
    var loading = false
    var error: String?
    var status: Int?
    private var revision = UUID()
    private var owner: UUID?
    private var location: String?
    /// Idle views refresh once at the next Core calendar day.
    func followListeningStats(
        _ path: String, store: MusicStore,
        now: () -> Date = { .now },
        sleep: (TimeInterval) async throws -> Void = { try await Task.sleep(for: .seconds($0)) }
    ) async {
        while store.connected, store.coreReachable, store.uiActive, !store.systemSleeping, !Task.isCancelled {
            let requestedAt = now()
            await load(path, store: store)
            guard !Task.isCancelled else { return }
            let dayEnd = listeningStatisticsDayEnd(at: requestedAt, timezone: value["timezone"].string)
            let currentTime = now()
            let playing = store.zones.contains(where: { $0.status == "playing" })
            let refreshAt = playing ? min(dayEnd, currentTime.addingTimeInterval(10)) : dayEnd
            if refreshAt > currentTime {
                do { try await sleep(refreshAt.timeIntervalSince(currentTime)) } catch { return }
            }
            guard !Task.isCancelled else { return }
            if now() >= dayEnd { value = .null }
        }
    }
    @discardableResult
    func load(_ path: String, store: MusicStore, identity: String? = nil, snapshot: String? = nil, reportConnectionFailure: Bool = true) async -> Bool {
        let request = UUID(), session = store.sessionID
        let identity = identity ?? path
        let snapshot = snapshot ?? path
        revision = request
        if owner != session || location != identity { value = store.pageSnapshots.value(snapshot, owner: session) ?? .null; error = nil; status = nil; owner = session; location = identity }
        loading = true
        defer { if revision == request { loading = false } }
        do {
            let next = try await store.api(path)
            try Task.checkCancellation()
            guard revision == request, session == store.sessionID else { return false }
            store.pageSnapshots.save(next, path: snapshot, owner: session)
            value = next; error = nil; status = nil
            return true
        } catch is CancellationError {} catch {
            guard revision == request, session == store.sessionID else { return false }
            self.error = error.localizedDescription
            self.status = (error as? RuntimeFailure)?.status
            if reportConnectionFailure, let status = (error as? RuntimeFailure)?.status, [401, 408, 502, 503, 504].contains(status) { await store.handle(error) }
        }
        return false
    }
}

func listeningStatisticsDayEnd(at date: Date, timezone: String?) -> Date {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = timezone.flatMap(TimeZone.init(identifier:)) ?? TimeZone(identifier: "Asia/Shanghai")!
    return calendar.dateInterval(of: .day, for: date)!.end
}

/// Read-only page snapshots never include credentials or survive an account session.
@MainActor final class PageSnapshots {
    private var owner: UUID?
    private var scrollOffsets: [String: Double] = [:]
    private var pages: [String: (value: JSONValue, bytes: Int, at: Date)] = [:]
    private func valid(_ path: String) -> Bool { ["/library/", "/catalog/", "/recommendations", "/stats", "/playlists", "/devices", "/endpoints", "/platform-recommendations/"].contains { path.hasPrefix($0) } }
    private func prepare(_ owner: UUID) { if self.owner != owner { pages = [:]; scrollOffsets = [:]; self.owner = owner } }
    func scrollOffset(_ key: String, owner: UUID) -> Double? { prepare(owner); return scrollOffsets[key] }
    func saveScroll(_ offset: Double, key: String, owner: UUID) {
        prepare(owner); guard offset.isFinite else { return }; scrollOffsets[key] = offset
        if scrollOffsets.count > 64, let old = scrollOffsets.keys.first(where: { $0 != key }) { scrollOffsets.removeValue(forKey: old) }
    }
    func value(_ path: String, owner: UUID) -> JSONValue? {
        prepare(owner); guard valid(path), let page = pages[path] else { return nil }
        pages[path] = (page.value, page.bytes, Date()); return page.value
    }
    func save(_ value: JSONValue, path: String, owner: UUID) {
        prepare(owner); guard valid(path), let data = try? JSONEncoder().encode(value), data.count <= 1024 * 1024 else { return }
        pages[path] = (value, data.count, Date())
        while pages.count > 32 || pages.values.reduce(0, { $0 + $1.bytes }) > 8 * 1024 * 1024 {
            guard let old = pages.min(by: { $0.value.at < $1.value.at })?.key else { break }; pages.removeValue(forKey: old)
        }
    }
}
