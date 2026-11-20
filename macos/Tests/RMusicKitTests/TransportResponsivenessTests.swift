import Foundation
import Testing
@testable import RMusicKit

@MainActor private final class ControlRuntime: RuntimeTransport {
    var paths: [String] = []
    var groupGate: CheckedContinuation<Void, Never>?
    var holdGroups = true
    var groupActive = false
    var controlGate: CheckedContinuation<Void, Never>?
    var holdControl = false
    var zoneGate: CheckedContinuation<Void, Never>?
    var holdZones = false
    var failControl = false
    var delay = Duration.zero
    var beforeControl: (() -> Void)?
    var current = Zone(id: "controls", name: "Mac", endpoint_id: "local", status: "playing", track: nil, position: 12, volume: 1, muted: false, playback_id: "first", queue_index: 0, queue: [])
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        guard operation == "request" else { return .null }
        let path = body["path"].text
        paths.append(path)
        if delay != .zero { try await Task.sleep(for: delay) }
        if path.hasPrefix("/sync/starts") {
            if holdGroups { await withCheckedContinuation { groupGate = $0 } }
            return .object(["items": .array(groupActive ? [.object(["members": .array([.object(["zone_id": .string(current.id)])])])] : [])])
        }
        if path.hasPrefix("/zones/controls/") {
            beforeControl?()
            if failControl { throw RuntimeFailure(message: "音区正由整组控制", status: 409) }
            let action = String(path.split(separator: "/").last!)
            if action == "pause" { current.status = "paused" }
            if action == "play" { current.status = "playing" }
            if action == "next" { current.queue_index = min(current.queue.count - 1, current.queue_index + 1) }
            if action == "previous" { current.queue_index = max(0, current.queue_index - 1) }
            if current.queue.indices.contains(current.queue_index) { current.track = current.queue[current.queue_index] }
            let reply = try JSONValue.encoded(current)
            if holdControl { await withCheckedContinuation { controlGate = $0 } }
            return reply
        }
        if path == "/zones" {
            let reply = JSONValue.object(["items": .array([try .encoded(current)])])
            if holdZones { await withCheckedContinuation { zoneGate = $0 } }
            return reply
        }
        return .object(["items": .array([])])
    }
    func shutdown() async {}
}

@Test @MainActor func lanPauseShowsIntentBeforeGroupCheckAndDoesNotReloadInventory() async throws {
    let io = ControlRuntime()
    let subject = MusicStore(runtime: io)
    subject.connected = true; subject.zones = [io.current]; subject.selectedZoneID = io.current.id
    let work = Task { await subject.transport("pause") }
    while io.groupGate == nil { await Task.yield() }
    let immediate = subject.zone?.status == "paused"
    io.holdGroups = false; io.groupGate?.resume(); io.groupGate = nil
    await work.value
    #expect(immediate, "Pause presentation must not wait for a network round trip")
    #expect(io.paths == ["/sync/starts?active=true&limit=100&offset=0", "/zones/controls/pause"])
    #expect(subject.zone?.status == "paused")
}


@MainActor private func controlStore(_ io: ControlRuntime) -> MusicStore {
    io.holdGroups = false
    let store = MusicStore(runtime: io)
    store.connected = true; store.zones = [io.current]; store.selectedZoneID = io.current.id
    return store
}

@Test @MainActor func rapidNextKeepsNewestPresentationAndSendsCommandsInOrder() async throws {
    let io = ControlRuntime()
    io.current.queue = ["a", "b", "c"].map { Track(id: $0, title: $0, artist: "", album: "", duration: 10, format: "flac", sample_rate: 48000, bit_depth: 24) }
    io.current.track = io.current.queue[0]
    let store = controlStore(io)
    io.holdControl = true
    let first = Task { await store.transport("next") }
    while io.controlGate == nil { await Task.yield() }
    let second = Task { await store.transport("next") }
    while store.zone?.queue_index != 2 { await Task.yield() }
    #expect(io.paths.filter { $0 == "/zones/controls/next" }.count == 1)
    var presentationAtSecondWrite = -1
    io.beforeControl = { presentationAtSecondWrite = store.zone?.queue_index ?? -1 }
    io.holdControl = false; io.controlGate?.resume(); io.controlGate = nil
    await first.value; await second.value
    io.beforeControl = nil
    #expect(presentationAtSecondWrite == 2)
    #expect(store.zone?.queue_index == 2 && store.zone?.track?.id == "c")
    #expect(io.paths.filter { $0 == "/zones/controls/next" }.count == 2)
}

@Test @MainActor func lateZoneReadDoesNotUndoACompletedPause() async throws {
    let io = ControlRuntime(), store = controlStore(io)
    io.holdZones = true
    let read = Task { try await store.refreshZones() }
    while io.zoneGate == nil { await Task.yield() }
    await store.transport("pause")
    io.holdZones = false; io.zoneGate?.resume(); io.zoneGate = nil
    try await read.value
    #expect(store.zone?.status == "paused")
}

