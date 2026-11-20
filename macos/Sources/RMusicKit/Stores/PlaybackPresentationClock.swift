import Foundation
import Observation

/// Presentation only. Never changes the Core queue, HAL clock, PCM or output buffers.
@MainActor @Observable final class PlaybackPresentationClock {
    private struct Anchor { let id: String; var position: Double; let at: Double; let playing: Bool; let ttl: Double; var rate = 1.0; var local = false }
    private var measured: Anchor?
    private var fallback: Anchor?
    func reset() { measured = nil; fallback = nil }
    func record(_ zone: Zone, local: Bool = false, at: Double = ProcessInfo.processInfo.systemUptime, wallTime: Double = Date.now.timeIntervalSince1970) {
        if local && fallback?.id == key(zone) && measured?.id != key(zone) { return }
        let stamp = zone.updated_at?.number ?? 0
        let age = stamp > 0 && zone.status == "playing" ? max(0, wallTime - stamp / 1000) : 0
        if age > 8 {
            if fallback?.id == key(zone) { return }
            fallback = Anchor(id: key(zone), position: zone.position, at: at, playing: false, ttl: 2); return
        }
        var next = Anchor(id: key(zone), position: zone.position + (local ? 0 : age), at: at, playing: !local && zone.status == "playing", ttl: 2)
        if let old = fallback, old.id == next.id, old.playing, next.playing, at-old.at < old.ttl {
            let shown = reading(zone, at: at).position, correction = next.position-shown
            if abs(correction) < 0.75 { next = Anchor(id: next.id, position: shown, at: at, playing: true, ttl: 2, rate: min(1.15,max(0.85,1+correction*0.25))) }
        }
        fallback = next
    }
    func accept(_ value: JSONValue, zone: Zone, roundTrip: Double, at: Double = ProcessInfo.processInfo.systemUptime) {
        guard let id = zone.playback_id, value["playback_id"].string == id,
              case .number(let position) = value["position"], position.isFinite, position >= 0 else { return }
        let age = max(0, value["age_ms"].number / 1000)
        guard age.isFinite, age < 1 else { return }
        let local = value["local"].boolean
        let playing = local ? value["playing"].boolean : zone.status == "playing"
        var next = Anchor(id: key(zone), position: position + (playing ? age + min(0.1, max(0, roundTrip) / 2) : 0), at: at, playing: playing, ttl: local ? 0.25 : 1, local: local)
        let previous = local ? measured : measured ?? fallback
        if let old = previous, old.id == next.id, local || (playing && old.playing && at-old.at < old.ttl) {
            let shown = reading(zone, at: at).position, correction = next.position-shown
            if local { next.position = max(shown, next.position) }
            if abs(correction) < 0.75 {
                next = Anchor(id: next.id, position: playing ? shown : max(shown, next.position), at: at, playing: playing, ttl: next.ttl, rate: min(1.15,max(0.85,1+correction*0.25)), local: local)
            }
        }
        measured = next
    }
    func reading(_ zone: Zone?, at: Double = ProcessInfo.processInfo.systemUptime) -> (position: Double, playing: Bool) {
        guard let zone else { return (0, false) }
        let candidates = [measured, fallback].compactMap { $0 }.filter { $0.id == key(zone) }
        // A local cursor remains authoritative when its lease expires: freeze rather
        // than replacing a stalled device clock with Core's advancing estimate.
        let anchor = candidates.first(where: { $0.local }) ?? candidates.first { at - $0.at <= $0.ttl } ?? candidates.first
        guard let anchor else { return (zone.position, zone.status == "playing") }
        let elapsed = max(0, at - anchor.at), running = anchor.playing && zone.status == "playing" && elapsed <= anchor.ttl
        let position = anchor.position + (anchor.playing && zone.status == "playing" ? min(elapsed, anchor.ttl)*anchor.rate : 0)
        let duration = zone.track?.duration ?? 0
        return (max(0, duration > 0 ? min(position, duration) : position), running)
    }
    private func key(_ zone: Zone) -> String { "\(zone.id):\(zone.playback_id ?? "")" }
}
