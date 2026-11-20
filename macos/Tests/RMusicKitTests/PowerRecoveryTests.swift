import Foundation
import Testing
import AppKit
@testable import RMusicKit

@MainActor private final class PowerRuntime: RuntimeTransport {
    var operations: [String] = []
    var failWake = false
    var holdWake = false
    var wakeEntered = false
    var wakeResult: CheckedContinuation<Void, Never>?
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        operations.append(operation)
        if operation == "resume_outputs", holdWake {
            wakeEntered = true
            await withCheckedContinuation { wakeResult = $0 }
        }
        if operation == "resume_outputs", failWake { throw RuntimeFailure(message: "Core offline", status: 503) }
        return .object(["host_id": .string("qa"), "items": .array([]), "jobs": .object([:]), "used_bytes": .number(0), "limit_bytes": .number(1024)])
    }
    func shutdown() async {}
}

@Test @MainActor func shutdownDuringWakeDiscardsLateResultsAndCannotRestartPolling() async {
    let runtime = PowerRuntime(), store = MusicStore(runtime: runtime)
    store.connected = true
    await store.suspend()
    runtime.holdWake = true
    let wake = Task { await store.wake() }
    let deadline = ContinuousClock.now + .seconds(3)
    while !runtime.wakeEntered, ContinuousClock.now < deadline { await Task.yield() }
    #expect(runtime.wakeEntered)
    await store.shutdown()
    runtime.wakeResult?.resume(); runtime.wakeResult = nil
    await wake.value
    #expect(!store.connected && !store.systemSleeping)
    #expect(runtime.operations == ["suspend_outputs", "resume_outputs"])
}

@Test @MainActor func wakeUsesExistingSessionAndRetriesCoreWithoutInventingPlayCommands() async {
    let runtime = PowerRuntime(), store = MusicStore(runtime: runtime)
    store.connected = true; store.coreReachable = true
    let session = store.sessionID
    await store.suspend(); await store.suspend(); await store.wake(); await store.wake()
    #expect(store.connected && !store.systemSleeping && store.sessionID == session)
    #expect(runtime.operations.prefix(2) == ["suspend_outputs", "resume_outputs"])
    #expect(runtime.operations.filter { $0 == "resume_outputs" }.count == 1)
    #expect(!runtime.operations.contains("connect") && !runtime.operations.contains("disconnect"))
    runtime.failWake = true
    await store.suspend(); await store.wake()
    #expect(store.connected && !store.coreReachable && !store.systemSleeping)
    runtime.failWake = false
    try? await store.pollOnce()
    #expect(store.coreReachable && store.sessionID == session)
    await store.shutdown()
}

@Test @MainActor func disconnectDuringSleepPreventsWakeFromResurrectingAnOldSession() async {
    let runtime = PowerRuntime(), store = MusicStore(runtime: runtime)
    store.connected = true
    await store.suspend(); await store.disconnect(); await store.wake()
    #expect(!store.connected && !store.systemSleeping)
    #expect(runtime.operations == ["suspend_outputs", "disconnect"])
    await store.shutdown()
}

@Test @MainActor func sleepDuringLoginQueuesItsOutputJoinAndWakeCannotLeaveTheGateClosed() async {
    let runtime = PowerRuntime(), store = MusicStore(runtime: runtime)
    store.connecting = true
    await store.suspend(); await store.wake()
    #expect(runtime.operations == ["suspend_outputs", "resume_outputs"])
    #expect(!store.connected && !store.systemSleeping)
    await store.shutdown()
}

@Test @MainActor func rapidWorkspaceSleepWakeEventsAreSerializedBoundedAndObserversRelease() async throws {
    let center = NotificationCenter()
    var transitions: [Bool] = [], active = 0, maximum = 0
    var monitor: SystemPowerMonitor? = SystemPowerMonitor(center: center) { sleeping in
        active += 1; maximum = max(maximum, active)
        transitions.append(sleeping)
        try? await Task.sleep(for: .milliseconds(5))
        active -= 1
    }
    weak var released = monitor
    for _ in 0..<1000 {
        center.post(name: NSWorkspace.willSleepNotification, object: nil)
        center.post(name: NSWorkspace.didWakeNotification, object: nil)
    }
    let deadline = ContinuousClock.now + .seconds(3)
    while (transitions != [true, false] || active != 0), ContinuousClock.now < deadline { await Task.yield() }
    #expect(transitions == [true, false] && maximum == 1)
    monitor = nil
    // The worker returns on the next turn after its final await; it must not form an idle owner cycle.
    while released != nil, ContinuousClock.now < deadline { await Task.yield() }
    #expect(released == nil)
    center.post(name: NSWorkspace.willSleepNotification, object: nil)
    await Task.yield()
    #expect(transitions == [true, false])
}

@Test func mediaControlsPreferActualLocalPlaybackOverSelectedIdleOrRemoteZones() {
    let track = Track(id: "t", title: "QA", artist: "QA", album: "QA", duration: 10, format: "flac", sample_rate: 44100, bit_depth: 24)
    func zone(_ id: String, _ endpoint: String, _ status: String) -> Zone {
        Zone(id: id, name: id, endpoint_id: endpoint, status: status, track: track, position: 2, volume: 1, muted: false, playback_id: "p", queue_index: 0, queue: [track])
    }
    let idle = zone("idle", "a", "stopped"), playing = zone("playing", "b", "playing"), paused = zone("paused", "a", "paused"), remote = zone("remote", "other", "playing")
    #expect(LocalPlaybackSelection.resolve(zones: [idle, playing, remote], selectedID: "idle", enabledEndpoints: ["a", "b"])?.id == "playing")
    #expect(LocalPlaybackSelection.resolve(zones: [idle, playing, remote], selectedID: "remote", enabledEndpoints: ["a", "b"])?.id == "playing")
    #expect(LocalPlaybackSelection.resolve(zones: [paused, playing], selectedID: "paused", enabledEndpoints: ["a"])?.id == "paused")
    #expect(LocalPlaybackSelection.resolve(zones: [remote], selectedID: "remote", enabledEndpoints: ["a"]) == nil)
}

@Test @MainActor func sleepKeepsAuthenticatedCoreContextAndOnlyJoinsNativeOutputs() async {
    let runtime = PowerRuntime(), store = MusicStore(runtime: runtime)
    store.connected = true; store.coreReachable = true
    store.user = .object(["id": .string("qa-owner")])
    await store.suspend()
    #expect(store.connected)
    #expect(store.user["id"].text == "qa-owner")
    #expect(runtime.operations == ["suspend_outputs"])
    await store.shutdown()
}
