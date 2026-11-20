import Foundation

/// Media keys follow an actual active local zone even when the window selects an idle/remote one.
enum LocalPlaybackSelection {
    static func resolve(zones: [Zone], selectedID: String, enabledEndpoints: Set<String>) -> Zone? {
        let local = zones.filter { enabledEndpoints.contains($0.endpoint_id) }
        let selected = local.first { $0.id == selectedID }
        if let selected, selected.status == "playing", selected.track != nil { return selected }
        if let playing = local.first(where: { $0.status == "playing" && $0.track != nil }) { return playing }
        if let selected, selected.status == "paused", selected.track != nil { return selected }
        return local.first { $0.status == "paused" && $0.track != nil } ?? selected ?? local.first
    }
}
