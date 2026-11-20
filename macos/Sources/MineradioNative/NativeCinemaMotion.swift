import Foundation
import simd

/// Pinned live (non-DJ) camera envelopes. Uses the existing timestamped tap;
/// delayed frames are aged in programme time, never replayed as new beats.
struct NativeCinemaMotion {
    struct Event {
        let start: Double, attack: Float, hold: Float, release: Float
        let amplitude: Float, zoom: Float, phi: Float, combo: Int
    }
    private var events: [Event] = []
    private var lastHit: Double = -10
    private var average: Float = 0, lowAverage: Float = 0, peak: Float = 0.30, dynamics: Float = 0.82
    private(set) var punch: Float = 0, radius: Float = 0, phi: Float = 0
    var eventCount: Int { events.count }
    mutating func reset() { self = NativeCinemaMotion() }
    private func clamp(_ x: Float, _ a: Float = 0, _ b: Float = 1) -> Float { min(b,max(a,x)) }
    private func ease(_ x: Float) -> Float { let t = clamp(x); return t*t*(3-2*t) }
    mutating func step(_ timeline: NativeFeatureTimeline, position: Double, playing: Bool, dt: Float) {
        let frames = max(0,dt*60)
        func blend(_ coefficient: Float) -> Float { 1-pow(1-coefficient,frames) }
        let low = (timeline.bands[0]+timeline.bands[1])*0.5
        let composite = clamp(timeline.energy*0.62+low*0.38)
        average += (composite-average)*blend(composite > average ? 0.010 : 0.004)
        lowAverage += (low-lowAverage)*blend(low > lowAverage ? 0.012 : 0.005)
        peak = max(0.30,peak*pow(0.9988,frames),composite)
        let floor = max(0.10,average*0.82), lift = ease((composite-floor)/max(0.18,peak-floor))
        var target = 0.42+lift*0.56+clamp((low-lowAverage)/0.36)*0.12
        if average < 0.18 && low < 0.32 { target *= 0.78 }
        if timeline.energy > 0.48 && low > 0.46 { target = max(target,0.92) }
        target = clamp(target,0.34,1.08)
        dynamics += (target-dynamics)*blend(target > dynamics ? 0.045 : 0.022)
        if playing {
            for hit in timeline.onsets where hit.position-lastHit >= 0.3312 || hit.strength >= 0.68 {
                let sum = max(0.001,hit.low+hit.body+0.16)
                let l = hit.low/sum, body = hit.body/sum, snap = 0.16/sum
                let mass = clamp(l*0.72+body*0.36+hit.strength*0.20)
                let deep = !(snap > 0.42 && snap > l*1.18 && snap > body*1.08) && !(body > 0.46 && body > l*1.12)
                let dyn = clamp(dynamics*(0.92+hit.strength*0.12+mass*0.08),0.18,1.18)
                var amplitude = clamp(0.15+hit.strength*0.34+0.75*0.06+mass*0.13+snap*0.04,0.18,0.72)*0.92
                if deep { amplitude = min(0.62,amplitude*1.12) }
                amplitude *= dyn
                let attack = clamp(0.028*(1.18-snap*0.55),0.014,0.038)
                let hold = clamp(0.030*(0.62+l*0.55+body*0.25),0.014,0.052)
                let release = clamp(0.185*(0.76+mass*0.56+body*0.18-snap*0.18),0.110,0.255)
                let combo = abs(Int(hit.position*2.7))%4
                var zoom = (0.070+mass*0.190+(deep ? 0.095 : 0.018)+hit.strength*0.045)*(0.76+dyn*0.28)
                var phi: Float = (body > 0.46 && body > l*1.12 ? 0.014 : deep ? 0.004 : 0.007)*(0.82+dyn*0.20)
                switch combo {
                case 0: amplitude *= 1.10; zoom *= 1.18; phi *= 0.72
                case 1: amplitude *= 0.84; zoom *= 0.88; phi *= 0.62
                case 2: amplitude *= 0.96; zoom *= 0.72; phi *= 1.22
                default: amplitude *= 0.74; zoom *= 0.62; phi *= 0.78
                }
                events.append(Event(start:hit.position-Double(attack*0.42),attack:attack,hold:hold,release:release,amplitude:clamp(amplitude,0.08,0.68),zoom:zoom,phi:phi,combo:combo))
                events = Array(events.suffix(8)); lastHit = hit.position
            }
        } else { events.removeAll(keepingCapacity:true) }
        events.removeAll { position-$0.start > Double($0.attack+$0.hold+$0.release) }
        var wanted: Float = 0, wantedRadius: Float = 0, wantedPhi: Float = 0
        for event in events {
            let age = Float(position-event.start)
            let envelope: Float = age < 0 ? 0 : age < event.attack ? ease(age/event.attack) : age < event.attack+event.hold ? 1 : 1-ease((age-event.attack-event.hold)/event.release)
            let value = envelope*event.amplitude
            guard value > wanted else { continue }; wanted = value
            wantedRadius = value*event.zoom*[Float(1),0.72,0.46,0.30][event.combo]
            wantedPhi = event.combo == 0 ? -value*0.0032 : event.combo == 1 ? -value*0.0014 : value*event.phi*(event.combo == 2 ? 0.92 : -0.22)
        }
        punch += (wanted-punch)*blend(wanted > punch ? 0.72 : 0.38)
        radius += (wantedRadius-radius)*blend(wantedRadius > radius ? 0.72 : 0.34)
        phi += (wantedPhi-phi)*blend(abs(wantedPhi) > abs(phi) ? 0.70 : 0.36)
    }
}
