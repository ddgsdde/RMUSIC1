import Foundation
import Observation

@MainActor @Observable
final class OutputsStore {
    var items: [NativeOutput] = []
    var detected: [AudioDevice] = []
    var busy: Set<String> = []
    var refreshing = false
    var roaming = false
    var removed = false
    private(set) var hostID = ""
    var error: String?
    var enabled: [NativeOutput] { items.filter { $0.enabled } }
    func reset() { items = []; busy = []; removed = false; roaming = false; hostID = ""; error = nil }
    func accept(_ response: OutputInventory) { roaming = response.roaming == true; hostID = response.host_id; items = arranged(response.items); removed = response.removed == true; error = nil }
    func refresh(_ store: MusicStore) async throws {
        guard !refreshing else { return }; refreshing = true; defer { refreshing = false }
        let epoch = store.sessionID
        if store.connected {
            let response: OutputInventory
            do { response = try await store.runtimeCall("outputs").decode(OutputInventory.self) }
            catch {
                if !store.bridgeOnly, let local = try? await store.runtimeCall("local_outputs",.object(["host_id":.string(hostID.isEmpty ? store.endpointID:hostID)])).decode(OutputInventory.self), epoch == store.sessionID {
                    let previous = roaming; accept(local); roaming = previous
                }
                throw error
            }
            guard epoch == store.sessionID, store.connected else { return }
            accept(response)
        } else if !store.bridgeOnly {
            let inventory = try await store.runtimeCall("local_outputs",.object(["host_id":.string(store.endpointID)])).decode(OutputInventory.self)
            if epoch == store.sessionID { accept(inventory) }
        } else { detected = try await store.runtimeCall("devices").decode([AudioDevice].self) }
    }
    func configure(_ output: NativeOutput, changes: [String: JSONValue], store: MusicStore) async throws {
        guard !busy.contains(output.id) else { return }
        let epoch = store.sessionID; busy.insert(output.id)
        defer { if epoch == store.sessionID { busy.remove(output.id) } }
        let offline = !store.bridgeOnly && (!store.connected || !store.coreReachable)
        let response = try await store.runtimeCall(offline ? "local_configure" : "configure_output", .object(["host_id":.string(hostID.isEmpty ? store.endpointID : hostID),"id": .string(output.id), "changes": .object(changes)])).decode(OutputInventory.self)
        guard epoch == store.sessionID else { return }
        let previous = roaming; accept(response); if offline && store.connected {roaming=previous}
        if !offline {try await store.refreshZones()}
    }
    func restore(_ store: MusicStore) async throws {
        let response = try await store.runtimeCall("restore_outputs").decode(OutputInventory.self)
        accept(response)
    }
    private func arranged(_ outputs: [NativeOutput]) -> [NativeOutput] {
        outputs.sorted { a, b in
            if a.followsSystemDefault != b.followsSystemDefault { return a.followsSystemDefault }
            return a.name == b.name ? a.id < b.id : a.name.localizedStandardCompare(b.name) == .orderedAscending
        }
    }
    func isLocalDevice(_ device: JSONValue) -> Bool {
        if !hostID.isEmpty { return device["id"].text == hostID }
        return !device["outputs"].array.isEmpty && device["outputs"].array.allSatisfy { contains($0["id"].text) }
    }
    func arrangedDevices(_ devices: [JSONValue]) -> [JSONValue] {
        devices.enumerated().sorted { a, b in
            let localA = isLocalDevice(a.element), localB = isLocalDevice(b.element)
            return localA != localB ? localA : a.offset < b.offset
        }.map(\.element)
    }
    func output(for zone: Zone?) -> NativeOutput? { items.first { $0.id == zone?.endpoint_id } }
    func contains(_ endpoint: String) -> Bool { items.contains { $0.id == endpoint } }
}
