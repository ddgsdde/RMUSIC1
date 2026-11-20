@preconcurrency import MetalKit
import simd

/// Fixed pools from sonic-topography.js: 20 falling meteors and 200 short-lived trails.
/// Sampling probabilities use elapsed time so the configured 24/30/60 fps changes no rate.
struct NativeSonicParticles {
    private struct Meteor {
        var position = SIMD3<Float>(0,-1000,0)
        var speed: Float = 0
        var strength: Float = 0
    }
    private struct Trail {
        var position = SIMD3<Float>.zero
        var velocity = SIMD3<Float>.zero
        var life: Float = 1
        var lifetime: Float = 0
        var scale: Float = 0
    }
    private var meteors = [Meteor](repeating:Meteor(),count:20)
    private var trails = [Trail](repeating:Trail(),count:200)
    private var meteorIndex = 0, trailIndex = 0
    private var lastMeteor: Float = -999
    private var random: UInt64 = 0x736f6e6963
    private var buffers: [MTLBuffer] = []
    private var slot = 0
    private var workshop = false
    private(set) var impacts: [SIMD3<Float>] = []
    var meteorCount: Int { meteors.reduce(0) { $0+($1.position.y > 0 ? 1 : 0) } }
    var trailCount: Int { trails.reduce(0) { $0+($1.life < $1.lifetime ? 1 : 0) } }
    var bufferCount: Int { buffers.count }

    mutating func close() { self = NativeSonicParticles() }
    private mutating func next() -> Float {
        random = random &* 6364136223846793005 &+ 1
        return Float((random >> 32) & 0xffffff)/Float(0x1000000)
    }
    mutating func addMeteor(strength: Float, time: Float, workshop: Bool = false) {
        self.workshop = workshop
        guard time-lastMeteor >= (workshop ? 1 : 0.55) else { return }
        lastMeteor = time
        let angle = next()*Float.pi*2, distance = workshop ? 10+next()*35 : next()*25
        let height = 30+next()*10, speed = 1+next()*0.5+strength*(workshop ? 2.5 : 1.5)
        meteors[meteorIndex] = Meteor(position:SIMD3(cos(angle)*distance,height,sin(angle)*distance),speed:speed,strength:strength)
        meteorIndex = (meteorIndex+1)%20
    }
    private mutating func spawnTrail(at position: SIMD3<Float>, speed: Float) {
        let jitter = SIMD3(next()-0.5,next()-0.5,next()-0.5)*1.5
        let velocity = SIMD3((next()-0.5)*2,next()*2+speed*10,(next()-0.5)*2)
        let lifetime = 0.5+next()*0.5, scale = next()*0.6+0.2
        trails[trailIndex] = Trail(position:position+jitter,velocity:velocity,life:0,lifetime:lifetime,scale:scale)
        trailIndex = (trailIndex+1)%200
    }
    mutating func step(beat: Float, time: Float, dt: Float, workshop: Bool = false) {
        self.workshop = workshop
        guard dt > 0, dt.isFinite else { return }
        impacts.removeAll(keepingCapacity:true)
        if !workshop, beat > 0.62, next() < 1-pow(0.955,dt*60) { addMeteor(strength:min(0.9,max(0.28,beat)),time:time) }
        for i in meteors.indices where meteors[i].position.y > 0 {
            var meteor = meteors[i]
            meteor.position.y -= meteor.speed*60*dt
            if meteor.position.y <= 0 {
                impacts.append(SIMD3(meteor.position.x,meteor.position.z,min(1.2,meteor.strength)))
                for _ in 0..<10 { spawnTrail(at:SIMD3(meteor.position.x,0.5,meteor.position.z),speed:meteor.speed*1.5) }
                meteor.position.y = -1000
            } else if next() < 1-pow(0.3,dt*60) {
                spawnTrail(at:meteor.position,speed:meteor.speed*0.2)
            }
            meteors[i] = meteor
        }
        for i in trails.indices where trails[i].life < trails[i].lifetime {
            trails[i].life += dt
            trails[i].position += trails[i].velocity*dt*10
        }
    }
    /// Two tiny instance buffers match the renderer's maximum two commands in flight.
    mutating func prepareBuffer(device: MTLDevice) -> MTLBuffer? {
        guard meteorCount+trailCount > 0 else { return nil }
        if buffers.isEmpty { buffers = (0..<2).compactMap { _ in device.makeBuffer(length:220*64,options:.storageModeShared) } }
        guard buffers.count == 2 else { return nil }
        let buffer = buffers[slot]; slot = (slot+1)%2
        let matrices = buffer.contents().assumingMemoryBound(to:simd_float4x4.self)
        for (i,meteor) in meteors.enumerated() {
            matrices[i] = nativeTransform(position:meteor.position,scale:meteor.position.y > 0 ? (workshop ? SIMD3(repeating:1.5) : SIMD3(0.6,1.8,0.6)) : .zero)
        }
        for (i,trail) in trails.enumerated() {
            let scale = trail.life < trail.lifetime ? trail.scale*(1-trail.life/trail.lifetime)*(workshop ? 1 : 0.8) : 0
            matrices[20+i] = nativeTransform(position:trail.position,scale:SIMD3(repeating:scale))
        }
        return buffer
    }
    mutating func render(encoder: MTLRenderCommandEncoder, program: NativeProgram, mesh: NativeMesh,
                         device: MTLDevice, modelView: simd_float4x4, projection: simd_float4x4,
                         warm: [Float], accent: [Float]) {
        guard let buffer = prepareBuffer(device:device) else { return }
        encoder.setFrontFacing(.counterClockwise); encoder.setCullMode(program.cullMode)
        encoder.setRenderPipelineState(program.pipeline)
        encoder.setVertexBuffer(mesh.vertices,offset:0,index:16)
        for (offset,count,color,opacity) in [(0,20,warm.map { $0*0.3+0.7 },Float(1)),(20*64,200,accent,Float(0.6))] {
            encoder.setVertexBuffer(buffer,offset:offset,index:17)
            let data = program.uniformData(["uColor":color,"uOpacity":[opacity]],modelView:modelView,projection:projection)
            data.withUnsafeBytes { encoder.setVertexBytes($0.baseAddress!,length:$0.count,index:0); encoder.setFragmentBytes($0.baseAddress!,length:$0.count,index:0) }
            encoder.drawPrimitives(type:mesh.primitive,vertexStart:0,vertexCount:mesh.count,instanceCount:count)
        }
    }
}