@Test @MainActor func zoneReadStartedDuringPauseDoesNotUndoItsConfirmedResult() async throws {
    let io = ControlRuntime(), store = controlStore(io)
    io.holdGroups = true
    let pause = Task { await store.transport("pause") }
    while io.groupGate == nil { await Task.yield() }
    io.holdZones = true
    let read = Task { try await store.refreshZones() }
    while io.zoneGate == nil { await Task.yield() }
    io.holdGroups = false; io.groupGate?.resume(); io.groupGate = nil
    await pause.value
    io.holdZones = false; io.zoneGate?.resume(); io.zoneGate = nil
    try await read.value
    #expect(store.zone?.status == "paused")
    #expect(!store.presentationClock.reading(store.zone).playing)
}

@Test @MainActor func latePausedSnapshotDoesNotOverwriteNewerPlayingSnapshot() async throws {
    let io = ControlRuntime()
    io.current.status = "paused"
    let store = controlStore(io)
    io.holdZones = true
    let slowRead = Task { try await store.refreshZones() }
    while io.zoneGate == nil { await Task.yield() }
    io.holdZones = false
    io.current.status = "playing"
    io.current.position = 18
    try await store.refreshZones()
    #expect(store.zone?.status == "playing")
    io.zoneGate?.resume(); io.zoneGate = nil
    try await slowRead.value
    #expect(store.zone?.status == "playing")
    #expect(store.zone?.position == 18)

    io.current.status = "paused"
    try await store.refreshZones()
    #expect(store.zone?.status == "paused")
}

@Test @MainActor func rejectedGroupControlRestoresPresentationWithoutChangingTheGroup() async throws {
    let io = ControlRuntime(), store = controlStore(io)
    io.groupActive = true
    await store.transport("pause")
    #expect(store.zone?.status == "playing")
    #expect(!io.paths.contains("/zones/controls/pause"))
    #expect(store.error?.contains("整组") == true)
    io.groupActive = false; io.failControl = true
    await store.transport("pause")
    #expect(store.zone?.status == "playing")
    #expect(store.error?.contains("整组") == true)
}

@Test @MainActor func controlLatencyUsesOnlyValidationAndTheControlReply() async throws {
    let io = ControlRuntime(), store = controlStore(io)
    io.delay = .milliseconds(60)
    let start = ContinuousClock.now
    await store.transport("pause")
    print("LAN control fixture: HTTP calls=\(io.paths.count), artificial RTT=60 ms, elapsed=\(start.duration(to: .now))")
    #expect(io.paths.count == 2)
    #expect(store.zone?.status == "paused")
}

@Test @MainActor func deviceFadeDefaultsAreBalancedAndPreserveSeparateCustomAndOffChoices() async throws {
    let uid = "fade-qa-" + UUID().uuidString, other = "fade-qa-" + UUID().uuidString
    defer { for value in [uid, other] { UserDefaults.standard.removeObject(forKey: "outputEffects.v1." + value) } }
    func output(_ uid: String) throws -> NativeOutput {
        try JSONDecoder().decode(NativeOutput.self, from: JSONEncoder().encode(JSONValue.object([
            "id": .string("local"), "uid": .string(uid), "name": .string("QA"), "available": .bool(true), "enabled": .bool(true), "online": .bool(true),
            "mix_rate": .number(48000), "channels": .number(2), "system_default": .bool(false),
            "config": .object(["enabled": .bool(true), "exclusive": .bool(false), "buffer_ms": .number(100)])
        ])))
    }
    let effects = DeviceEffects(), base = ControlRuntime().current, device = try output(uid), second = try output(other)
    #expect(effects.zone(for: device, base: base).transitions == PlaybackSettingsDraft.balancedTransitions)
    let custom = JSONValue.object(["fade_in_ms": .number(50), "fade_out_ms": .number(90)])
    try effects.edit("transitions", value: custom, uid: uid)
    #expect(effects.zone(for: device, base: base).transitions == custom)
    #expect(effects.zone(for: second, base: base).transitions == PlaybackSettingsDraft.balancedTransitions)
    let off = JSONValue.object(["fade_in_ms": .number(0), "fade_out_ms": .number(0)])
    try effects.edit("transitions", value: off, uid: uid)
    #expect(DeviceEffects().zone(for: device, base: base).transitions == off)
    let io = ControlRuntime(), store = controlStore(io)
    try await store.effects.reconcile(second, zone: base, store: store)
    #expect(io.paths.last == "/zones/controls")
    var confirmed = base; confirmed.transitions = PlaybackSettingsDraft.balancedTransitions
    let count = io.paths.count
    try await store.effects.reconcile(second, zone: confirmed, store: store)
    #expect(io.paths.count == count)
}
