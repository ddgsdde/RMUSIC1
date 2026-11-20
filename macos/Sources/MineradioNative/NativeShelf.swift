@preconcurrency import MetalKit
import Foundation
import simd

/// Eleven cards at most, no networking and no audio. Core still validates every action.
@MainActor final class NativeShelf {
    private struct Card { let content: NativeShelfContent; let texture: MTLTexture }
    private let device: MTLDevice
    private var cards: [Int:Card] = [:]
    private var task: Task<Void,Never>?
    private var generation = 0
    private var desired: [NativeShelfContent] = []
    private var items: [NativeQueueItem] = []
    private var current = -1, target: Float = 0, center: Float = 0, reveal: Float = 0
    private var hoverIndex: Int?
    private(set) var hits: [NativeShelfHit] = []
    private var published: [Int] = []
    var select: (Int,String) -> Void = { _,_ in }
    var windowChanged: ([Int]) -> Void = { _ in }
    var textureCount: Int { cards.count }
    var pending: Bool { task != nil }
    init(device: MTLDevice) { self.device = device }

    func close() {
        generation += 1; task?.cancel(); task = nil; cards.removeAll(); desired = []; hits = []; reveal = 0
        if !published.isEmpty { published = []; publish([]) }
    }
    private func publish(_ indices: [Int]) {
        // updateNSView must never mutate SwiftUI's state synchronously.
        Task { @MainActor [weak self] in
            guard let self, self.published == indices else { return }
            self.windowChanged(indices)
        }
    }
    func update(_ state: NativeStageState, parameters p: NativeParameters, active: Bool) {
        let previousIDs = items.map(\.id)
        items = Array(state.queue.prefix(50))
        if current != state.queueIndex || previousIDs != items.map(\.id) {
            let index = items.firstIndex { $0.index == state.queueIndex } ?? 0
            target = Float(index)
            if cards.isEmpty { center = target }
            current = state.queueIndex
        }
        guard active && state.shelf && state.immersive && !items.isEmpty else { close(); return }
        let range = NativeShelfLayout.window(center:Int(target.rounded()),total:items.count)
        let indices = range.map { items[$0].index }
        if indices != published { published = indices; publish(indices) }
        let next = range.map { NativeShelfContent(item:items[$0],current:items[$0].index == current,background:p.number("shelfBgOpacity",0.79),accent:p.color("shelfAccentColor",SIMD3(repeating:1))) }
        guard next != desired else { return }
        desired = next; generation += 1; let generation = generation
        task?.cancel()
        let keep = Set(indices)
        cards = cards.filter { keep.contains($0.key) }
        let missing = next.filter { cards[$0.item.index]?.content != $0 }
        guard !missing.isEmpty else { task = nil; return }
        task = Task { [weak self] in
            // One small job at a time; never retain eleven transient pixel buffers.
            for content in missing {
                let worker = Task.detached(priority:.utility) { try NativeShelfRaster.make(content) }
                do {
                    let bitmap = try await withTaskCancellationHandler { try await worker.value } onCancel: { worker.cancel() }
                    try Task.checkCancellation()
                    guard let self, self.generation == generation else { return }
                    let descriptor = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:.rgba8Unorm,width:NativeShelfRaster.width,height:NativeShelfRaster.height,mipmapped:false)
                    descriptor.usage = .shaderRead; descriptor.storageMode = .shared
                    if let texture = self.device.makeTexture(descriptor:descriptor) {
                        bitmap.pixels.withUnsafeBytes { texture.replace(region:MTLRegionMake2D(0,0,NativeShelfRaster.width,NativeShelfRaster.height),mipmapLevel:0,withBytes:$0.baseAddress!,bytesPerRow:NativeShelfRaster.width*4) }
                        self.cards[content.item.index] = Card(content:content,texture:texture)
                    }
                } catch { return }
            }
            guard let self, self.generation == generation else { return }; self.task = nil
        }
    }
    func pointer(_ point: SIMD2<Float>?) { hoverIndex = point.flatMap { p in hits.reversed().first { $0.contains(p) }?.item.index } }
    func click(_ point: SIMD2<Float>) -> Bool {
        guard let hit = hits.reversed().first(where:{ $0.contains(point) }) else { return false }
        select(hit.item.index,hit.item.id); return true
    }
    func scroll(_ delta: Float, at point: SIMD2<Float>) -> Bool {
        guard hits.contains(where:{$0.contains(point)}) else { return false }
        target = min(Float(max(0,items.count-1)),max(0,target+max(-1,min(1,delta*0.025))))
        return true
    }
    func render(encoder: MTLRenderCommandEncoder, program: NativeProgram, mesh: NativeMesh, sampler: MTLSamplerState, state: NativeStageState, parameters p: NativeParameters, camera: NativeCamera, viewport: MTLViewport, size: SIMD2<Float>, time: Float, dt: Float) {
        guard state.shelf, state.immersive, !items.isEmpty else { hits = []; return }
        center += (target-center)*(1-exp(-dt/0.12))
        reveal = min(1,reveal+dt/max(0.08,p.number("shelfSummonOpenDuration",0.91)))
        let shelfCamera = camera.shelfCamera(preset:state.preset,fixed:p.raw["shelfCameraMode"] as? String == "static")
        let view = shelfCamera.view(), layout = NativeShelfLayout(size:size,parameters:p,preset:state.preset)
        let focus = layout.model(delta:0,index:Int(target),time:time,reveal:1,hover:0)
        let projection = NativeShelfLayout.fittedProjection(shelfCamera.projection(aspect:Float(viewport.width/max(1,viewport.height))),focus:view*focus)
        encoder.setViewport(viewport); encoder.setRenderPipelineState(program.pipeline)
        encoder.setVertexBuffer(mesh.vertices,offset:0,index:16); encoder.setFragmentSamplerState(sampler,index:1)
        hits = []
        let ordered = NativeShelfLayout.window(center:Int(target.rounded()),total:items.count).sorted { abs(Float($0)-center) > abs(Float($1)-center) }
        for index in ordered {
            let item = items[index]
            guard let card = cards[item.index] else { continue }
            let delta = Float(index)-center, distance = abs(delta)
            let presence = p.raw["shelfPresence"] as? String == "always" || p.flag("shelfPinnedOpen",false) || hoverIndex != nil
            let alpha = (distance < 0.5 ? Float(1) : max(0.22,1-distance*0.30))*p.number("shelfOpacity",1)*reveal*(presence ? 1 : 0.72)
            let model = layout.model(delta:delta,index:index,time:time,reveal:reveal,hover:hoverIndex == item.index ? 1 : 0)
            let bytes = program.uniformData(["uOpacity":[alpha]],modelView:view*model,projection:projection)
            bytes.withUnsafeBytes { encoder.setVertexBytes($0.baseAddress!,length:$0.count,index:0);encoder.setFragmentBytes($0.baseAddress!,length:$0.count,index:0) }
            encoder.setFragmentTexture(card.texture,index:1)
            encoder.drawPrimitives(type:.triangle,vertexStart:0,vertexCount:mesh.count)
            hits.append(NativeShelfHit(item:item,inverse:simd_inverse(projection*view*model)))
        }
    }
}
