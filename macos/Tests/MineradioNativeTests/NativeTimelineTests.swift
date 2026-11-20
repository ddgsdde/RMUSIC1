import Foundation
import Metal
import Testing
@testable import MineradioNative

@Test func shortAudioOnsetsSurviveBatchesAndMatchAtDifferentDrawRates() {
    let batch = [0.0,0.04,0.08].enumerated().map { i, t in
        NativeVisualFrame(session: "p", position: t, bands: Array(repeating: i == 1 ? 0.8 : 0, count: 8), energy: i == 1 ? 0.8 : 0)
    }
    var fast = NativeFeatureTimeline(), slow = NativeFeatureTimeline()
    fast.push(batch, session: "p"); slow.push(batch, session: "p")
    for i in 0...12 { fast.step(position: Double(i) / 120, playing: true, dt: 1/120) }
    slow.step(position: 0.1, playing: true, dt: 0.1)
    #expect(abs(fast.beat - exp(-0.06 * 5)) < 0.00001)
    #expect(abs(fast.beat - slow.beat) < 0.00001)
    slow.step(position: 0.11, playing: true, dt: 0.01)
    #expect(slow.beat < fast.beat) // retained frame cannot retrigger a consumed onset
}

@Test func nativeFeatureClockNeverConsumesFutureAudioAndPauseReleasesQueuedFrames() {
    var timeline = NativeFeatureTimeline()
    timeline.push([NativeVisualFrame(session:"old",position:1,bands:Array(repeating:1,count:8),energy:1),NativeVisualFrame(session:"p",position:1.1,bands:Array(repeating:0.6,count:8),energy:0.8)],session:"p")
    timeline.step(position:1.05,playing:true,dt:1/60)
    #expect(timeline.bands.allSatisfy { $0 == 0 })
    timeline.step(position:1.1,playing:true,dt:1/60)
    #expect(timeline.bands.allSatisfy { $0 > 0 })
    #expect(timeline.energy == 0.8)
    timeline.pause()
    for _ in 0..<120 { timeline.step(position:1.1,playing:false,dt:1/60) }
    #expect(timeline.energy == 0 && timeline.bands.allSatisfy { $0 < 0.00001 })
    timeline.step(position:1.1,playing:true,dt:1/60)
    #expect(timeline.energy == 0)
}

@Test func nativeCameraMovementIsIndependentOfPlaybackAndUsesElapsedTime() {
    var fast = NativeCamera(), slow = NativeCamera()
    fast.drag(x:120,y:60); slow.drag(x:120,y:60)
    fast.zoom(-100); slow.zoom(-100)
    for _ in 0..<60 { fast.step(1/60) }
    for _ in 0..<24 { slow.step(1/24) }
    let a = fast.view(), b = slow.view()
    for i in 0..<4 { for j in 0..<4 { #expect(abs(a[i][j]-b[i][j]) < 0.0001) } }
    for preset in 0...12 { fast.reset(preset:preset); let matrix = fast.view(); for i in 0..<4 { for j in 0..<4 { #expect(matrix[i][j].isFinite) } } }
}

@Test(arguments: [1, 4]) @MainActor func allPinnedNativeShaderProgramsBuildAgainstTheActualMetalDevice(sampleCount: Int) throws {
    let device = try #require(MTLCreateSystemDefaultDevice())
    for name in ["cover","bloom","stars","skull","terrain","floating","workshop","lyrics","lyricGlow","shelf","sonicParticle","backCover","lyricSpark"] {
        let program = try NativeProgram(device:device,name:name,additive:["bloom","stars","skull"].contains(name),sampleCount:sampleCount)
        #expect(program.byteCount > 0 && program.byteCount <= 4096)
        #expect(program.members["projectionMatrix"] != nil)
    }
}

@Test func nativeGlyphMasksAreNotRebuiltForColorOrPlaybackUpdates() {
    let a = NativeParameters(##"{"lyricWeight":750,"uiAccentColor":"#ffffff"}"##)
    let color = NativeParameters(##"{"lyricWeight":750,"uiAccentColor":"#000000","lyricHighlightColor":"#ff0000"}"##)
    let font = NativeParameters(##"{"lyricWeight":400,"uiAccentColor":"#ffffff"}"##)
    #expect(a.lyricRasterKey == color.lyricRasterKey)
    #expect(a.lyricRasterKey != font.lyricRasterKey)
}
