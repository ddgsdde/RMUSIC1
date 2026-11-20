import Foundation
import Observation

/// Shared, bounded inventory labels. UI selection never mutates a zone or device name.
@MainActor @Observable final class OutputLabels {
    private var labels: [String: String] = [:]
    @ObservationIgnored private var refreshed = Date.distantPast
    @ObservationIgnored private var owner: UUID?
    @ObservationIgnored private var loading = false
    func reset() { labels = [:]; refreshed = .distantPast; owner = nil }
    func accept(_ devices: [JSONValue]) {
        var next: [String: String] = [:]
        for device in devices {
            for output in device["outputs"].array {
                let name = output["name"].text, host = device["name"].text
                if !output["id"].text.isEmpty { next[output["id"].text] = [host, name].filter { !$0.isEmpty }.joined(separator: " · ") }
            }
        }
        labels = next; refreshed = Date()
    }
    func label(_ zone: Zone) -> String { labels[zone.endpoint_id] ?? zone.name }
    func refresh(_ store: MusicStore) async {
        if owner != store.sessionID { reset(); owner = store.sessionID }
        guard !loading, Date().timeIntervalSince(refreshed) >= 5 else { return }
        loading = true; defer { loading = false }
        let epoch = store.sessionID
        if let value = try? await store.api("/devices"), epoch == store.sessionID, !Task.isCancelled { accept(value["items"].array) }
    }
}
