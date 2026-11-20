import AppKit
import Metal
import simd
import Testing
@testable import MineradioNative

private func pixels(_ kind: Int) -> [UInt8] {
    var result = [UInt8](repeating:255,count:256*256*4)
    for y in 0..<256 { for x in 0..<256 {
        let rgb: [UInt8] = kind == 2 ? [128,128,128] : kind == 1 || kind == 3 && x >= 192 ? [30,70,210] : [210,30,50]
        for c in 0..<3 { result[(y*256+x)*4+c] = rgb[c] }
    } }
    return result
}
private func bytes(_ value: SIMD3<Float>) -> [Int] { [value.x,value.y,value.z].map { Int(($0*255).rounded()) } }

@Test func nativePaletteMatchesPinnedJavaScriptGoldenColorsAndAreaDominance() {
    // Golden values executed from stage/07-lyrics-palette-text-utils.js at the pinned revision.
    let red = NativeCoverPalette.extract(pixels(0),width:256,height:256)
    let blue = NativeCoverPalette.extract(pixels(1),width:256,height:256)
    let gray = NativeCoverPalette.extract(pixels(2),width:256,height:256)
    let regions = NativeCoverPalette.extract(pixels(3),width:256,height:256)
    #expect(bytes(red.lyric) == [249,144,156]); #expect(bytes(red.secondary) == [239,153,103]); #expect(bytes(red.highlight) == [248,209,207])
    #expect(bytes(blue.lyric) == [144,167,249]); #expect(bytes(blue.secondary) == [138,103,239]); #expect(bytes(blue.highlight) == [207,208,248])
    #expect(gray.monochrome && bytes(gray.primary) == [128,128,128]); #expect(bytes(gray.lyric) == [216,241,255])
    #expect(bytes(regions.primary) == [210,30,50]); #expect(bytes(regions.warm) == [210,30,50]); #expect(bytes(regions.cool) == [30,70,210]); #expect(bytes(regions.accent) == [30,70,210])
}

@Test func nativeDepthMatchesPinnedJavaScriptGoldenSamples() throws {
    var image = [UInt8](repeating:255,count:256*256*4)
    for y in 0..<256 { for x in 0..<256 { let i = (y*256+x)*4; image[i] = UInt8(x); image[i+1] = UInt8(y); image[i+2] = UInt8((x+y)%256) } }
    let result = try NativeCoverDepth.make(image,width:256,height:256)
    let golden: [(Int,Int,[Int])] = [(0,0,[1,0,0,0]),(128,128,[197,38,137,113]),(1,1,[1,6,3,1]),(127,127,[199,34,136,141]),(254,254,[114,6,71,254]),(64,192,[131,38,98,132]),(255,255,[114,0,68,255])]
    for (x,y,expected) in golden { for c in 0..<4 { #expect(abs(Int(result[(y*256+x)*4+c])-expected[c]) <= 1) } }
}

@Test @MainActor func nativeArtworkStopsInBackgroundAndCloseReleasesGpuTextures() async throws {
    let device = try #require(MTLCreateSystemDefaultDevice())
    let image = try #require(NSBitmapImageRep(bitmapDataPlanes:nil,pixelsWide:16,pixelsHigh:16,bitsPerSample:8,samplesPerPixel:4,hasAlpha:true,isPlanar:false,colorSpaceName:.deviceRGB,bytesPerRow:64,bitsPerPixel:32))
    let data = try #require(image.bitmapData)
    for i in 0..<256 { data[i*4] = 210; data[i*4+1] = 30; data[i*4+2] = 50; data[i*4+3] = 255 }
    let png = try #require(image.representation(using:.png,properties:[:]))
    let cover = "data:image/png;base64,"+png.base64EncodedString()
    var artwork: NativeArtwork? = NativeArtwork(device:device)
    weak var released = artwork
    artwork?.update(cover,active:false)
    try await Task.sleep(for:.milliseconds(40))
    #expect(artwork?.textures["uCoverTex"] == nil)
    artwork?.update(cover,active:true)
    let deadline = ContinuousClock.now.advanced(by:.seconds(3))
    while artwork?.textures["uEdgeTex"] == nil && ContinuousClock.now < deadline { try await Task.sleep(for:.milliseconds(10)) }
    #expect(artwork?.textures["uCoverTex"]?.width == 256); #expect(artwork?.textures["uEdgeTex"]?.width == 256)
    #expect(artwork?.blend == 0)
    artwork?.step(0.3); #expect((artwork?.blend ?? 0) > 0 && (artwork?.blend ?? 1) < 1)
    artwork?.step(1)
    #expect(artwork.map { bytes($0.palette.primary) } == [210,30,50])
    artwork?.update(nil,active:true)
    #expect(artwork?.textures["uCoverTex"] != nil)
    artwork?.step(0.3); #expect(artwork?.textures["uPrevCoverTex"] != nil)
    artwork?.step(1); #expect(artwork?.textures["uCoverTex"] == nil)
    artwork?.close(); #expect(artwork?.textures.isEmpty == true)
    artwork = nil
    try await Task.sleep(for:.milliseconds(20))
    #expect(released == nil)
}

@Test func nativeCoverColoursStayContinuousAcrossInterruptedAndDifferentFrameRateTransitions() {
    var red = NativeCoverPalette(), blue = red, green = red
    red.primary = SIMD3(1,0,0); blue.primary = SIMD3(0,0,1); green.primary = SIMD3(0,1,0)
    var colour = NativePaletteTransition(); colour.move(to:red);colour.step(1)
    colour.move(to:blue); #expect(colour.value.primary == red.primary)
    colour.step(0.375); let midway = colour.value.primary
    #expect(abs(midway.x-0.5)<0.0001 && abs(midway.z-0.5)<0.0001)
    colour.move(to:green); #expect(colour.value.primary == midway)
    colour.step(1); #expect(colour.value.primary == green.primary)
    var fast = NativePaletteTransition(), slow = fast
    fast.move(to:blue);slow.move(to:blue)
    for _ in 0..<30 { fast.step(1/60) };for _ in 0..<15 { slow.step(1/30) }
    #expect(length(fast.value.primary-slow.value.primary)<0.0001)
}
