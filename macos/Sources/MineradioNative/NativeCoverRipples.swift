@preconcurrency import MetalKit
import simd

/// The twelve authored cover impulses, independent of the terrain's ten rings.
/// Two tiny textures follow the renderer's two in-flight command buffers.
struct NativeCoverRipples {
    private(set) var values = [SIMD4<Float>](repeating:SIMD4(0,0,-10,0),count:12)
    private var nextIndex = 0, previousBass = false
    private var cooldown: Float = 0
    private var random: UInt64 = 0x636f766572
    private var textures: [MTLTexture] = []
    private var slot = 0
    var count: Int { values.filter { $0.w > 0.005 }.count }
    mutating func reset() { self = NativeCoverRipples() }
    private mutating func next() -> Float {
        random = random &* 6364136223846793005 &+ 1
        return Float((random >> 32) & 0xffffff)/Float(0x1000000)
    }
    mutating func step(bass: Float, dt: Float) {
        cooldown += dt
        let hit = bass > 0.30 && !previousBass
        previousBass = bass > 0.225
        if hit && cooldown > 0.32 {
            cooldown = 0
            let count = next() < 0.5 ? 2 : 3
            var regions = Array(0..<9)
            for _ in 0..<count {
                let region = regions.remove(at:min(regions.count-1,Int(next()*Float(regions.count))))
                let x = (Float(region%3)/2-0.5)*4.8*0.72+(next()-0.5)*0.7
                let y = (Float(region/3)/2-0.5)*4.8*0.72+(next()-0.5)*0.7
                values[nextIndex] = SIMD4(x,y,0,0.65+bass*1.4+next()*0.25)
                nextIndex = (nextIndex+1)%values.count
            }
        }
        for i in values.indices where values[i].w > 0.005 {
            values[i].z += dt
            if values[i].z > 2 { values[i].z = -10; values[i].w = 0 }
        }
    }
    mutating func texture(device: MTLDevice) -> MTLTexture? {
        if textures.isEmpty {
            let d = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:.rgba32Float,width:1,height:12,mipmapped:false)
            d.usage = .shaderRead; d.storageMode = .shared
            textures = (0..<2).compactMap { _ in device.makeTexture(descriptor:d) }
        }
        guard textures.count == 2 else { return nil }
        let texture = textures[slot]; slot = (slot+1)%2
        var packed = values.filter { $0.w > 0.005 }
        packed += Array(repeating:SIMD4<Float>(0,0,-10,0),count:12-packed.count)
        packed.withUnsafeBytes { texture.replace(region:MTLRegionMake2D(0,0,1,12),mipmapLevel:0,withBytes:$0.baseAddress!,bytesPerRow:16) }
        return texture
    }
}
