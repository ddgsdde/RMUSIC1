@preconcurrency import MetalKit

/// One reusable attachment pair. Apple tile memory holds MSAA samples only for
/// this pass; the resolved drawable is the only color image stored afterwards.
@MainActor final class NativeRenderTargets {
    let sampleCount: Int
    private let device: MTLDevice
    private var color: MTLTexture?
    private var depth: MTLTexture?
    var retainedTextureCount: Int { (color == nil ? 0 : 1) + (depth == nil ? 0 : 1) }
    init(device: MTLDevice) {
        self.device = device
        sampleCount = [4,2].first { device.supportsTextureSampleCount($0) } ?? 1
    }
    func release() { color = nil; depth = nil }
    func pass(resolving output: MTLTexture, clear: MTLClearColor = MTLClearColorMake(0.025,0.03,0.045,1)) -> MTLRenderPassDescriptor? {
        if depth?.width != output.width || depth?.height != output.height {
            release()
            func texture(_ format: MTLPixelFormat) -> MTLTexture? {
                let d = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:format,width:output.width,height:output.height,mipmapped:false)
                d.textureType = sampleCount > 1 ? .type2DMultisample : .type2D
                d.sampleCount = sampleCount; d.usage = .renderTarget
                d.storageMode = device.supportsFamily(.apple1) ? .memoryless : .private
                return device.makeTexture(descriptor:d)
            }
            if sampleCount > 1 { color = texture(output.pixelFormat) }
            depth = texture(.depth32Float)
        }
        guard let depth, sampleCount == 1 || color != nil else { release(); return nil }
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = sampleCount > 1 ? color : output
        pass.colorAttachments[0].resolveTexture = sampleCount > 1 ? output : nil
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].clearColor = clear
        pass.colorAttachments[0].storeAction = sampleCount > 1 ? .multisampleResolve : .store
        pass.depthAttachment.texture = depth
        pass.depthAttachment.loadAction = .clear; pass.depthAttachment.storeAction = .dontCare
        pass.depthAttachment.clearDepth = 1
        return pass
    }
}
