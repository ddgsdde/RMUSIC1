import Foundation
import Metal
import Testing
import simd
@testable import MineradioNative

@MainActor private func finish(_ ready: () -> Bool) async throws {
    let deadline = ContinuousClock.now.advanced(by:.seconds(5))
    while !ready(), ContinuousClock.now < deadline { try await Task.sleep(for:.milliseconds(10)) }
    #expect(ready())
}

@Test @MainActor func lyricWindowReusesGlyphsAcrossLinesAndReleasesPendingWork() async throws {
    let device = try #require(MTLCreateSystemDefaultDevice()), program = try NativeProgram(device:device,name:"lyrics")
    var lyrics: NativeLyrics? = NativeLyrics(device:device)
    weak var released = lyrics
    let lines = (0..<80).map { NativeLyricLine(time:Double($0)*3,text:"Line \($0) — memory stays bounded",translation:"第 \($0) 行歌词",words:[]) }
    let p = NativeParameters("{}")
    _ = lyrics?.prepare(lines:lines,position:30,session:"same",parameters:p,settings:p.lyricRasterKey,program:program)
    try await finish { lyrics?.pending == false }
    let first = try #require(lyrics?.rasterizedCount)
    #expect(first <= 16)
    _ = lyrics?.prepare(lines:lines,position:33,session:"same",parameters:p,settings:p.lyricRasterKey,program:program)
    try await finish { lyrics?.pending == false }
    #expect(lyrics!.rasterizedCount-first == 2)
    for i in stride(from:15,to:76,by:5) {
        _ = lyrics?.prepare(lines:lines,position:Double(i)*3,session:"same",parameters:p,settings:p.lyricRasterKey,program:program)
        try await finish { lyrics?.pending == false }
        #expect(lyrics!.textureCount <= 16)
    }
    _ = lyrics?.prepare(lines:lines,position:30,session:"new",parameters:p,settings:p.lyricRasterKey,program:program)
    lyrics?.close(); #expect(lyrics?.textureCount == 0 && lyrics?.pending == false)
    lyrics = nil
    try await Task.sleep(for:.milliseconds(30)); #expect(released == nil)
}

@Test @MainActor func shelfCardsStayBoundedAndCloseCannotPublishStaleTextures() async throws {
    let device = try #require(MTLCreateSystemDefaultDevice())
    var shelf: NativeShelf? = NativeShelf(device:device)
    weak var released = shelf
    var state = NativeStageState(preset:7,fps:60,density:118,scale:1,intensity:0.85,active:true,visible:true,immersive:true,parameters:"{}",position:0,playing:true,cover:nil)
    state.shelf = true
    state.queue = (0..<50).map { NativeQueueItem(id:"track-\($0)",title:"Shelf \($0)",artist:"Synthetic",cover:"",index:$0+100) }
    for index in [100,110,130,149] {
        state.queueIndex = index; shelf?.update(state,parameters:NativeParameters("{}"),active:true)
        try await finish { shelf?.pending == false }
        #expect(shelf?.textureCount == 11)
    }
    state.queueIndex = 100; shelf?.update(state,parameters:NativeParameters("{}"),active:true)
    shelf?.close(); #expect(shelf?.textureCount == 0 && shelf?.pending == false)
    shelf = nil
    try await Task.sleep(for:.milliseconds(30)); #expect(released == nil)
}

@Test func shelfProjectionPickingAndVisibleWindowMatchCoreIndices() {
    #expect(NativeShelfLayout.window(center:0,total:50) == 0..<11)
    #expect(NativeShelfLayout.window(center:49,total:50) == 39..<50)
    let p = NativeParameters("{}"), camera = NativeCamera()
    let layout = NativeShelfLayout(size:SIMD2(1360,860),parameters:p,preset:7)
    let model = layout.model(delta:0,index:3,time:1,reveal:1,hover:0)
    let projection = NativeShelfLayout.fittedProjection(camera.projection(aspect:1360/860),focus:camera.view()*model)
    let matrix = projection*camera.view()*model
    let center = matrix*SIMD4<Float>(0,0,0,1)
    let item = NativeQueueItem(id:"identity",title:"Title",artist:"Artist",cover:"",index:103)
    let hit = NativeShelfHit(item:item,inverse:simd_inverse(matrix))
    #expect(hit.contains(SIMD2(center.x,center.y)/center.w))
    #expect(!hit.contains(SIMD2(-0.95,0.9)))
    #expect(hit.item.id == "identity" && hit.item.index == 103)
}

