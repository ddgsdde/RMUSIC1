import Foundation

/// Present control intent immediately while preserving Core command order and group ownership.
@MainActor final class PlaybackCommands {
    private(set) var revisions: [String: UUID] = [:]
    private var pending: [String: UUID] = [:]
    private var writers: [String: Task<Void, Never>] = [:]

    func reset() {
        writers.values.forEach { $0.cancel() }
        writers.removeAll(); pending.removeAll(); revisions.removeAll()
    }

    func reconcile(_ incoming: [Zone], previous: [Zone], since snapshot: [String: UUID]) -> [Zone] {
        var result = incoming.map { zone in
            if pending[zone.id] != nil || revisions[zone.id] != snapshot[zone.id] {
                return previous.first { $0.id == zone.id } ?? zone
            }
            return zone
        }
        for zone in previous where pending[zone.id] != nil && !result.contains(where: { $0.id == zone.id }) {
            result.append(zone)
        }
        return result
    }

    func send(_ action: String, body: [String: JSONValue], zone: Zone, store: MusicStore) async {
        let stamp = UUID(), session = store.sessionID, predecessor = writers[zone.id]
        revisions[zone.id] = stamp; pending[zone.id] = stamp
        show(project(action, body: body, zone: zone, store: store), store: store)
        let work = Task { @MainActor [weak self, weak store] in
            await predecessor?.value
            guard let self, let store, !Task.isCancelled, store.sessionID == session else { return }
            do {
                try await store.requireUngrouped(zone.id)
                guard !Task.isCancelled, store.sessionID == session else { return }
                let reply = try await store.api("/zones/\(zone.id.pathComponentEncoded)/\(action)", method: "POST", body: .object(body)).decode(Zone.self)
                guard store.sessionID == session, self.pending[zone.id] == stamp else { return }
                self.show(reply, store: store)
                store.details.update(store.zone?.track, store: store)
                store.details.prefetch(store.zone, store: store)
                store.error = nil
            } catch {
                guard store.sessionID == session, self.pending[zone.id] == stamp else { return }
                self.show(zone, store: store)
                await store.handle(error)
            }
            if self.pending[zone.id] == stamp {
                self.pending.removeValue(forKey: zone.id)
                self.revisions[zone.id] = UUID()
                self.writers.removeValue(forKey: zone.id)
            }
        }
        writers[zone.id] = work
        await work.value
    }

    private func show(_ zone: Zone, store: MusicStore) {
        guard let index = store.zones.firstIndex(where: { $0.id == zone.id }) else { return }
        store.zones[index] = zone
        if store.selectedZoneID == zone.id {
            store.presentationClock.reset(); store.presentationClock.record(zone, local: store.outputs.items.contains { $0.id == zone.endpoint_id })
            store.updateSystemPlayback()
        }
    }

    private func project(_ action: String, body: [String: JSONValue], zone original: Zone, store: MusicStore) -> Zone {
        var zone = original
        switch action {
        case "pause":
            zone.position = store.presentationClock.reading(original).position
            zone.status = "paused"
        case "play" where body["queue_index"] == nil && body["track_id"] == nil && zone.status == "paused":
            zone.status = "playing"
        case "play", "next", "previous":
            var index = body["queue_index"]?.integer ?? zone.queue_index
            if let id = body["track_id"]?.string, let found = zone.queue.firstIndex(where: { $0.id == id }) { index = found }
            if action == "previous" { index = max(0, index - 1) }
            if action == "next" && zone.repeat != "one" { index += 1; if index >= zone.queue.count && zone.repeat == "all" { index = 0 } }
            zone.position = 0; zone.playback_id = nil
            if zone.queue.indices.contains(index) {
                zone.queue_index = index; zone.track = zone.queue[index]; zone.status = "playing"
            } else if action == "next" { zone.status = "stopped" }
        case "seek": zone.position = max(0, body["position"]?.number ?? zone.position)
        case "stop": zone.status = "stopped"; zone.position = 0; zone.playback_id = nil
        default: break
        }
        zone.updated_at = .number(Date.now.timeIntervalSince1970 * 1000)
        return zone
    }
}
