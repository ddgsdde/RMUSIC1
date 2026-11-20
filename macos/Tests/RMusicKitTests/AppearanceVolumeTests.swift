import Foundation
import Testing
@testable import RMusicKit

@Test @MainActor func themeAndAppearanceSurviveReopeningWithoutChangingSystemAppearance() throws {
    let suite = "rmusic-tests-\(UUID())"
    let defaults = UserDefaults(suiteName: suite)!
    defer { defaults.removePersistentDomain(forName: suite) }
    // All preferences use an isolated suite; the user's application defaults are untouched.
    let appearance = AppearanceStore(defaults: defaults)
    #expect(appearance.mode == "system")
    appearance.theme = .ocean; appearance.mode = "dark"
    let reopened = AppearanceStore(defaults: defaults)
    #expect(reopened.theme == .ocean)
    #expect(reopened.scheme == .dark)
}
@MainActor private final class VolumeRuntime: RuntimeTransport {
    var calls: [(String, JSONValue)] = []
    func call(_ operation: String, _ body: JSONValue) async throws -> JSONValue {
        calls.append((operation, body))
        return try volumeInventory(software: false, supported: true, enabled: true, level: 0.32)
    }
    func shutdown() async {}
}
private func volumeInventory(software: Bool, supported: Bool, enabled: Bool, level: Double = 0.4) throws -> JSONValue {
    .object(["host_id": .string("mac-test"), "items": .array([.object([
        "id": .string("output"), "uid": .string("fixture-device-uid"), "name": .string("Fixture"),
        "available": .bool(true), "enabled": .bool(true), "online": .bool(true), "mix_rate": .number(48000), "channels": .number(2), "system_default": .bool(false),
        "config": .object(["enabled": .bool(true), "exclusive": .bool(false), "buffer_ms": .number(100), "volume_control": .string(software ? "software" : supported && enabled ? "device" : "fixed")]),
        "hardware_volume_enabled": .bool(enabled), "device_volume": .object(["supported": .bool(supported), "value": .number(level)])
    ])])])
}
@Test @MainActor func deviceVolumeFixedMaximumAndSoftwareAreDistinctAndReadbackIsUsed() async throws {
    let runtime = VolumeRuntime()
    let owned = MusicStore(runtime: runtime)
    let zone = try JSONDecoder().decode(Zone.self, from: Data(#"{"id":"zone","name":"Fixture","endpoint_id":"output","status":"paused","track":null,"position":0,"volume":0.2,"muted":false,"queue_index":0,"queue":[]}"#.utf8))
    owned.zones = [zone]; owned.selectedZoneID = zone.id
    func load(_ software: Bool, _ supported: Bool, _ enabled: Bool) throws {
        owned.outputs.items = try volumeInventory(software: software, supported: supported, enabled: enabled).decode(OutputInventory.self).items
    }
    try load(false, false, true)
    #expect(owned.volumeMode == .fixed)
    #expect(owned.volumeValue == 1)
    await owned.setVolume(0.3)
    #expect(runtime.calls.isEmpty)
    try load(false, true, false)
    #expect(owned.volumeMode == .fixed)
    try load(false, true, true)
    #expect(owned.volumeMode == .device)
    #expect(owned.volumeValue == 0.4)
    await owned.setVolume(0.33)
    try #require(runtime.calls.count == 1)
    #expect(runtime.calls[0].0 == "set_device_volume")
    #expect(runtime.calls[0].1["id"].text == "output")
    #expect(owned.volumeValue == 0.32)
    try load(true, true, true)
    #expect(owned.volumeMode == .software)
    #expect(owned.volumeValue == 0.2)
}

@Test @MainActor func softwareVolumeAdjustmentUnmutesOnlyForPositiveLevels() async throws {
    let runtime = VolumeRuntime()
    let store = MusicStore(runtime: runtime)
    let zone = try JSONDecoder().decode(Zone.self, from: Data(#"{"id":"zone","name":"Fixture","endpoint_id":"output","status":"paused","track":null,"position":0,"volume":0.2,"muted":true,"queue_index":0,"queue":[]}"#.utf8))
    store.zones = [zone]; store.selectedZoneID = zone.id
    store.outputs.items = try volumeInventory(software: true, supported: true, enabled: true).decode(OutputInventory.self).items
    await store.setVolume(0)
    await store.setVolume(0.24)
    let patches = runtime.calls.map(\.1).filter { $0["method"].text == "PATCH" && $0["path"].text == "/zones/zone" }
    try #require(patches.count == 2)
    #expect(patches[0]["json"]["volume"].number == 0)
    #expect(patches[0]["json"].object["muted"] == nil)
    #expect(patches[1]["json"]["volume"].number == 0.24)
    #expect(patches[1]["json"].object["muted"]?.boolean == false)
}