@Test func bilingualLyricSlotsMatchPinnedDefaultsAndDoNotOverlap() {
    let lines = (0..<12).map { NativeLyricLine(time:Double($0)*2,text:"Move with the sound",translation:"跟随音乐",words:[]) }
    let layout = NativeLyricLayout(lines:lines)
    #expect(abs(layout.translationGap-1.2104) < 0.0001)
    #expect(abs(layout.primary[1]-2.1214) < 0.0001)
    let primary = layout.y(index:5,translation:false,displayed:5)
    let translated = layout.y(index:5,translation:true,displayed:5)
    let next = layout.y(index:6,translation:false,displayed:5)
    #expect(primary-translated > (0.60+0.36)/2)
    #expect(translated-next > (0.60+0.36)/2)
    let hidden = NativeLyricLayout(lines:lines,parameters:NativeParameters(#"{"lyricTranslationMode":"off"}"#))
    #expect(hidden.primary[1] == 1)
}

@Test func sonicRipplesComeFromMeasuredFeatureOnsetsAndDecayWithinBoundedRing() {
    var timeline = NativeFeatureTimeline(), sonic = NativeSonicMotion()
    let p = NativeParameters("{}")
    timeline.push([NativeVisualFrame(session:"a",position:1,bands:Array(repeating:0.8,count:8),energy:0.8)],session:"a")
    timeline.step(position:1,playing:true,dt:1/60); sonic.step(timeline,parameters:p,dt:1/60)
    let ripple = sonic.rippleValues(workshop:false)
    #expect(ripple.count == 40 && stride(from:3,to:40,by:4).contains { abs(ripple[$0]) > 0 })
    #expect(sonic.rippleValues(workshop:true).count == 80)
    timeline.pause()
    for _ in 0..<400 { timeline.step(position:1,playing:false,dt:1/60); sonic.step(timeline,parameters:p,dt:1/60) }
    let silent = sonic.rippleValues(workshop:false)
    #expect(stride(from:3,to:40,by:4).allSatisfy { silent[$0] == 0 })
    sonic.close(); #expect(sonic.time == 0)
}

@Test @MainActor func residentGlyphAndGlowActuallyRenderOnMetalWithCompatibleLayouts() async throws {
    let device = try #require(MTLCreateSystemDefaultDevice()), queue = try #require(device.makeCommandQueue())
    let text = try NativeProgram(device:device,name:"lyrics"), glow = try NativeProgram(device:device,name:"lyricGlow",additive:true)
    #expect(text.stride == glow.stride)
    #expect(text.fields.map(\.input.name) == glow.fields.map(\.input.name))
    let lyrics = NativeLyrics(device:device), p = NativeParameters("{}")
    let lines = [NativeLyricLine(time:0,text:"原生歌词 Metal",translation:nil,words:[])]
    _ = lyrics.prepare(lines:lines,position:0,session:"gpu",parameters:p,settings:p.lyricRasterKey,program:text)
    try await finish { !lyrics.pending }
    let colorDescriptor = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:.bgra8Unorm,width:512,height:512,mipmapped:false)
    colorDescriptor.usage = .renderTarget; colorDescriptor.storageMode = .shared
    let color = try #require(device.makeTexture(descriptor:colorDescriptor))
    let depthDescriptor = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:.depth32Float,width:512,height:512,mipmapped:false)
    depthDescriptor.usage = .renderTarget
    let depth = try #require(device.makeTexture(descriptor:depthDescriptor))
    let pass = MTLRenderPassDescriptor()
    pass.colorAttachments[0].texture = color; pass.colorAttachments[0].loadAction = .clear; pass.colorAttachments[0].storeAction = .store
    pass.depthAttachment.texture = depth; pass.depthAttachment.loadAction = .clear
    let command = try #require(queue.makeCommandBuffer()), encoder = try #require(command.makeRenderCommandEncoder(descriptor:pass))
    let sampler = try #require(device.makeSamplerState(descriptor:MTLSamplerDescriptor()))
    var state = NativeStageState(preset:0,fps:60,density:118,scale:1,intensity:0.85,active:true,visible:true,immersive:true,parameters:"{}",position:0,playing:true,cover:nil)
    state.session = "gpu"; state.lyrics = lines
    lyrics.render(encoder:encoder,program:text,glow:glow,sampler:sampler,state:state,parameters:p,position:0,time:1,dt:1/60,viewport:MTLViewport(originX:0,originY:0,width:512,height:512,znear:0,zfar:1),camera:NativeCamera(),palette:NativeCoverPalette())
    encoder.endEncoding()
    await withCheckedContinuation { (done: CheckedContinuation<Void,Never>) in command.addCompletedHandler { _ in done.resume() }; command.commit() }
    #expect(command.status == .completed)
    var pixels = [UInt8](repeating:0,count:512*512*4)
    pixels.withUnsafeMutableBytes { color.getBytes($0.baseAddress!,bytesPerRow:512*4,from:MTLRegionMake2D(0,0,512,512),mipmapLevel:0) }
    // Ignore clear alpha (255); only actual colored pixels prove a glyph was drawn.
    let lit = stride(from:0,to:pixels.count,by:4).filter { max(pixels[$0],pixels[$0+1],pixels[$0+2]) > 16 }.count
    #expect(lit > 100 && lit < 512*512/2)
    lyrics.close(); #expect(lyrics.textureCount == 0)
}
