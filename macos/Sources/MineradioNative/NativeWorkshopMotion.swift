import Foundation
import simd

private struct NativeWorkshopTrigger {
    var history = [Float](repeating:0,count:40)
    var index = 0, smoothed: Float = 0, previous: Float = 0, hold: Float = 0
    mutating func step(flux: Float, sensitivity: Float, cooldown: Float, strength: Float, dt: Float) -> Float? {
        smoothed += (flux-smoothed)*0.4
        history[index] = smoothed; index = (index+1)%40
        let average = history.reduce(0,+)/40
        let variance = history.reduce(0) { $0+pow($1-average,2) }/40
        let threshold = max(0.05,average+sqrt(variance)*max(0.1,5-sensitivity*4))
        defer { previous = smoothed }
        if hold > 0 { hold -= dt*60; return nil }
        guard previous > threshold, previous >= smoothed, previous-smoothed > 0.0001 else { return nil }
        hold = cooldown;return previous*2.2*strength
    }
}

/// The pinned Workshop's frequency bins, smoothing, spectral flux and idle wave.
/// Input is still the one existing playback FFT, expanded exactly as the old host.
struct NativeWorkshopMotion {
    private var previous = [Float](repeating:0,count:512)
    private var smoothed: [String:Float] = [:]
    private var previousBrightness: Float = 0, silence: Float = 0, idle: Float = 0
    private var pulse = NativeWorkshopTrigger(), meteor = NativeWorkshopTrigger()
    private var rings = [SIMD4<Float>](repeating:SIMD4(0,0,-100,0),count:10)
    private var cursor = 0, random: UInt64 = 0x776f726b
    private(set) var meteorStrength: Float?
    mutating func reset() { self = NativeWorkshopMotion() }
    private mutating func next() -> Float { random = random &* 6364136223846793005 &+ 1;return Float((random>>32)&0xffffff)/Float(0x1000000) }
    mutating func impact(_ impact: SIMD3<Float>, time: Float) { rings[cursor] = SIMD4(impact.x,impact.y,time,-impact.z*1.5);cursor = (cursor+1)%10 }
    mutating func step(_ bands: [Float], time: Float, dt: Float, gain: Float = 1) {
        var sums = [Float](repeating:0,count:8)
        var total: Float = 0, flux: Float = 0, pulseFlux: Float = 0, meteorFlux: Float = 0, centroid: Float = 0
        let edges = [4,12,24,45,81,120,180,255]
        for bin in 0..<512 {
            let value = max(0,min(1,bands[min(7,bin/64)]))*0.45*gain
            total += value;centroid += Float(bin)*value
            let change = value-previous[bin];flux += abs(change)
            if bin <= 12 { pulseFlux += max(0,change) }
            if (92...340).contains(bin) { meteorFlux += max(0,change) }
            previous[bin] = value
            if let band = edges.firstIndex(where:{bin <= $0}) { sums[band] += value }
        }
        let energy = total/512, brightness = total > 0 ? (sums[5]+sums[6]+sums[7])/total : 0
        let rawBands = zip(sums,[Float(5),9,13,21,37,40,61,76]).map(/)
        let density = Float(rawBands.filter { $0 > energy*1.5 }.count)/8
        let target: [String:Float] = ["uBass":(sums[0]+sums[1]+sums[2])/27,"uMid":(sums[3]+sums[4])/58,"uTreble":(sums[5]+sums[6]+sums[7])/177,"uEnergy":energy,"uSubBass":rawBands[0],"uLowMid":rawBands[2],"uHighMid":rawBands[4],"uPresence":rawBands[5],"uBrilliance":rawBands[6],"uAir":rawBands[7],"uWarmth":total > 0 ? (sums[0]+sums[1]+sums[2]+sums[3])/total : 0,"uBrightness":brightness,"uSharpness":max(0,brightness-previousBrightness)*10,"uSmoothness":max(0,1-flux/512*2),"uDensity":density,"uSpectralCentroid":total > 0 ? centroid/total : 0]
        previousBrightness = brightness
        let alpha = 1-pow(1-(total > 0 ? Float(0.15) : 0.08),dt*60)
        for (key,value) in target { let old = smoothed[key] ?? 0;smoothed[key] = old+(value-old)*alpha }
        if energy > 0.02 { silence = 0 } else { silence += dt }
        idle = max(0,min(1,idle+(silence >= 1 ? dt : -dt)))
        if let strength = pulse.step(flux:pulseFlux,sensitivity:0.05,cooldown:0,strength:0.25,dt:dt) {
            rings[cursor] = SIMD4((next()-0.5)*50,(next()-0.5)*50,time,min(strength*40,8));cursor = (cursor+1)%10
        }
        meteorStrength = meteor.step(flux:meteorFlux,sensitivity:0.3,cooldown:60,strength:0.5,dt:dt).map { min($0*20,4) }
    }
    func apply(to values: inout [String:[Float]], time: Float) {
        for (key,value) in smoothed { values[key] = [value] }
        values["uIdleWave"] = [idle]
        values["uRipples"] = rings.flatMap { [$0.x,$0.y,$0.z,abs($0.w),time-$0.z < 4.8 ? Float(1) : 0,$0.w < 0 ? Float(1) : 0,0,0] }
    }
}
