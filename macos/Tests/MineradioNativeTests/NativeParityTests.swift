import Foundation
import Metal
import Testing
import simd
@testable import MineradioNative

@Test @MainActor func coverImpulseTextureHasAuthoredVerticalLayoutAndBoundedLifetime() throws {
    var ripples = NativeCoverRipples()
    ripples.step(bass:0,dt:0.5);ripples.step(bass:0.8,dt:1/60)
    #expect((2...3).contains(ripples.count))
    let device = try #require(MTLCreateSystemDefaultDevice())
    let prepared = ripples.texture(device:device)
    let texture = try #require(prepared)
    #expect(texture.width == 1 && texture.height == 12 && texture.pixelFormat == .rgba32Float)
    var pixels = [Float](repeating:0,count:48)
    pixels.withUnsafeMutableBytes { texture.getBytes($0.baseAddress!,bytesPerRow:16,from:MTLRegionMake2D(0,0,1,12),mipmapLevel:0) }
    #expect((0..<ripples.count).allSatisfy { pixels[$0*4+3] > 0.65 })
    for _ in 0..<121 { ripples.step(bass:0,dt:1/60) }
    #expect(ripples.count == 0)
    for _ in 0..<100 { ripples.step(bass:0,dt:0.4);ripples.step(bass:1,dt:0.01) }
    #expect(ripples.values.count == 12 && ripples.count <= 12)
    ripples.reset();#expect(ripples.count == 0)
}

