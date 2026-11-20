import Foundation
import simd

/// Values from the pinned lyricMotionProfile; the display mode and movement
/// style are independent. Changing colors never rebuilds glyph masks.
struct NativeLyricStyle {
    let name: String
    let progressEase: Float, contextDrift: Float, edge: Float, sweep: Float, shimmer: Float, glow: Float, floatAmplitude: Float
    init(_ p: NativeParameters, karaoke: Bool = false) {
        name = p.raw["lyricMotionStyle"] as? String ?? "float"
        let values: [Float]
        switch name {
        case "smooth": values = [0.72,0.030,0.62,0.18,0.05,0.74,0.55]
        case "float": values = [0.66,0.120,1.04,0.36,0.14,1.16,1.45]
        case "shine": values = [1.02,0.052,1.42,1.22,0.34,1.30,0.82]
        case "glitch": values = [1.24,0.035,1.18,0.54,0.28,1.08+max(0,min(1.5,p.number("lyricGlitchIntensity",1)))*0.10,0.70]
        case "quick": values = [1.34,0.034,0.70,0.28,0.10,0.86,0.62]
        default: values = [0.90,0.066,1.18,0.72,0.22,1,1]
        }
        let softness = max(0.15,min(1.2,p.number("lyricMotionSoftness",0.72)))
        progressEase = max(0.08,min(0.72,(karaoke ? 0.34 : 0.18)*values[0]/max(0.35,softness)))
        contextDrift = values[1]; edge = values[2]; sweep = values[3]; shimmer = values[4]; glow = values[5]; floatAmplitude = values[6]
    }
}

struct NativeLyricEffects {
    private var random: UInt64 = 0x6c79726963
    private var nextAt: Float = 0, lastBeat: Float = -10, burst: Float = 0, hold: Float = 0
    private(set) var seed: Float = 0, pulse: Float = 0
    private mutating func next() -> Float {
        random = random &* 6364136223846793005 &+ 1
        return Float((random >> 32) & 0xffffff)/Float(0x1000000)
    }
    mutating func step(time: Float, dt: Float, beat: Float, parameters p: NativeParameters) -> SIMD2<Float> {
        guard p.raw["lyricMotionStyle"] as? String == "glitch" else { pulse = 0; burst = 0; return .zero }
        let rate = max(0.45,min(2.2,p.number("lyricGlitchRate",1))), jitter = max(0,min(1.8,p.number("lyricGlitchJitter",0.72)))
        let bound = p.flag("lyricGlitchCameraBind",true), drive = max(0,min(1.55,beat*1.12*(0.86+p.number("cinemaShake",0.5)*0.10)))
        let gate = bound ? max(0,min(1,(drive-0.14)/0.58)) : 1
        if nextAt == 0 { nextAt = time+(0.09+next()*0.34)/(0.62+rate*0.42) }
        if bound && drive > 0.22 && time-lastBeat > 0.055 {
            burst = max(burst,drive*gate*(0.52+jitter*0.18+next()*0.58))
            hold = max(hold,(0.014+next()*0.060)*(0.75+jitter*0.20)); seed = next()*997; lastBeat = time
        } else if !bound && time >= nextAt {
            burst = max(burst,0.18+next()*0.66); hold = 0.01+next()*0.04; seed = next()*997
            nextAt = time+(0.09+next()*0.34)/(0.62+rate*0.42)
        }
        hold = max(0,hold-dt); burst *= pow(hold > 0 ? 0.22 : 0.018,dt)
        pulse = max(0,min(1.95,(burst*gate+(bound ? drive*(0.16+jitter*0.045) : 0))*max(0,min(1.5,p.number("lyricGlitchIntensity",1)))))
        let slice = p.number("lyricGlitchSlice",0.72), scale = 0.50+jitter*0.62
        return SIMD2(sin(time*(61+rate*26)+seed)*(0.011+slice*0.023)*(0.65+pulse*0.55),cos(time*(37+rate*15)+seed*1.7)*(0.003+slice*0.005)*(0.55+pulse*0.45))*pulse*scale
    }
}
