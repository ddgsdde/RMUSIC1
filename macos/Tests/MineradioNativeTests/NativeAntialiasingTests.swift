import Metal
import Testing
import simd
@testable import MineradioNative

@Test @MainActor func multisamplingResolvesTerrainEdgesWithoutBlurringTheInterior() async throws {
    let device = try #require(MTLCreateSystemDefaultDevice()), queue = try #require(device.makeCommandQueue())
    #expect(device.supportsTextureSampleCount(4))
    let size = 128
    func texture(_ format: MTLPixelFormat, samples: Int, storage: MTLStorageMode) throws -> MTLTexture {
        let d = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:format,width:size,height:size,mipmapped:false)
        d.textureType = samples == 1 ? .type2D : .type2DMultisample
        d.sampleCount = samples; d.usage = .renderTarget; d.storageMode = storage
        return try #require(device.makeTexture(descriptor:d))
    }
    var images: [[UInt8]] = []
    for samples in [1,4] {
        let program = try NativeProgram(device:device,name:"terrain",sampleCount:samples)
        let mesh = try program.boxes(device:device,grid:1,extent:1.05/0.9)
        let matrix = mesh.instances!.contents().assumingMemoryBound(to:simd_float4x4.self)
        matrix.pointee = nativeTransform(position:SIMD3(0,0,0),scale:SIMD3(3,1,3),rotation:SIMD3(0,0.29,0))
        let output = try texture(.bgra8Unorm,samples:1,storage:.shared)
        let targets = NativeRenderTargets(device:device)
        let pass: MTLRenderPassDescriptor
        if samples == 4 {
            pass = try #require(targets.pass(resolving:output))
            let reused = try #require(targets.pass(resolving:output))
            #expect(pass.colorAttachments[0].texture === reused.colorAttachments[0].texture)
            #expect(targets.retainedTextureCount == 2)
            if device.supportsFamily(.apple1) { #expect(pass.colorAttachments[0].texture?.storageMode == .memoryless) }
        } else {
            pass = MTLRenderPassDescriptor()
            pass.colorAttachments[0].texture = output
            pass.colorAttachments[0].storeAction = .store
            pass.depthAttachment.texture = try texture(.depth32Float,samples:1,storage:.private)
            pass.depthAttachment.loadAction = .clear; pass.depthAttachment.storeAction = .dontCare
        }
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].clearColor = MTLClearColorMake(0,0,0,0)
        let command = try #require(queue.makeCommandBuffer()), encoder = try #require(command.makeRenderCommandEncoder(descriptor:pass))
        encoder.setRenderPipelineState(program.pipeline); encoder.setFrontFacing(.counterClockwise); encoder.setCullMode(program.cullMode)
        encoder.setVertexBuffer(mesh.vertices,offset:0,index:16); encoder.setVertexBuffer(mesh.instances,offset:0,index:17)
        let state = NativeStageState(preset:7,fps:60,density:118,scale:1,intensity:0.85,active:true,visible:true,immersive:true,parameters:"{}",position:0,playing:true,cover:nil)
        var values = nativeUniforms(state:state,parameters:NativeParameters("{}"),timeline:NativeFeatureTimeline(),time:0,pixel:1,hasCover:false,tint:SIMD3(repeating:1))
        values["uGlintStrength"] = [0]
        let view = nativeTransform(position:SIMD3(0,0,-10),rotation:SIMD3(Float.pi/2,0,0))
        let data = program.uniformData(values,modelView:view,projection:NativeCamera().projection(aspect:1),view:view)
        data.withUnsafeBytes { encoder.setVertexBytes($0.baseAddress!,length:$0.count,index:0); encoder.setFragmentBytes($0.baseAddress!,length:$0.count,index:0) }
        encoder.drawPrimitives(type:.triangle,vertexStart:0,vertexCount:mesh.count,instanceCount:1); encoder.endEncoding()
        await withCheckedContinuation { (done: CheckedContinuation<Void,Never>) in command.addCompletedHandler { _ in done.resume() }; command.commit() }
        #expect(command.status == .completed)
        var pixels = [UInt8](repeating:0,count:size*size*4)
        pixels.withUnsafeMutableBytes { output.getBytes($0.baseAddress!,bytesPerRow:size*4,from:MTLRegionMake2D(0,0,size,size),mipmapLevel:0) }
        images.append(pixels)
        targets.release(); #expect(targets.retainedTextureCount == 0)
    }
    let partial = images.map { pixels in stride(from:3,to:pixels.count,by:4).filter { pixels[$0] > 0 && pixels[$0] < 255 }.count }
    #expect(partial[0] == 0)
    #expect(partial[1] > 20)
    // MSAA changes edge coverage, rather than applying a blur to the entire material.
    var interiorDifference = 0
    for y in 61..<67 { for x in 61..<67 { for c in 0..<3 {
        let i = (y*size+x)*4+c
        interiorDifference = max(interiorDifference,abs(Int(images[0][i])-Int(images[1][i])))
    } } }
    #expect(images[0][(64*size+64)*4+3] == 255)
    #expect(interiorDifference <= 2)
    print("Terrain edge coverage: 1x=\(partial[0]), 4x=\(partial[1]); interior RGB delta=\(interiorDifference)")
}
