import Foundation
import Observation

/// Local snapshots and remote Core leases share the playback's existing analysis tap.
@MainActor @Observable final class VisualFeatureStore {
    var frames: [VisualFeature] = []
    var available = false
    func follow(_ store: MusicStore, zoneID: String, playbackID: String, reactive: Bool) async {
        frames = []; available = false
        var after: Double?
        defer { frames = []; available = false }
        while !Task.isCancelled {
            guard store.connected, store.zone?.id == zoneID, store.zone?.playback_id == playbackID, store.zone?.status == "playing", !store.systemSleeping else { return }
            do {
                guard let zone = store.zone else { return }
                let began = ProcessInfo.processInfo.systemUptime
                let local = store.outputs.contains(zone.endpoint_id)
                var body: [String: JSONValue] = [:]
                if let after { body["after"] = .number(after) }
                if !local && !reactive { frames = []; available = false; try await Task.sleep(for: .seconds(1)); continue }
                let report: JSONValue
                if local {
                    report = try await store.runtimeCall("playback_observation", .object(["endpoint_id": .string(zone.endpoint_id), "audio": .bool(reactive)]))
                } else {
                    report = try await store.api("/zones/\(zoneID.pathComponentEncoded)/visualization", method: "POST", body: .object(body))
                }
                try Task.checkCancellation()
                guard store.zone?.id == zoneID, store.zone?.playback_id == playbackID else { return }
                if local && report["playback_id"].string != playbackID { try await Task.sleep(for: .milliseconds(50)); continue }
                guard report["playback_id"].string == playbackID else { frames = []; available = false; try await Task.sleep(for: .milliseconds(250)); continue }
                if !local { store.presentationClock.accept(report, zone: zone, roundTrip: ProcessInfo.processInfo.systemUptime - began) }
                available = report["supported"].boolean
                guard available else { frames = []; try await Task.sleep(for: .seconds(1)); continue }
                let incoming = try report["frames"].decode([VisualFeature].self).filter { $0.valid && $0.playback_id == playbackID }
                if !incoming.isEmpty { frames = Array(incoming.prefix(24)); after = incoming.map(\.position).max() }
                try await Task.sleep(for: .milliseconds(local ? 50 : 100))
            } catch is CancellationError { return }
            catch { frames = []; available = false; do { try await Task.sleep(for: .seconds(1)) } catch { return } }
        }
    }
}
