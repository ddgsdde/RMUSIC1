import Metal
import Testing
import simd
@testable import MineradioNative

@Test(arguments: [SIMD2<Float>(-0.525,-0.525), SIMD2<Float>(16.27,28.49), SIMD2<Float>(40.1,-16.3)])
@MainActor func terrainFaceKeepsItsMaterialSeedDuringSmallCameraMovement(location: SIMD2<Float>) async throws {
    let device = try #require(MTLCreateSystemDefaultDevice()), queue = try #require(device.makeCommandQueue())
    let program = try NativeProgram(device:device,name:"terrain")
    let mesh = try program.boxes(device:device,grid:1,extent:1.05/0.9)
    let transform = mesh.instances!.contents().assumingMemoryBound(to:simd_float4x4.self)
    transform.pointee.columns.3.x = location.x; transform.pointee.columns.3.z = location.y
    let center = transform.pointee.columns.3
    let colorDescriptor = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:.bgra8Unorm,width:128,height:128,mipmapped:false)
    colorDescriptor.usage = .renderTarget; colorDescriptor.storageMode = .shared
    let color = try #require(device.makeTexture(descriptor:colorDescriptor))
    let depthDescriptor = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:.depth32Float,width:128,height:128,mipmapped:false)
    depthDescriptor.usage = .renderTarget
    let depth = try #require(device.makeTexture(descriptor:depthDescriptor))
    let state = NativeStageState(preset:7,fps:60,density:118,scale:1,intensity:0.85,active:true,visible:true,immersive:true,parameters:"{}",position:0,playing:true,cover:nil)
    var values = nativeUniforms(state:state,parameters:NativeParameters("{}"),timeline:NativeFeatureTimeline(),time:0,pixel:1,hasCover:false,tint:SIMD3(repeating:1))
    values["uSubBass"] = [0.8]; values["uSmoothness"] = [1]; values["uWarmth"] = [0]; values["uBrightness"] = [0]
    values["uPresence"] = [0.8]; values["uBrilliance"] = [0.7]; values["uAir"] = [0.6]
    for key in ["uCoolCore","uWarmCore"] { values[key] = [0.1,0.3,0.5] }
    for key in ["uCoolEdge","uWarmEdge"] { values[key] = [0.8,0.1,0.1] }
    var samples: [[Int]] = []
    for x: Float in [-0.001,0,0.001] {
        let view = nativeTransform(position:SIMD3(-center.x+x,center.z,-12),rotation:SIMD3(Float.pi/2,0,0))
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = color; pass.colorAttachments[0].loadAction = .clear; pass.colorAttachments[0].storeAction = .store
        pass.depthAttachment.texture = depth; pass.depthAttachment.loadAction = .clear
        let command = try #require(queue.makeCommandBuffer()), encoder = try #require(command.makeRenderCommandEncoder(descriptor:pass))
        encoder.setRenderPipelineState(program.pipeline)
        encoder.setFrontFacing(.counterClockwise); encoder.setCullMode(program.cullMode)
        encoder.setVertexBuffer(mesh.vertices,offset:0,index:16); encoder.setVertexBuffer(mesh.instances,offset:0,index:17)
        let data = program.uniformData(values,modelView:view,projection:NativeCamera().projection(aspect:1),view:view)
        data.withUnsafeBytes { encoder.setVertexBytes($0.baseAddress!,length:$0.count,index:0); encoder.setFragmentBytes($0.baseAddress!,length:$0.count,index:0) }
        encoder.drawPrimitives(type:.triangle,vertexStart:0,vertexCount:mesh.count,instanceCount:1)
        encoder.endEncoding()
        await withCheckedContinuation { (done: CheckedContinuation<Void,Never>) in command.addCompletedHandler { _ in done.resume() }; command.commit() }
        #expect(command.status == .completed)
        var pixels = [UInt8](repeating:0,count:128*128*4)
        pixels.withUnsafeMutableBytes { color.getBytes($0.baseAddress!,bytesPerRow:128*4,from:MTLRegionMake2D(0,0,128,128),mipmapLevel:0) }
        var patch: [Int] = []
        for y in 61..<67 { for x in 61..<67 { patch.append(contentsOf:(0..<3).map { Int(pixels[(y*128+x)*4+$0]) }) } }
        #expect(patch.max()! > 16)
        samples.append(patch)
    }
    let spatial = (0..<3).map { channel in
        let values = stride(from:channel,to:samples[1].count,by:3).map { samples[1][$0] }
        return values.max()!-values.min()!
    }.max()!
    let temporal = zip(samples[0],samples[2]).map { abs($0-$1) }.max()!
    print("Terrain constant-face RGB spread: spatial=\(spatial), camera delta=\(temporal)")
    #expect(spatial <= 2 && temporal <= 2)
}

