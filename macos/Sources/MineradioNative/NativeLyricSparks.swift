@preconcurrency import MetalKit
import simd

/// The authored 132 lyric motes. Two small buffers, created only when enabled.
@MainActor final class NativeLyricSparks {
    private var buffers: [MTLBuffer] = []
    private var slot = 0
    private let seeds: [SIMD4<Float>] = {
        var random: UInt64 = 0x6c79726963
        func next() -> Float { random = random &* 6364136223846793005 &+ 1;return Float((random>>32)&0xffffff)/Float(0x1000000) }
        return (0..<132).map { _ in
            let angle = next()*2*Float.pi, ring = 0.78+pow(next(),1.45)*0.58
            return SIMD4(cos(angle)*(0.50+next()*0.22)*ring+(next()-0.5)*0.12,sin(angle)*(0.42+next()*0.22)*ring+(next()-0.5)*0.14,(next()-0.5)*0.24,next()*1000)
        }
    }()
    var bufferCount: Int { buffers.count }
    func close() { buffers = [];slot = 0 }
    func prepare(device: MTLDevice, program: NativeProgram, width: Float, height: Float, time: Float, beat: Float, bass: Float, mid: Float) -> MTLBuffer? {
        if buffers.isEmpty { buffers = (0..<2).compactMap { _ in device.makeBuffer(length:132*program.stride,options:.storageModeShared) } }
        guard buffers.count == 2 else { return nil }
        let buffer = buffers[slot];slot = (slot+1)%2
        let values = buffer.contents().assumingMemoryBound(to:Float.self)
        for (i,seed) in seeds.enumerated() {
            let phase = Float(i)*12.989+seed.w, breath = 0.62+0.38*sin(time*(0.32+Float(i%7)*0.025)+phase)
            let point = SIMD3(seed.x*width+sin(time*(0.18+Float(i%5)*0.025)+phase)*(0.045+bass*0.030+beat*0.052)+cos(time*0.11+phase)*0.018*breath,seed.y*height+cos(time*(0.16+Float(i%6)*0.024)+phase)*(0.042+mid*0.026+beat*0.046)+sin(time*0.13+phase)*0.016*breath,seed.z+sin(time*(0.24+Float(i%4)*0.035)+phase)*(0.036+beat*0.028))
            for field in program.fields {
                let at = i*program.stride/4+field.offset/4
                if field.input.name == "position" { values[at] = point.x;values[at+1] = point.y;values[at+2] = point.z }
                if field.input.name == "seed" { values[at] = seed.w }
            }
        }
        return buffer
    }
}