@Test func nativeEulerCompositionMatchesPinnedThreeXYZGolden() {
    // three.r128: new Matrix4().makeRotationFromEuler(new Euler(.3,.5,.7)).
    let golden: [Float] = [0.671212166,0.723807454,-0.159928099,0,-0.565354208,0.639408930,0.521086211,0,0.479425539,-0.259343380,0.838386644,0,0,0,0,1]
    let matrix = nativeTransform(rotation:SIMD3(0.3,0.5,0.7))
    for column in 0..<4 { for row in 0..<4 { #expect(abs(matrix[column][row]-golden[column*4+row]) < 0.000001) } }
}

@Test func workshopAudioMatchesPinnedEngineGoldenAndSilenceRemainsAnimated() {
    var workshop = NativeWorkshopMotion()
    workshop.step([0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8],time:1,dt:1/60)
    var values: [String:[Float]] = [:];workshop.apply(to:&values,time:1)
    // eG.getAudioData(1/60), with the old host's 512-bin expansion, from the pinned bundle.
    let golden: [String:Float] = ["uBass":0.00625,"uMid":0.008728448,"uTreble":0.020593220,"uEnergy":0.030375,"uSubBass":0.00675,"uLowMid":0.006230769,"uHighMid":0.009851351,"uPresence":0.0131625,"uBrilliance":0.019143443,"uAir":0.025667763,"uWarmth":0.002994792,"uBrightness":0.03515625,"uSharpness":0.3515625,"uSmoothness":0.08925,"uDensity":0,"uSpectralCentroid":49.525]
    // JavaScript accumulates in f64; the native visual state uses bounded f32.
    for (key,value) in golden { #expect(abs(values[key]![0]-value) < max(1,abs(value))*0.000005) }
    for i in 0..<300 { workshop.step(Array(repeating:0,count:8),time:1+Float(i)/60,dt:1/60) }
    workshop.apply(to:&values,time:6)
    #expect(values["uEnergy"]![0] < 0.00001 && values["uIdleWave"] == [1])
    #expect(values["uRipples"]?.count == 80)
}

@Test func cameraUsesSourceTimeImpulsesAndClearsThemOnPauseOrReset() {
    var timeline = NativeFeatureTimeline(), cinema = NativeCinemaMotion()
    timeline.push([NativeVisualFrame(session:"one",position:1,bands:[0.8,0.7,0,0,0,0,0,0],energy:0.8)],session:"one")
    timeline.step(position:1,playing:true,dt:1/60)
    cinema.step(timeline,position:1,playing:true,dt:1/60)
    #expect(cinema.punch > 0 && cinema.radius > 0 && cinema.eventCount == 1)
    timeline.step(position:2,playing:true,dt:1/60)
    cinema.step(timeline,position:2,playing:true,dt:1/60)
    #expect(cinema.eventCount == 0)
    for _ in 0..<120 { cinema.step(timeline,position:2,playing:false,dt:1/60) }
    #expect(cinema.punch < 0.00001 && cinema.radius < 0.00001)
    cinema.reset();#expect(cinema.eventCount == 0 && cinema.punch == 0)
}

@Test func allLyricStylesKeepAuthoredShaderControlsAndKaraokeIsNotDelayed() {
    let golden: [(String,Float,Float,Float)] = [("smooth",0.62,0.18,0.05),("float",1.04,0.36,0.14),("shine",1.42,1.22,0.34),("glitch",1.18,0.54,0.28),("quick",0.70,0.28,0.10),("standard",1.18,0.72,0.22)]
    for (name,edge,sweep,shimmer) in golden {
        let style = NativeLyricStyle(NativeParameters("{\"lyricMotionStyle\":\"\(name)\"}"))
        #expect(style.edge == edge && style.sweep == sweep && style.shimmer == shimmer)
        var motion = NativeLyricMotion()
        _ = motion.wordProgress(key:0,target:0,karaoke:false,style:style,dt:1/60)
        let eased = motion.wordProgress(key:0,target:0.4,karaoke:false,style:style,dt:1/60)
        #expect(eased > 0 && eased < 0.4)
        #expect(motion.wordProgress(key:0,target:0.75,karaoke:true,style:style,dt:1/60) == 0.75)
    }
}

@Test func workshopCustomPaletteMatchesPinnedMRcAndCssBackdropWithFullGridExtent() {
    var state = NativeStageState(preset:8,fps:60,density:118,scale:1,intensity:0.85,active:true,visible:true,immersive:true,parameters:"{}",position:0,playing:true,cover:nil)
    state.parameters = ##"{"sonicWorkshopColorMode":"custom","sonicWorkshopCustomColor":"#808080","sonicWorkshopCoolColorMode":"custom","sonicWorkshopCoolColor":"#808080"}"##
    let values = nativeUniforms(state:state,parameters:NativeParameters(state.parameters),timeline:NativeFeatureTimeline(),time:1,pixel:1,hasCover:false,tint:SIMD3(repeating:1))
    #expect(abs(values["uCoolCore"]![0]-128/255) < 0.00001)
    let backdrop = nativeWorkshopBackdrop(SIMD3(0,0.5,1))
    #expect(backdrop == SIMD3<Float>(0,188/255,1))
    #expect(values["uHalfExtent"] == [84] && abs(values["uAudioIntensity"]![0]-0.85) < 0.00001)
}

@Test func translationVisibilityDoesNotZoomTheLyricProjectionEnvelope() {
    let lines = [NativeLyricLine(time:0,text:"Follow the rhythm",translation:"跟随节奏",words:[])]
    let visible = NativeLyricLayout(lines:lines,parameters:NativeParameters(#"{"lyricTranslationMode":"multi"}"#))
    let hidden = NativeLyricLayout(lines:lines,parameters:NativeParameters(#"{"lyricTranslationMode":"off"}"#))
    #expect(visible.envelopeSlot == hidden.envelopeSlot)
    #expect(visible.primary[1] > hidden.primary[1])
}

@Test func lyricBeatLightHonorsItsSwitchAndDecaysWithoutNewAudio() {
    var on = NativeLyricLight(), off = NativeLyricLight()
    for i in 0..<60 {
        on.step(time:Float(i)/60,dt:1/60,energy:0.5,beat:0.8,parameters:NativeParameters("{}"))
        off.step(time:Float(i)/60,dt:1/60,energy:0.5,beat:0.8,parameters:NativeParameters(#"{"lyricGlowBeat":false}"#))
    }
    #expect(on.beat > 0.8 && off.beat == 0 && on.solar > off.solar)
    for i in 0..<240 { on.step(time:1+Float(i)/60,dt:1/60,energy:0,beat:0,parameters:NativeParameters(#"{"lyricGlow":false}"#)) }
    #expect(on.solar < 0.0001 && on.beat < 0.0001)
}

@Test @MainActor func lyricSparkBuffersStayBoundedAndReleaseWhenDisabled() throws {
    let device = try #require(MTLCreateSystemDefaultDevice()), program = try NativeProgram(device:device,name:"lyricSpark",additive:true)
    let sparks = NativeLyricSparks()
    #expect(sparks.bufferCount == 0)
    for i in 0..<120 {
        let buffer = sparks.prepare(device:device,program:program,width:6,height:0.9,time:Float(i)/60,beat:0.5,bass:0.4,mid:0.2)
        #expect(buffer?.length == 132*program.stride)
        #expect(sparks.bufferCount == 2)
    }
    sparks.close();#expect(sparks.bufferCount == 0)
}
