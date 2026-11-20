import Metal
import Testing
import simd
@testable import MineradioNative

@Test func sonicMeteorLandsOnceAndItsTrailsExpireWithoutAudio() {
    var particles = NativeSonicParticles(), sonic = NativeSonicMotion()
    particles.addMeteor(strength:0.9,time:0)
    #expect(particles.meteorCount == 1)
    particles.addMeteor(strength:0.9,time:0.1)
    #expect(particles.meteorCount == 1)
    var impacts = 0, peakTrails = 0
    for frame in 1...240 {
        particles.step(beat:0,time:Float(frame)/60,dt:1/60)
        impacts += particles.impacts.count
        for impact in particles.impacts { sonic.impact(impact) }
        peakTrails = max(peakTrails,particles.trailCount)
    }
    #expect(impacts == 1 && peakTrails > 0)
    #expect(particles.meteorCount == 0 && particles.trailCount == 0)
    #expect(stride(from:3,to:40,by:4).contains { sonic.rippleValues(workshop:false)[$0] < 0 })
    // Repeated playback bursts never grow the fixed pools.
    for frame in 1...7200 {
        particles.step(beat:0.9,time:Float(frame)/60,dt:1/60)
        #expect(particles.meteorCount <= 20 && particles.trailCount <= 200)
    }
    particles.close()
    #expect(particles.meteorCount == 0 && particles.trailCount == 0 && particles.impacts.isEmpty)
}

@Test @MainActor func sonicParticlesActuallyDrawAndReleaseTheirTwoInstanceBuffers() async throws {
    let device = try #require(MTLCreateSystemDefaultDevice()), queue = try #require(device.makeCommandQueue())
    let program = try NativeProgram(device:device,name:"sonicParticle")
    let mesh = try program.boxes(device:device,grid:1,extent:1.05/0.9)
    var particles = NativeSonicParticles()
    particles.addMeteor(strength:0.9,time:0)
    var first = particles.prepareBuffer(device:device)
    let position = try #require(first).contents().assumingMemoryBound(to:simd_float4x4.self).pointee.columns.3
    let colorDescriptor = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:.bgra8Unorm,width:128,height:128,mipmapped:false)
    colorDescriptor.usage = .renderTarget; colorDescriptor.storageMode = .shared
    let color = try #require(device.makeTexture(descriptor:colorDescriptor))
    let depthDescriptor = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:.depth32Float,width:128,height:128,mipmapped:false)
    depthDescriptor.usage = .renderTarget
    let depth = try #require(device.makeTexture(descriptor:depthDescriptor))
    let pass = MTLRenderPassDescriptor()
    pass.colorAttachments[0].texture = color; pass.colorAttachments[0].loadAction = .clear; pass.colorAttachments[0].storeAction = .store
    pass.depthAttachment.texture = depth; pass.depthAttachment.loadAction = .clear
    let command = try #require(queue.makeCommandBuffer()), encoder = try #require(command.makeRenderCommandEncoder(descriptor:pass))
    particles.render(encoder:encoder,program:program,mesh:mesh,device:device,modelView:nativeTransform(position:SIMD3(-position.x,-position.y,-position.z-4)),projection:NativeCamera().projection(aspect:1),warm:[1,0,0],accent:[0,1,0])
    encoder.endEncoding()
    await withCheckedContinuation { (done: CheckedContinuation<Void,Never>) in command.addCompletedHandler { _ in done.resume() }; command.commit() }
    #expect(command.status == .completed)
    var pixels = [UInt8](repeating:0,count:128*128*4)
    pixels.withUnsafeMutableBytes { color.getBytes($0.baseAddress!,bytesPerRow:128*4,from:MTLRegionMake2D(0,0,128,128),mipmapLevel:0) }
    let lit = stride(from:0,to:pixels.count,by:4).filter { max(pixels[$0],pixels[$0+1],pixels[$0+2]) > 128 }.count
    #expect(lit > 100 && lit < 128*128/2)
    #expect(particles.bufferCount == 2)
    let reused = particles.prepareBuffer(device:device) === first
    #expect(reused)
    first = nil; particles.close()
    #expect(particles.bufferCount == 0)
    // Test owner release separately from command-buffer and Objective-C autorelease retention.
    weak var released: AnyObject?
    autoreleasepool {
        var owner = NativeSonicParticles()
        owner.addMeteor(strength:0.9,time:0)
        let buffer = owner.prepareBuffer(device:device)
        released = buffer as AnyObject?
        owner.close()
    }
    #expect(released == nil)
}