@Test @MainActor func terrainGlintsKeepOriginalTimingWithAdjustableLowerAmplitude() async throws {
    let device = try #require(MTLCreateSystemDefaultDevice()), queue = try #require(device.makeCommandQueue())
    let program = try NativeProgram(device:device,name:"terrain"), mesh = try program.boxes(device:device,grid:1,extent:1.05/0.9)
    let transform = mesh.instances!.contents().assumingMemoryBound(to:simd_float4x4.self)
    transform.pointee.columns.0.x *= 4; transform.pointee.columns.2.z *= 4
    let center = transform.pointee.columns.3
    let descriptor = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:.bgra8Unorm,width:256,height:256,mipmapped:false)
    descriptor.usage = .renderTarget; descriptor.storageMode = .shared
    let texture = try #require(device.makeTexture(descriptor:descriptor))
    let state = NativeStageState(preset:7,fps:60,density:118,scale:1,intensity:0.85,active:true,visible:true,immersive:true,parameters:"{}",position:0,playing:true,cover:nil)
    var values = nativeUniforms(state:state,parameters:NativeParameters("{}"),timeline:NativeFeatureTimeline(),time:0,pixel:1,hasCover:false,tint:SIMD3(repeating:1))
    values["uSubBass"] = [0.8]; values["uSmoothness"] = [1]; values["uBrilliance"] = [0.15]
    values["uWarmth"] = [0]; values["uBrightness"] = [0]
    for key in ["uCoolCore","uWarmCore","uRippleColor"] { values[key] = [0.1,0.3,0.5] }
    let view = nativeTransform(position:SIMD3(-center.x,center.z,-12),rotation:SIMD3(Float.pi/2,0,0)), projection = NativeCamera().projection(aspect:1)
    // Keep geometry frozen: only the fragment's authored treble light clock advances.
    let vertex = program.uniformData(values,modelView:view,projection:projection,view:view)
    var changes = 0, peakFlash = 0
    for frame in 0..<90 {
        values["uTime"] = [Float(frame)/120]
        var samples: [[UInt8]] = []
        for strength: Float in [0,0.22,1] {
            values["uGlintStrength"] = [strength]
            let fragment = program.uniformData(values,modelView:view,projection:projection,view:view)
            let pass = MTLRenderPassDescriptor(); pass.colorAttachments[0].texture = texture
            pass.colorAttachments[0].loadAction = .clear; pass.colorAttachments[0].storeAction = .store
            let command = try #require(queue.makeCommandBuffer()), encoder = try #require(command.makeRenderCommandEncoder(descriptor:pass))
            encoder.setRenderPipelineState(program.pipeline); encoder.setFrontFacing(.counterClockwise); encoder.setCullMode(program.cullMode)
            encoder.setVertexBuffer(mesh.vertices,offset:0,index:16); encoder.setVertexBuffer(mesh.instances,offset:0,index:17)
            vertex.withUnsafeBytes { encoder.setVertexBytes($0.baseAddress!,length:$0.count,index:0) }
            fragment.withUnsafeBytes { encoder.setFragmentBytes($0.baseAddress!,length:$0.count,index:0) }
            encoder.drawPrimitives(type:.triangle,vertexStart:0,vertexCount:mesh.count,instanceCount:1); encoder.endEncoding()
            await withCheckedContinuation { (done: CheckedContinuation<Void,Never>) in command.addCompletedHandler { _ in done.resume() }; command.commit() }
            #expect(command.status == .completed)
            var pixels = [UInt8](repeating:0,count:256*256*4)
            pixels.withUnsafeMutableBytes { texture.getBytes($0.baseAddress!,bytesPerRow:256*4,from:MTLRegionMake2D(0,0,256,256),mipmapLevel:0) }
            samples.append(pixels)
        }
        // All three renders have identical geometry and clock. Low-strength flashes
        // occur on exactly the original frames/pixels, at 22% of the original gain.
        var worstError: Float = 0
        for i in samples[0].indices where i%4 != 3 && samples[2][i] < 250 {
            let full = Int(samples[2][i])-Int(samples[0][i]), low = Int(samples[1][i])-Int(samples[0][i])
            worstError = max(worstError,abs(Float(low)-Float(full)*0.22))
            if full > 8 { changes += 1; peakFlash = max(peakFlash,low) }
        }
        #expect(worstError <= 2)
    }
    print("Authored flash samples: \(changes), attenuated peak RGB gain: \(peakFlash)")
    #expect(changes > 0 && peakFlash > 0 && peakFlash <= 27)
}
