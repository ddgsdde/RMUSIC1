import Foundation
import Testing
@testable import RMusicKit

@MainActor private final class RefreshRuntime: RuntimeTransport {
    var fail = false
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        if fail { throw RuntimeFailure(message: "upstream timeout", status: 504) }
        return .object(["text": .string("verified previous page")])
    }
    func shutdown() async {}
}

@Test @MainActor func forcedRefreshFailureRetainsOnlyTheSameSongAndPageWithinTheSameSession() async {
    let runtime = RefreshRuntime(), store = MusicStore(runtime: runtime), resource = ResourceStore()
    await resource.load("/song/a/comments?refresh=false", store: store, identity: "a:comments:page1")
    runtime.fail = true
    await resource.load("/song/a/comments?refresh=true", store: store, identity: "a:comments:page1")
    #expect(resource.value["text"].text == "verified previous page" && resource.status == 504)
    await resource.load("/song/a/comments?page=2", store: store, identity: "a:comments:page2")
    #expect(resource.value.object.isEmpty)
    runtime.fail = false
    await resource.load("/song/b/background", store: store, identity: "b:background")
    await store.disconnect()
    runtime.fail = true
    await resource.load("/song/b/background?refresh=true", store: store, identity: "b:background")
    #expect(resource.value.object.isEmpty)
    await store.shutdown()
}

@Test func historyUsesSelectedTimezoneAcrossDayAndDaylightSavingBoundaries() {
    let timestamp = ISO8601DateFormatter().date(from: "2026-03-08T07:30:00Z")!.timeIntervalSince1970 * 1000
    let utc = listeningHistoryDate(milliseconds: timestamp, timezone: "UTC")
    let shanghai = listeningHistoryDate(milliseconds: timestamp, timezone: "Asia/Shanghai")
    let newYork = listeningHistoryDate(milliseconds: timestamp, timezone: "America/New_York")
    #expect(utc != shanghai && utc != newYork && newYork != shanghai)
    #expect(listeningHistoryDate(milliseconds: timestamp, timezone: "bad/timezone") == utc)
    #expect(listeningFinishReason("disconnect") == "设备断开")
    #expect(listeningFinishReason("") == "会话未结束")
    #expect(listeningFinishReason("future-reason") == "future-reason")
}

@Test @MainActor func idleListeningStatisticsRefreshAtCoreMidnight() async throws {
    struct Finished: Error {}
    let runtime = MidnightStatsRuntime(), store = MusicStore(runtime: runtime), resource = ResourceStore()
    store.connected = true
    store.coreReachable = true
    var time = try #require(ISO8601DateFormatter().date(from: "2026-11-14T15:59:58Z"))
    var waits: [TimeInterval] = []
    await resource.followListeningStats("/stats?period_days=1", store: store, now: { time }, sleep: { duration in
        waits.append(duration)
        if waits.count == 2 { throw Finished() }
        time.addTimeInterval(duration)
    })
    #expect(waits == [2, 86400])
    #expect(runtime.reads == 2)
    #expect(resource.value["end_day"].text == "2026-11-15")
    #expect(resource.value["summary"]["listened_seconds"].number == 0)
}

@MainActor private final class MidnightStatsRuntime: RuntimeTransport {
    var reads = 0
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        reads += 1
        return .object([
            "end_day": .string(reads == 1 ? "2026-11-14" : "2026-11-15"), "timezone": .string("Asia/Shanghai"),
            "summary": .object(["listened_seconds": .number(reads == 1 ? 1800 : 0)])
        ])
    }
    func shutdown() async {}
}

@Test func removedDevicePurgeRequiresAdminCapabilityAndExactNameAndUsesStableID() throws {
    let target = DevicePurge(id: "device-id", name: "我的 DAC")
    let value = try target.payload(confirmation: "我的 DAC", admin: true, supported: true)
    #expect(value["confirmation"].text == "device-id")
    #expect(throws: RuntimeFailure.self) { try target.payload(confirmation: "我的 DAC ", admin: true, supported: true) }
    #expect(throws: RuntimeFailure.self) { try target.payload(confirmation: "我的 DAC", admin: false, supported: true) }
    #expect(throws: RuntimeFailure.self) { try target.payload(confirmation: "我的 DAC", admin: true, supported: false) }
}
