@preconcurrency import MetalKit
import ImageIO

/// Two cover generations, one low-resolution depth map, and one shared point sprite.
@MainActor final class NativeArtwork {
    private let device: MTLDevice
    private(set) var textures: [String: MTLTexture] = [:]
    private var colours = NativePaletteTransition()
    var palette: NativeCoverPalette { colours.value }
    private var coverProgress: Float = 1
    private var clearing = false
    var tint: SIMD3<Float> { palette.primary }
    private(set) var blend: Float = 1
    private var desired: String?, applied: String?
    private var generation = 0
    private var preparation: Task<Void,Never>?
    private struct Prepared: Sendable { let pixels: [UInt8]; let depth: [UInt8]; let palette: NativeCoverPalette }
    init(device: MTLDevice) {
        self.device = device
        textures["black"] = texture(width: 1,height: 1,pixels: [0,0,0,255])
        var dot = [UInt8](repeating: 255,count: 64*64*4)
        let stops: [(Float,Float)] = [(0,0.96),(0.42,0.78),(0.72,0.22),(1,0)]
        for y in 0..<64 { for x in 0..<64 {
            let d = min(1,sqrt(pow(Float(x)-32,2)+pow(Float(y)-32,2))/31)
            for i in 1..<stops.count where d >= stops[i-1].0 && d <= stops[i].0 {
                let t = (d-stops[i-1].0)/(stops[i].0-stops[i-1].0)
                dot[(y*64+x)*4+3] = UInt8(max(0,min(255,(stops[i-1].1+(stops[i].1-stops[i-1].1)*t)*255)))
            }
        } }
        textures["uDotTex"] = texture(width: 64,height: 64,pixels: dot)
    }
    func close() { generation += 1; preparation?.cancel(); preparation = nil; desired = nil; applied = nil; textures.removeAll() }
    func step(_ dt: Float) {
        colours.step(dt); coverProgress = min(1,coverProgress+max(0,dt)/0.75)
        blend = coverProgress*coverProgress*(3-2*coverProgress)
        if clearing && coverProgress >= 1 { textures["uCoverTex"] = nil; textures["uPrevCoverTex"] = nil; clearing = false }
    }
    func texture(width: Int, height: Int, pixels: [UInt8]) -> MTLTexture? {
        let descriptor = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .rgba8Unorm,width: width,height: height,mipmapped: false)
        descriptor.usage = .shaderRead; descriptor.storageMode = .shared
        guard let texture = device.makeTexture(descriptor: descriptor) else { return nil }
        pixels.withUnsafeBytes { texture.replace(region: MTLRegionMake2D(0,0,width,height),mipmapLevel: 0,withBytes: $0.baseAddress!,bytesPerRow: width*4) }
        return texture
    }
    func update(_ encoded: String?, active: Bool) {
        if desired != encoded { generation += 1; preparation?.cancel(); preparation = nil; desired = encoded }
        guard active else { if preparation != nil { generation += 1; preparation?.cancel(); preparation = nil }; return }
        guard let encoded, encoded.count <= 1_048_576 else {
            if !clearing, applied != nil {
                textures["uPrevCoverTex"] = textures["uCoverTex"] ?? textures["black"]
                textures["uCoverTex"] = textures["black"]; textures["uEdgeTex"] = nil
                colours.move(to: NativeCoverPalette()); coverProgress = 0; blend = 0; clearing = true; applied = nil
            }
            return
        }
        guard applied != encoded, preparation == nil else { return }
        let generation = generation
        preparation = Task { [weak self] in
            let worker = Task.detached(priority: .utility) { try Self.prepare(encoded) }
            do {
                let result = try await withTaskCancellationHandler { try await worker.value } onCancel: { worker.cancel() }
                try Task.checkCancellation()
                guard let self, self.generation == generation else { return }
                self.textures["uPrevCoverTex"] = self.textures["uCoverTex"] ?? self.textures["black"]
                self.textures["uCoverTex"] = self.texture(width:256,height:256,pixels:result.pixels)
                self.textures["uEdgeTex"] = self.texture(width:256,height:256,pixels:result.depth)
                self.colours.move(to: result.palette); self.coverProgress = 0; self.blend = 0; self.clearing = false; self.applied = encoded
                self.preparation = nil
            } catch {
                guard let self, self.generation == generation else { return }
                self.preparation = nil
                // Retry only when the input or lifecycle changes, not every display tick.
                self.applied = encoded
            }
        }
    }
    nonisolated private static func prepare(_ encoded: String) throws -> Prepared {
        try Task.checkCancellation()
        guard let comma = encoded.firstIndex(of:","),
              let data = Data(base64Encoded:String(encoded[encoded.index(after:comma)...])),
              let source = CGImageSourceCreateWithData(data as CFData,nil),
              let image = CGImageSourceCreateThumbnailAtIndex(source,0,[kCGImageSourceCreateThumbnailFromImageAlways:true,kCGImageSourceThumbnailMaxPixelSize:256,kCGImageSourceCreateThumbnailWithTransform:true] as CFDictionary) else { throw NativeStageFailure.resource("cover") }
        var pixels = [UInt8](repeating:0,count:256*256*4)
        let drawn = pixels.withUnsafeMutableBytes { bytes -> Bool in
            guard let context = CGContext(data:bytes.baseAddress,width:256,height:256,bitsPerComponent:8,bytesPerRow:1024,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            context.translateBy(x:0,y:256); context.scaleBy(x:1,y:-1)
            context.draw(image,in:CGRect(x:0,y:0,width:256,height:256)); return true
        }
        guard drawn else { throw NativeStageFailure.resource("cover pixels") }
        try Task.checkCancellation()
        let palette = NativeCoverPalette.extract(pixels,width:256,height:256)
        let depth = try NativeCoverDepth.make(pixels,width:256,height:256)
        return Prepared(pixels:pixels,depth:depth,palette:palette)
    }
}
