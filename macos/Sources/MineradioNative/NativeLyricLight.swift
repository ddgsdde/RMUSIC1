import Foundation

/// Pinned lyric solar / beat envelopes, fed by the existing presentation tap.
struct NativeLyricLight {
    private var energy: Float = 0
    private(set) var beat: Float = 0, bloom: Float = 0, solar: Float = 0
    mutating func step(time: Float, dt: Float, energy input: Float, beat pulse: Float, parameters p: NativeParameters) {
        func blend(_ a: Float) -> Float { 1-pow(1-a,max(0,dt*60)) }
        let strength = p.flag("lyricGlow",true) ? max(0,min(0.85,p.number("lyricGlowStrength",0.28))) : 0
        energy += (input*0.25-energy)*blend(0.08)
        let target = p.flag("lyricGlowBeat",true) && strength > 0 ? pulse*1.22 : 0
        beat += (target-beat)*blend(target > beat ? 0.32 : 0.10)
        let drive = min(1.7,strength/0.5), breath = 0.5+0.5*sin(time*1.05)
        let wanted = strength > 0 ? max(0,min(1.45,(0.18+breath*0.16+max(energy,pulse*0.10)*0.90+beat*1.18+sin(time*0.37+1.2)*0.035)*drive)) : 0
        bloom += (wanted-bloom)*blend(wanted > bloom ? 0.075 : 0.050)
        solar += (bloom-solar)*blend(0.12)
    }
}
