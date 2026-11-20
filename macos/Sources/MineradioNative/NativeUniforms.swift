import simd

/// The bundled Workshop's MRc constructs Color(r,g,b), then getHexString()
/// encodes that working value for its CSS backdrop. Shader uniforms stay raw.
func nativeWorkshopBackdrop(_ color: SIMD3<Float>) -> SIMD3<Float> {
    func channel(_ input: Float) -> Float {
        let x = max(0,min(1,input))
        let encoded = x <= 0.0031308 ? x*12.92 : 1.055*pow(x,1/2.4)-0.055
        return (encoded*255).rounded()/255
    }
    return SIMD3(channel(color.x),channel(color.y),channel(color.z))
}

/// backgroundStarRiverTargetAlpha() from the pinned visual source, per preset.
func nativeStarRiverAlpha(preset: Int, enabled: Bool) -> Float {
    guard enabled else { return 0 }
    switch preset {
    case 5,7: return 0
    case 8: return 0.28
    case 6: return 0.38
    case 9: return 0.12
    case 10: return 0.18
    case 11: return 0.10
    case 12: return 0.16
    default: return 0.34
    }
}

/// Algorithm parameters retain the names of the pinned shaders and the existing visual console.
func nativeUniforms(state: NativeStageState, parameters p: NativeParameters, timeline: NativeFeatureTimeline, time: Float, pixel: Float, hasCover: Bool, tint: SIMD3<Float>, palette: NativeCoverPalette = NativeCoverPalette(), hasDepth: Bool = false, coverBlend: Float = 1) -> [String: [Float]] {
    let bands = timeline.bands, bass = (bands[0] + bands[1]) / 2, mid = (bands[2] + bands[3] + bands[4]) / 3, treble = (bands[5] + bands[6] + bands[7]) / 3
    var v: [String: [Float]] = [
        "uTime": [time], "uPreset": [Float(state.preset)], "uAlpha": [1], "uOpacity": [1], "uPixel": [pixel], "uIntensity": [Float(state.intensity)],
        "uBass": [bass], "uMid": [mid], "uTreble": [treble], "uBeat": [timeline.beat], "uEnergy": [timeline.energy],
        "uDepth": [p.number("depth", 0.2)], "uPointScale": [p.number("point", 1)], "uSpeed": [p.number("speed", 1)], "uTwist": [p.number("twist", 0)],
        "uColorBoost": [p.number("color", 1.1)], "uScatter": [p.number("scatter", 0)], "uCoverRes": [max(80/118, min(180/118,p.number("coverResolution",1.55)))],
        "uBgFade": [p.number("bgFade",0.2)], "uHasCover": [hasCover ? 1 : 0], "uHasDepth": [hasDepth ? 1 : 0], "uAiBoost": [hasDepth ? 0.55 : 0], "uEdgeEnabled": [p.flag("edge",false) ? 1 : 0], "uColorMixT": [coverBlend*coverBlend*(3-2*coverBlend)], "uVinylSpin": [time * 0.14],
        "uBloomStrength": [p.flag("bloom",false) ? p.number("bloomStrength",0.62) : 0], "uBloomSize": [2.65], "uParticleDim": [1],
        "uBackdropAdapt": [p.flag("coverBackdropAdapt",true) ? p.number("lyricBackgroundAdapt",0.72) : 0],
        "uTintColor": [tint.x,tint.y,tint.z], "uTintStrength": [p.raw["visualTintMode"] as? String == "custom" ? 0.42 : 0],
        "uMouseXY": [-999,-999], "uHandXY": [-999,-999],
        "uJawOpen": [bass * 0.3 + mid * 0.35], "uSkullFlash": [timeline.beat], "uColorA": [0.72,0.68,0.60], "uColorB": [1,0.96,0.85], "uShadow": [0.063,0.051,0.051], "uLight": [1,0.89,0.63]
    ]
    let amplitude = p.number("sonicGroundAmplitude",50)
    v["uAmplitude"] = [amplitude <= 50 ? amplitude / 50 : 1 + pow((amplitude - 50)/50,2)*14]
    v["uIdleWave"] = [1]; v["uAudioIntensity"] = [p.number("sonicWorkshopAudioIntensity",1.15)]
    v["uResponseRange"] = [p.number("sonicWorkshopResponseRange",1.3)]; v["uHalfExtent"] = [70]
    v["uPeakIntensity"] = [p.number("sonicWorkshopPeakIntensity",0.62)]; v["uPeakEnabled"] = [1]
    v["uGlintStrength"] = [max(0,min(100,p.number("sonicGroundGlints",22)))/100]
    for (i,name) in ["SubBass","Bass","LowMid","Mid","HighMid","Presence","Brilliance","Air"].enumerated() { if state.preset == 7 || state.preset == 8 { v["u"+name] = [bands[i] * p.number("sonicGround"+name,50)/50] } }
    let low = bands[0]+bands[1]+bands[2]+bands[3], high = bands[5]+bands[6]+bands[7], sum = max(0.001,low+high)
    for (name, value) in [("Smoothness",min(1,1-treble*0.42+mid*0.14)),("Density",min(1,0.45+treble*0.35+timeline.beat*0.10)),("Warmth",low/sum),("Brightness",high/sum),("Sharpness",min(1,treble*0.70+timeline.beat*0.20))] { v["u"+name] = [value] }
    func put(_ name: String, _ c: SIMD3<Float>) { v[name] = [c.x,c.y,c.z] }
    func mix(_ a: SIMD3<Float>, _ b: SIMD3<Float>, _ t: Float) -> SIMD3<Float> { a+(b-a)*t }
    let white = SIMD3<Float>(repeating:1)
    if state.preset == 8 {
        let custom = p.raw["visualTintMode"] as? String == "custom"
        let primary = custom ? tint : p.raw["sonicWorkshopColorMode"] as? String == "custom" ? p.color("sonicWorkshopCustomColor",palette.primary) : palette.primary
        func region(_ key: String, _ fallback: SIMD3<Float>, defaultMode: String = "cover") -> SIMD3<Float> {
            custom ? primary : (p.raw[key+"Mode"] as? String ?? defaultMode) == "custom" ? p.color(key,fallback) : fallback
        }
        let base = region("sonicWorkshopBaseColor",palette.base)
        let cool = region("sonicWorkshopCoolColor",palette.cool,defaultMode:"custom")
        let actualCool = !custom && (p.raw["sonicWorkshopCoolColorMode"] as? String ?? "custom") == "custom" ? p.color("sonicWorkshopCoolColor",white) : cool
        let warm = region("sonicWorkshopWarmColor",palette.warm)
        put("uBaseColor1",base*0.2); put("uBaseColor2",mix(base,primary,0.22))
        put("uCoolCore",actualCool); put("uCoolEdge",actualCool*0.48)
        put("uWarmCore",warm); put("uWarmEdge",warm*0.56)
        put("uRippleColor",region("sonicWorkshopRippleColor",palette.light)); put("uPeakColor",region("sonicWorkshopPeakColor",palette.accent))
        v["uGlowIntensity"] = [0.8]
        // MRc in the pinned custom-theme bridge intentionally passes normalized
        // components to Color(r,g,b), rather than constructing Color(hex).
        v["uAudioIntensity"] = [Float(state.intensity)*p.number("sonicWorkshopAudioIntensity",1.15)/1.15]
        v["uHalfExtent"] = [84]
    } else {
        let custom = p.raw["sonicGroundColorMode"] as? String == "custom"
        // sonicCoverGroundTheme uses coverPalette.primary (the authored display tone).
        // rawAreaPrimary belongs to the Workshop regions, not this ground material.
        let base = custom ? p.color("sonicGroundBaseColor",SIMD3(0.02,0.027,0.047)) : mix(palette.lyric,SIMD3(5.0/255,7.0/255,12.0/255),0.84)
        let cool = custom ? p.color("sonicGroundCoolColor",SIMD3(0,0.4,1)) : mix(palette.lyric,white,0.08)
        let warm = custom ? p.color("sonicGroundWarmColor",SIMD3(1,0.235,0.098)) : mix(palette.secondary,SIMD3(1,0.694,0.353),0.18)
        put("uBaseColor1",base); put("uBaseColor2",mix(base,custom ? white : palette.highlight,custom ? 0.12 : 0.14))
        put("uCoolCore",cool); put("uCoolEdge",mix(cool,base,0.34))
        put("uWarmCore",warm); put("uWarmEdge",mix(warm,base,0.26))
        put("uRippleColor",custom ? p.color("sonicGroundAccentColor",SIMD3(0.2,0.9,1)) : mix(palette.highlight,white,0.10))
        put("uPeakColor",palette.accent)
        v["uGlowIntensity"] = [min(2.2,max(0.45,0.55+p.number("sonicGroundGlow",20)*0.014+p.number("bloomStrength",0.62)*0.22))]
    }
    v["uFogColor"] = v["uBaseColor1"]
    return v
}
