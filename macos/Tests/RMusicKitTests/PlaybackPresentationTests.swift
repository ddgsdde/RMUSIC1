import Foundation
import Testing
@testable import RMusicKit

@Test @MainActor func audiblePresentationUsesMeasuredAgeAndMonotonicTimeWithoutCrossingSessions() throws {
    let zone = try JSONDecoder().decode(Zone.self, from: Data(#"{"id":"z","name":"QA","endpoint_id":"e","status":"playing","track":null,"playback_id":"p","position":1,"volume":1,"muted":false,"queue_index":0,"queue":[]}"#.utf8))
    let clock = PlaybackPresentationClock()
    clock.record(zone, at: 100)
    clock.accept(.object(["local": .bool(true), "playback_id": .string("p"), "position": .number(10), "playing": .bool(true), "age_ms": .number(100)]), zone: zone, roundTrip: 0.02, at: 100)
    #expect(abs(clock.reading(zone, at: 100.04).position - 10.15) < 0.00001)
    clock.accept(.object(["local": .bool(true), "playback_id": .string("p"), "position": .number(10.2), "playing": .bool(false), "age_ms": .number(10)]), zone: zone, roundTrip: 0.02, at: 100.1)
    #expect(abs(clock.reading(zone, at: 100.2).position - 10.21) < 0.00001 && !clock.reading(zone, at: 100.2).playing)
    clock.accept(.object(["playback_id": .string("old"), "position": .number(900)]), zone: zone, roundTrip: 0, at: 100.2)
    #expect(abs(clock.reading(zone, at: 100.2).position - 10.21) < 0.00001)
    clock.reset(); #expect(clock.reading(zone, at: 100.3).position == 1)
}

@Test func mineradioPayloadKeepsLyricOffsetSeparateFromTheAudioClock() {
    let playback = MineradioPlayback(id: "t", session: "p", queueIndex: 0, title: "QA", artist: "QA", position: 10, duration: 20, playing: false, lyricOffset: 0.1)
    #expect(playback.dictionary["position"] as? Double == 10)
    #expect(playback.dictionary["lyricOffset"] as? Double == 0.1)
    #expect(VisualFeature(playback_id: "p", position: 10, bands: Array(repeating: 0, count: 8), energy: 0, peak: 0).dictionary["position"] as? Double == 10)
}
@Test @MainActor func repeatedBrowserSnapshotsDoNotRewindLyricsBetweenFiveSecondReports() throws {
    let raw = #"{"id":"z","name":"QA","endpoint_id":"browser","status":"playing","track":null,"playback_id":"p","position":10,"updated_at":996000,"volume":1,"muted":false,"queue_index":0,"queue":[]}"#
    let zone = try JSONDecoder().decode(Zone.self, from: Data(raw.utf8)), clock = PlaybackPresentationClock()
    clock.record(zone, at: 100, wallTime: 1000)
    #expect(clock.reading(zone, at: 100).position == 14)
    clock.record(zone, at: 101, wallTime: 1001)
    #expect(clock.reading(zone, at: 101).position >= 15)
    let before = clock.reading(zone, at: 101.1).position
    clock.record(zone, at: 101.1, wallTime: 1001)
    #expect(clock.reading(zone, at: 101.1).position >= before)
}
@Test @MainActor func remoteAnalysisJitterDoesNotRewindLyricsButAnExplicitSeekStillMovesImmediately() throws {
    let zone = try JSONDecoder().decode(Zone.self, from: Data(#"{"id":"z","name":"QA","endpoint_id":"browser","status":"playing","track":null,"playback_id":"p","position":10,"volume":1,"muted":false,"queue_index":0,"queue":[]}"#.utf8))
    let clock = PlaybackPresentationClock(); clock.record(zone, at: 100)
    func report(_ position: Double, at: Double) { clock.accept(.object(["playback_id": .string("p"),"position": .number(position),"age_ms": .number(0)]), zone: zone, roundTrip: 0, at: at) }
    report(9.9, at: 100); #expect(clock.reading(zone, at: 100).position >= 10)
    var last = clock.reading(zone, at: 100).position
    for step in 1...50 {
        let at = 100 + Double(step)*0.1, before = clock.reading(zone, at: at).position
        report(10 + Double(step)*0.1 - (step%2 == 0 ? 0.08 : 0.02), at: at)
        let next = clock.reading(zone, at: at).position
        #expect(next >= before && next >= last); last = next
    }
    report(3, at: 105.1); #expect(clock.reading(zone, at: 105.1).position == 3)
}

@Test @MainActor func localClockDoesNotFallBackToTheCoreEstimateDuringAStall() throws {
    let zone = Zone(id: "z", name: "Mac", endpoint_id: "local", status: "playing", track: nil, position: 8, volume: 1, muted: false, playback_id: "p", queue_index: 0, queue: [])
    let clock = PlaybackPresentationClock(); clock.record(zone, at: 100)
    clock.accept(.object(["local": .bool(true), "playback_id": .string("p"), "position": .number(2), "playing": .bool(true)]), zone: zone, roundTrip: 0, at: 100)
    #expect(clock.reading(zone, at: 100.3).position == 2.25)
    #expect(!clock.reading(zone, at: 100.3).playing)
    clock.record(zone, at: 101)
    #expect(clock.reading(zone, at: 101).position == 2.25)
}

@Test @MainActor func localCursorJitterAndBufferingDoNotRewindTheProgressBar() {
    let zone = Zone(id: "z", name: "Mac", endpoint_id: "local", status: "playing", track: nil, position: 0, volume: 1, muted: false, playback_id: "p", queue_index: 0, queue: [])
    let clock = PlaybackPresentationClock(); clock.record(zone, at: 100)
    var last = 0.0
    for step in 0...60 {
        let at = 100 + Double(step)*0.05, before = clock.reading(zone, at: at).position
        let position = step < 30 ? Double(step)*0.05 : 1.45
        clock.accept(.object(["local": .bool(true), "playback_id": .string("p"), "position": .number(position), "playing": .bool(step < 30), "age_ms": .number(step%2 == 0 ? 30 : 5)]), zone: zone, roundTrip: 0.002, at: at)
        let next = clock.reading(zone, at: at).position
        #expect(next >= before && next >= last)
        #expect(next <= position + 0.25)
        last = next
    }
}

@Test @MainActor func localStartupWaitsForAudibleProgressInsteadOfTheCoreWallClock() {
    var zone = Zone(id: "z", name: "Mac", endpoint_id: "local", status: "playing", track: nil, position: 0, volume: 1, muted: false, playback_id: "new", queue_index: 0, queue: [])
    let clock = PlaybackPresentationClock(); clock.record(zone, local: true, at: 100)
    #expect(clock.reading(zone, at: 100.2).position == 0)
    zone.position = 0.2; clock.record(zone, local: true, at: 100.2)
    #expect(clock.reading(zone, at: 100.2).position == 0)
    clock.accept(.object(["local": .bool(true), "playback_id": .string("new"), "position": .number(0.01), "playing": .bool(true)]), zone: zone, roundTrip: 0, at: 100.3)
    #expect(clock.reading(zone, at: 100.3).position == 0.01)
}

@MainActor private final class ClockRuntime: RuntimeTransport {
    var observations = 0
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        if operation == "playback_observation" {
            observations += 1
            #expect(!body["audio"].boolean)
            return .object(["local": .bool(true), "playback_id": .string("p"), "position": .number(Double(observations)*0.1), "playing": .bool(true)])
        }
        return .null
    }
    func shutdown() async {}
}

@Test @MainActor func transportClockKeepsLowFrequencyProgressWhenUnfocused() async throws {
    let io = ClockRuntime(), store = MusicStore(runtime: io)
    store.connected = true
    store.zones = [Zone(id: "z", name: "Mac", endpoint_id: "local", status: "playing", track: nil, position: 0, volume: 1, muted: false, playback_id: "p", queue_index: 0, queue: [])]
    store.selectedZoneID = "z"
    store.outputs.items = [NativeOutput(id: "local", uid: "virtual", name: "QA", available: true, enabled: true, online: true, mix_rate: 48000, channels: 2, system_default: false, config: OutputConfiguration(enabled: true, device_id: "virtual", exclusive: false, buffer_ms: 100, max_sample_rate: nil, max_bit_depth: nil), report: nil)]
    store.startPolling(); defer { store.stopPolling() }
    let deadline = ContinuousClock.now + .seconds(0.7)
    while io.observations < 2 && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(10)) }
    #expect(io.observations >= 2)
    #expect(store.presentationClock.reading(store.zone).position > 0)
    store.setUIActive(false)
    let count = io.observations
    try await Task.sleep(for: .milliseconds(650))
    #expect(io.observations > count)
    #expect(io.observations <= count + 3)
}
