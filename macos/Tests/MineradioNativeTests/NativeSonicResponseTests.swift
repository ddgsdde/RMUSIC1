import Testing
import Foundation
@testable import MineradioNative

@Test func sonicMaterialAndHeightUseTheSameAuthoredResponsePhase() {
    var timeline = NativeFeatureTimeline(), sonic = NativeSonicMotion()
    let p = NativeParameters("{}")
    timeline.push([NativeVisualFrame(session:"a",position:1,bands:[0.2,0.3,0.4,0.1,0.6,0.7,0.8,0.9],energy:0.6)],session:"a")
    timeline.step(position:1,playing:true,dt:1/60); sonic.step(timeline,parameters:p,dt:1/60)
    var values: [String:[Float]] = [:]; sonic.applyAudioUniforms(to:&values)
    // Pinned sonic-topography.js equations, evaluated with the fresh source-time kick (0.75).
    // New onsets are no longer aged by a render frame before their first presentation.
    let golden: [String:Float] = ["uWarmth":0.71598182,"uBrightness":0.28401818,"uEnergy":0.661875,"uSmoothness":0.83010203,"uDensity":0.67273735,"uSharpness":0.44547474]
    for (key,value) in golden { #expect(abs(values[key]![0]-value) < 0.00001) }
}

@Test func playbackReconciliationClearsReactionsWithoutRewindingTheScene() {
    var timeline = NativeFeatureTimeline(), sonic = NativeSonicMotion()
    let p = NativeParameters("{}")
    timeline.push([NativeVisualFrame(session:"a",position:1,bands:Array(repeating:0.8,count:8),energy:0.8)],session:"a")
    timeline.step(position:1,playing:true,dt:1/60)
    for _ in 0..<60 { sonic.step(timeline,parameters:p,dt:1/60) }
    let time = sonic.time, yaw = sonic.yaw
    sonic.resetAudio()
    #expect(sonic.time == time && sonic.yaw == yaw)
    #expect(sonic.bands.allSatisfy { $0 == 0 } && sonic.pulse == 0)
    #expect(stride(from:3,to:40,by:4).allSatisfy { sonic.rippleValues(workshop:false)[$0] == 0 })
    timeline.reset(); sonic.step(timeline,parameters:p,dt:1/60)
    #expect(sonic.time > time && sonic.yaw > yaw)
}

@Test func trebleRipplesKeepTheAuthoredProbabilityAndNaturalSpread() {
    var timeline = NativeFeatureTimeline(), sonic = NativeSonicMotion()
    let p = NativeParameters("{}")
    var position = 0.0, radii: [Int] = []
    for _ in 0..<100 {
        let begin = sonic.time
        for _ in 0..<30 {
            position += 1.0/60.0
            timeline.push([NativeVisualFrame(session:"a",position:position,bands:[0,0,0,0,0,1,1,0],energy:0.6)],session:"a")
            timeline.step(position:position,playing:true,dt:1/60); sonic.step(timeline,parameters:p,dt:1/60)
        }
        let rows = sonic.rippleValues(workshop:false)
        for i in stride(from:0,to:40,by:4) where rows[i+2] >= begin && rows[i+3] < 0 {
            let radius = hypot(rows[i],rows[i+1])
            #expect(radius >= 9.99 && radius <= 45.01)
            radii.append(Int(radius.rounded()))
        }
        for _ in 0..<30 {
            position += 1.0/60.0
            timeline.push([NativeVisualFrame(session:"a",position:position,bands:Array(repeating:0,count:8),energy:0)],session:"a")
            timeline.step(position:position,playing:true,dt:1/60); sonic.step(timeline,parameters:p,dt:1/60)
        }
    }
    print("Treble onsets emitted \(radii.count)/100 ripples across \(Set(radii).count) radius buckets")
    #expect((40...70).contains(radii.count) && Set(radii).count > 10)
}
