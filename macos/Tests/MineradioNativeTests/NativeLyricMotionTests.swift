import Testing
import simd
@testable import MineradioNative

@Test func lyricBoundaryCorrectionDoesNotReverseTheTransitionButSeekStillWorks() {
    let lines = [0.0,5,10].map { NativeLyricLine(time:$0,text:"line",translation:nil,words:[]) }
    var cursor = NativeLyricCursor()
    let indices = [4.99,5.01,4.98,5.02].map { cursor.select(lines:lines,position:$0,playing:true) }
    #expect(indices == [0,1,1,1])
    #expect(cursor.select(lines:lines,position:4.7,playing:true) == 0)
    #expect(cursor.select(lines:lines,position:10.1,playing:true) == 2)
    #expect(cursor.select(lines:lines,position:9.98,playing:false) == 1)
    cursor.reset()
    #expect(cursor.select(lines:lines,position:4.98,playing:true) == 0)
}

@Test func lyricMotionMatchesPinnedTwoStageTrackAndDoesNotSnapAcrossLines() {
    var motion = NativeLyricMotion()
    motion.step(target:0,slot:1,dt:1/60)
    _ = motion.pose(key:0,y:0,z:0,scale:1,opacity:1,active:true,slot:1,lineStep:1,dt:1/60)
    let golden: [(Float,Float)] = [(0.1566,0.021141),(0.28867644,0.05725828),(0.40006971,0.10353783),(0.49401879,0.15625276)]
    for value in golden {
        motion.step(target:1,slot:1,dt:1/60)
        let pose = motion.pose(key:0,y:motion.offset!,z:0,scale:1,opacity:1,active:false,slot:1,lineStep:1,dt:1/60)
        #expect(abs(motion.offset!-value.0) < 0.00001)
        #expect(abs(pose.y-value.1) < 0.00001)
    }
    let old = motion.offset!
    motion.step(target:100,slot:2,dt:1/60)
    #expect(motion.offset!-old <= 1.36001)
    // Prewarming another glyph does not replace the shared track phase.
    _ = motion.pose(key:1,y:0,z:0,scale:1,opacity:0,active:false,slot:1,lineStep:1,dt:1/60)
    motion.retain([1]); #expect(motion.count == 1 && motion.offset! > old)
    motion.reset(); #expect(motion.offset == nil && motion.count == 0)
}

@Test func lyricTrackDurationIsStableAtConfiguredFrameRates() {
    func simulate(_ fps: Float) -> Float {
        var motion = NativeLyricMotion()
        motion.step(target:0,slot:2,dt:1/fps)
        _ = motion.pose(key:0,y:0,z:0,scale:1,opacity:1,active:true,slot:2,lineStep:1,dt:1/fps)
        var y: Float = 0
        for _ in 0..<Int(fps) {
            motion.step(target:2,slot:2,dt:1/fps)
            y = motion.pose(key:0,y:motion.offset!,z:0,scale:1,opacity:1,active:true,slot:2,lineStep:1,dt:1/fps).y
        }
        return y
    }
    #expect(abs(simulate(24)-simulate(60)) < 0.002)
    #expect(abs(simulate(30)-simulate(60)) < 0.002)
}

@Test func nativeStarLayerMatchesAuthoredPresetComposition() {
    let values: [Float] = [0.34,0.34,0.34,0.34,0.34,0,0.38,0,0.28,0.12,0.18,0.10,0.16]
    for (preset,value) in values.enumerated() {
        #expect(nativeStarRiverAlpha(preset:preset,enabled:true) == value)
        #expect(nativeStarRiverAlpha(preset:preset,enabled:false) == 0)
    }
}

@Test func sonicGroundUsesThePinnedDisplayPaletteAndWorkshopKeepsAreaPigments() {
    var palette = NativeCoverPalette()
    palette.primary = SIMD3(0.18,0.08,0.32)
    palette.lyric = SIMD3(0.9,0.95,1)
    let state = NativeStageState(preset:7,fps:60,density:118,scale:1,intensity:0.85,active:true,visible:true,immersive:true,parameters:"{}",position:0,playing:true,cover:nil)
    let values = nativeUniforms(state:state,parameters:NativeParameters("{}"),timeline:NativeFeatureTimeline(),time:1,pixel:1,hasCover:true,tint:palette.primary,palette:palette)
    // sonicCoverGroundTheme reads palette.primary, whose native equivalent is lyric.
    let base = palette.lyric*0.16+SIMD3<Float>(5,7,12)/255*0.84
    let cool = palette.lyric*0.92+SIMD3<Float>(repeating:0.08)
    for i in 0..<3 {
        #expect(abs(values["uBaseColor1"]![i]-base[i]) < 0.00001)
        #expect(abs(values["uCoolCore"]![i]-cool[i]) < 0.00001)
    }
}

@Test func sonicMaterialKeepsThePinnedColorTransitionAcrossCoverChanges() {
    var theme = NativeSonicTheme(), first: [String:[Float]] = ["uBaseColor1":[0.5,0.5,0.5]]
    theme.apply(to:&first,dt:1/60)
    #expect(abs(first["uBaseColor1"]![0]-0.0345) < 0.00001)
    var next: [String:[Float]] = ["uBaseColor1":[0,0,0]]
    theme.apply(to:&next,dt:1/60)
    #expect(abs(next["uBaseColor1"]![0]-0.032775) < 0.00001)
}
