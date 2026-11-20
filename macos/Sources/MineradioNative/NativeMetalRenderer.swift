@preconcurrency import MetalKit
import Foundation
import OSLog
import simd

private final class NativeDeviceBox: @unchecked Sendable {
    let device: MTLDevice
    init(_ device: MTLDevice) { self.device = device }
}

@MainActor final class NativeMetalRenderer: NSObject, MTKViewDelegate {
    let device: MTLDevice
    private let queue: MTLCommandQueue
    private let targets: NativeRenderTargets
    private let slots = DispatchSemaphore(value: 2)
    private var programs: [String: NativeProgram] = [:]
    private var meshes: [String: NativeMesh] = [:]
    private var meshGrids: [String: Int] = [:]
    private let lyrics: NativeLyrics
    private let artwork: NativeArtwork
    let shelf: NativeShelf
    private let sampler: MTLSamplerState
    private let depth: MTLDepthStencilState
    private let particleDepth: MTLDepthStencilState
    private let overlayDepth: MTLDepthStencilState
    private var compilation: Task<Void,Never>?
    private var state: NativeStageState
    private var parameters = NativeParameters("{}")
    private var lastParameters = ""
    private var lastTime = ProcessInfo.processInfo.systemUptime, clock: Float = 0
    private var occluded = false, closed = false
    private weak var view: MTKView?
    private var timeline = NativeFeatureTimeline()
    private var sonic = NativeSonicMotion()
    private var sonicTheme = NativeSonicTheme()
    private var sonicParticles = NativeSonicParticles()
    private var coverRipples = NativeCoverRipples()
    private var workshop = NativeWorkshopMotion()
    private var workshopTheme = NativeSonicTheme()
    var camera = NativeCamera()
    private let logger = Logger(subsystem: "com.rmusic.mac.client", category: "NativeStage")
    private var metricsAt = 0.0, gaps: [Double] = [], draws = 0
    private var lastActivity = ""
    private var starAlpha: Float = 0
    private var pointerTarget = SIMD2<Float>.zero, pointerParallax = SIMD2<Float>.zero
    private var pointerActive = false, burst: Float = 0

    init(view: MTKView, state: NativeStageState, failed: @escaping (String) -> Void) throws {
        guard let device = view.device, let queue = device.makeCommandQueue() else { throw NativeStageFailure.device }
        self.device = device; self.queue = queue; self.view = view; self.state = state
        targets = NativeRenderTargets(device:device)
        lyrics = NativeLyrics(device: device)
        artwork = NativeArtwork(device: device)
        shelf = NativeShelf(device:device)
        let descriptor = MTLSamplerDescriptor()
        descriptor.minFilter = .linear; descriptor.magFilter = .linear
        descriptor.sAddressMode = .clampToEdge; descriptor.tAddressMode = .clampToEdge
        sampler = device.makeSamplerState(descriptor: descriptor)!
        let depthDescriptor = MTLDepthStencilDescriptor()
        depthDescriptor.depthCompareFunction = .lessEqual; depthDescriptor.isDepthWriteEnabled = true
        depth = device.makeDepthStencilState(descriptor: depthDescriptor)!
        depthDescriptor.isDepthWriteEnabled = false
        particleDepth = device.makeDepthStencilState(descriptor: depthDescriptor)!
        depthDescriptor.depthCompareFunction = .always; depthDescriptor.isDepthWriteEnabled = false
        overlayDepth = device.makeDepthStencilState(descriptor: depthDescriptor)!
        super.init()
        camera.reset(preset: state.preset)
        let box = NativeDeviceBox(device)
        let sampleCount = targets.sampleCount
        compilation = Task { [weak self, weak view] in
            let worker = Task.detached(priority: .utility) {
                var result: [String: NativeProgram] = [:]
                for name in ["cover","bloom","stars","skull","terrain","floating","workshop","lyrics","lyricGlow","shelf","sonicParticle","backCover","lyricSpark"] {
                    try Task.checkCancellation()
                    result[name] = try NativeProgram(device: box.device,name: name,additive: ["bloom","stars","skull","lyricGlow","lyricSpark"].contains(name),premultiplied:name == "shelf",sampleCount:sampleCount)
                }
                return result
            }
            do {
                let built = try await withTaskCancellationHandler { try await worker.value } onCancel: { worker.cancel() }
                try Task.checkCancellation()
                guard let self, !self.closed else { return }
                self.programs = built
                try self.rebuildGeometry()
                self.update(self.state)
                view?.setNeedsDisplay(view?.bounds ?? .zero)
                self.compilation = nil
            } catch is CancellationError {} catch {
                Logger(subsystem: "com.rmusic.mac.client",category: "NativeStage").error("Native stage setup failed: \(String(describing:error),privacy:.public)")
                failed(String(describing:error))
            }
        }
    }
    func close() {
        closed = true; compilation?.cancel(); compilation = nil
        view?.isPaused = true; view?.delegate = nil
        view?.releaseDrawables()
        targets.release()
        programs.removeAll(); meshes.removeAll(); meshGrids.removeAll(); artwork.close(); lyrics.close(); shelf.close(); sonic.close(); sonicParticles.close(); coverRipples.reset(); timeline.reset()
    }
    func resetCamera() { camera.reset(preset: state.preset) }
    func pointer(_ point: SIMD2<Float>?) {
        shelf.pointer(point);pointerActive = point != nil
        pointerTarget = point ?? .zero
    }
    func setOccluded(_ value: Bool) {
        occluded = value
        artwork.update(state.cover,active:state.active && state.visible && !occluded)
        shelf.update(state,parameters:parameters,active:state.active && state.visible && !occluded)
        if value { lyrics.close(); sonicParticles.close(); coverRipples.reset() }
        refreshActivity()
    }
    func scrollShelf(_ delta: Float, at point: SIMD2<Float>) -> Bool {
        guard shelf.scroll(delta,at:point) else { return false }
        shelf.update(state,parameters:parameters,active:state.active && state.visible && !occluded); return true
    }
    private func refreshActivity() {
        let animate = state.shouldRenderContinuously && !occluded && !closed
        if view?.isPaused == animate {
            lastTime = ProcessInfo.processInfo.systemUptime; gaps.removeAll(keepingCapacity: true)
            if !animate { view?.releaseDrawables(); targets.release() }
        }
        view?.enableSetNeedsDisplay = !animate; view?.isPaused = !animate
        if Bundle.main.bundleIdentifier?.hasPrefix("com.rmusic.mac.client.qa") == true {
            let activity = "preset=\(state.preset), active=\(state.active), visible=\(state.visible), occluded=\(occluded), programs=\(programs.count), drawing=\(animate)"
            if activity != lastActivity { lastActivity = activity; logger.info("Native stage activity: \(activity,privacy:.public)") }
        }
    }
    func update(_ next: NativeStageState) {
        guard !closed else { return }
        var rebuild = state.density != next.density || state.preset != next.preset || state.backCover != next.backCover
        let expected = state.position+(state.playing ? max(0,next.sampledAt-state.sampledAt) : 0)
        if state.session != next.session || abs(next.position-expected) > 1 { timeline.reset(); sonic.resetAudio(); sonicParticles.close(); coverRipples.reset(); camera.resetAudio(); workshop.reset() }
        if !next.active || !next.visible || ![7,8].contains(next.preset) { sonicParticles.close() }
        if state.session != next.session || state.lyrics != next.lyrics || !next.active || !next.visible || !next.immersive { lyrics.close() }
        if state.preset != next.preset { camera.reset(preset: next.preset); sonic.close(); sonicParticles.close(); sonicTheme = NativeSonicTheme(); workshop.reset(); workshopTheme = NativeSonicTheme(); burst = 0.18 }
        state = next
        if lastParameters != next.parameters {
            let previousQuality = parameters.raw["performanceQuality"] as? String
            let previousDensity = parameters.number("sonicGroundDensity",46)
            lastParameters = next.parameters; parameters = NativeParameters(next.parameters)
            rebuild = rebuild || previousQuality != parameters.raw["performanceQuality"] as? String
            rebuild = rebuild || previousDensity != parameters.number("sonicGroundDensity",46)
        }
        artwork.update(next.cover,active: next.active && next.visible && !occluded)
        shelf.update(next,parameters:parameters,active:next.active && next.visible && !occluded)
        if next.playing { timeline.push(next.frames,session: next.session) } else { timeline.pause() }
        view?.preferredFramesPerSecond = [24,30,60].contains(next.fps) ? next.fps : 30
        refreshActivity()
        if rebuild { try? rebuildGeometry() }
        if next.visible && !occluded && next.active { view?.setNeedsDisplay(view?.bounds ?? .zero) }
    }
    private func rebuildGeometry() throws {
        let grid = max(80,min(180,state.density))
        var next: [String: NativeMesh] = [:]
        var grids: [String: Int] = [:]
        // Keep only the current scene's buffers. Entering the player changes the
        // point budget, but does not change terrain, shelf or floating geometry.
        func mesh(_ name: String, grid: Int, build: () throws -> NativeMesh) throws {
            if meshGrids[name] == grid, let existing = meshes[name] { next[name] = existing }
            else { next[name] = try build() }
            grids[name] = grid
        }
        if let p = programs["cover"], ![3,6,8].contains(state.preset) { try mesh("cover", grid: grid) { try p.pointGeometry(device: device,grid: grid) } }
        if state.backCover, ![6,8].contains(state.preset), let p = programs["backCover"] { try mesh("backCover", grid: 0) { try p.backCoverGeometry(device:device) } }
        if let p = programs["stars"] { try mesh("stars", grid: grid) { try p.pointGeometry(device: device,grid: grid,stars: true) } }
        if state.preset == 6, let p = programs["skull"] { try mesh("skull", grid: grid) { try p.pointGeometry(device: device,grid: grid,skull: true) } }
        if state.preset == 7, let p = programs["terrain"] {
            let density = parameters.number("sonicGroundDensity",46)
            let requested = Int(((96+128*density/100)/4).rounded())*4
            let cap = ["eco":112,"balanced":160,"high":192,"ultra":224][parameters.raw["performanceQuality"] as? String ?? "eco"] ?? 160
            let terrainGrid = max(96,min(cap,requested))
            try mesh("terrain", grid: terrainGrid) { try p.boxes(device:device,grid:terrainGrid,extent:168) }
            if let floating = programs["floating"] { try mesh("floating", grid: 1) { try floating.boxes(device:device,grid:1,extent:1.05/0.9) } }
            if let particles = programs["sonicParticle"] { try mesh("sonicParticle", grid: 1) { try particles.boxes(device:device,grid:1,extent:1.05/0.9) } }
        }
        if state.preset == 8, let p = programs["workshop"] {
            let workshopGrid = state.density == 80 ? 80 : state.density == 180 ? 140 : 100
            try mesh("workshop", grid: workshopGrid) { try p.boxes(device: device,grid:workshopGrid,extent:168) }
            if let particles = programs["sonicParticle"] { try mesh("sonicParticle", grid: 1) { try particles.boxes(device:device,grid:1,extent:1.05/0.9) } }
        }
        if let p = programs["shelf"] { try mesh("shelf", grid: 0) { try p.plane(device:device,width:2.05,height:1.025) } }
        meshes = next; meshGrids = grids
    }
    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}
    func draw(in view: MTKView) { autoreleasepool { render(view) } }
    private func sonicModel() -> simd_float4x4 {
        let scale = 0.096+parameters.number("sonicGroundRange",82)*0.00072
        return nativeTransform(position:SIMD3(0,-4.05-parameters.number("sonicGroundLower",68)*0.034,-4.2-parameters.number("sonicGroundDepth",62)*0.055),scale:SIMD3(repeating:scale),rotation:SIMD3(0,sonic.yaw,0))
    }
    private func render(_ view: MTKView) {
        guard !closed, state.visible, !occluded, !programs.isEmpty, slots.wait(timeout: .now()) == .success else { return }
        var committed = false
        defer { if !committed { slots.signal() } }
        guard let drawable = view.currentDrawable, let command = queue.makeCommandBuffer() else { return }
        let now = ProcessInfo.processInfo.systemUptime, rawGap = now-lastTime
        let dt = Float(min(0.08,max(0,rawGap))); lastTime = now
        let motionDT = state.motion ? dt : 0
        if state.active && state.motion { clock += dt }
        pointerParallax += (pointerTarget-pointerParallax)*(1-pow(0.96,motionDT*60))
        burst *= exp(-motionDT*6)
        camera.step(motionDT); artwork.step(dt)
        let starTarget = nativeStarRiverAlpha(preset:state.preset,enabled:parameters.flag("backgroundStarRiver",true))
        starAlpha += (starTarget-starAlpha)*(1-pow(1-(starTarget > starAlpha ? 0.085 : 0.16),motionDT*60))
        let position = state.position + (state.playing ? min(2,max(0,now-state.sampledAt)) : 0)
        if state.motion {
        timeline.step(position: position,playing: state.playing,dt: dt)
        camera.animate(timeline,position:position,playing:state.playing,time:clock,dt:dt,parameters:parameters)
        coverRipples.step(bass:(timeline.bands[0]+timeline.bands[1])*0.5,dt:dt)
        if [7,8].contains(state.preset) { sonic.step(timeline,parameters:parameters,dt:dt) }
        if state.preset == 7 {
            sonicParticles.step(beat:timeline.beat,time:sonic.time,dt:dt)
            for impact in sonicParticles.impacts { sonic.impact(impact) }
        }
        if state.preset == 8 {
            workshop.step(timeline.measuredBands,time:clock,dt:dt,gain:parameters.number("sonicWorkshopInputGain",82)/82)
            if let strength = workshop.meteorStrength { sonicParticles.addMeteor(strength:strength,time:clock,workshop:true) }
            sonicParticles.step(beat:0,time:clock,dt:dt,workshop:true)
            for impact in sonicParticles.impacts { workshop.impact(impact,time:clock) }
        }
        }
        let tint = parameters.raw["visualTintMode"] as? String == "custom" ? parameters.color("visualTintColor",artwork.tint) : artwork.tint
        var values = nativeUniforms(state: state,parameters: parameters,timeline: timeline,time: clock,pixel: Float(view.drawableSize.width/max(1,view.bounds.width)),hasCover: artwork.textures["uCoverTex"] != nil,tint: tint,palette: artwork.palette,hasDepth: artwork.textures["uEdgeTex"] != nil,coverBlend: artwork.blend)
        if state.preset == 7 { sonicTheme.apply(to:&values,dt:dt) }
        if state.preset == 8 { workshopTheme.apply(to:&values,dt:dt);workshop.apply(to:&values,time:clock) }
        let base = values["uBaseColor1"] ?? [0.025,0.03,0.045]
        let backdrop = state.preset == 8 ? nativeWorkshopBackdrop(SIMD3(base[0],base[1],base[2])) : SIMD3(0.025,0.03,0.045)
        guard let pass = targets.pass(resolving:drawable.texture,clear:MTLClearColorMake(Double(backdrop.x),Double(backdrop.y),Double(backdrop.z),1)),
              let encoder = command.makeRenderCommandEncoder(descriptor:pass) else { return }
        values["uRippleCount"] = [Float(coverRipples.count)]
        values["uMouseXY"] = [pointerTarget.x*2.4,pointerTarget.y*2.4]
        values["uMouseActive"] = [pointerActive ? 1 : 0];values["uBurstAmt"] = [burst]
        let rippleTexture = coverRipples.count > 0 ? coverRipples.texture(device:device) : nil
        let projection = camera.projection(aspect: Float(max(1,view.drawableSize.width)/max(1,view.drawableSize.height)))
        let viewMatrix = camera.view(time: clock,cinema: parameters.flag("cinema",true))
        encoder.setFrontFacing(.counterClockwise)
        for name in ["stars","backCover","bloom","cover","skull","terrain","floating","workshop"] {
            guard let program = programs[name], let mesh = meshes[name == "bloom" ? "cover" : name] else { continue }
            if name == "cover" || name == "bloom" {
                if [3,6,8].contains(state.preset) || ([5,7].contains(state.preset) && !parameters.flag("backgroundStarRiver",true)) { continue }
                if name == "bloom" && !parameters.flag("bloom",false) { continue }
            }
            if name == "stars" && starAlpha < 0.006 { continue }
            values["uAlpha"] = [name == "stars" ? starAlpha : ["cover","bloom","backCover"].contains(name) && artwork.textures["uCoverTex"] == nil ? 0 : 1]
            let model: simd_float4x4
            if name == "skull" { model = nativeTransform(position: SIMD3(0,0.22,0.1),scale: SIMD3(repeating: 2.34),rotation: SIMD3(-0.26,0,0)) }
            else if name == "terrain" || name == "floating" {
                model = sonicModel()
            } else if ["cover","bloom","backCover"].contains(name) { model = nativeTransform(rotation:SIMD3(-pointerParallax.y*0.04,pointerParallax.x*0.04,0)) }
            else { model = matrix_identity_float4x4 }
            let floatingCount = max(0,min(100,Int(parameters.number("sonicGroundFloatingCount",80))))
            var instanceBuffer = mesh.instances, instanceCount = mesh.instanceCount
            if name == "floating" {
                guard let buffer = sonic.floatingInstances(device:device,count:floatingCount,parameters:parameters) else { continue }
                instanceBuffer = buffer; instanceCount = floatingCount
            }
            values["uTime"] = [["terrain","floating"].contains(name) ? sonic.time : clock]
            if ["terrain","floating"].contains(name) {
                sonic.applyAudioUniforms(to:&values)
            }
            values["uPulse"] = [sonic.pulse]
            if name != "workshop" { values["uRipples"] = sonic.rippleValues(workshop:false) }
            encoder.setDepthStencilState(["terrain","workshop"].contains(name) ? depth : name == "floating" ? particleDepth : overlayDepth)
            encoder.setCullMode(program.cullMode)
            encoder.setRenderPipelineState(program.pipeline)
            encoder.setVertexBuffer(mesh.vertices,offset: 0,index: 16)
            encoder.setVertexBuffer(instanceBuffer,offset: 0,index: 17)
            let bytes = program.uniformData(values,modelView: viewMatrix*model,projection: projection,model: model,view: viewMatrix)
            bytes.withUnsafeBytes { encoder.setVertexBytes($0.baseAddress!,length: $0.count,index: 0); encoder.setFragmentBytes($0.baseAddress!,length: $0.count,index: 0) }
            for item in program.manifest.textures {
                let texture = (item.name == "uRippleTex" ? rippleTexture : artwork.textures[item.name == "uMap" ? "uDotTex" : item.name]) ?? artwork.textures["black"]
                encoder.setVertexTexture(texture,index: item.binding); encoder.setFragmentTexture(texture,index: item.binding)
                encoder.setVertexSamplerState(sampler,index: item.binding); encoder.setFragmentSamplerState(sampler,index: item.binding)
            }
            encoder.drawPrimitives(type: mesh.primitive,vertexStart: 0,vertexCount: mesh.count,instanceCount: instanceCount)
        }
        if [7,8].contains(state.preset), let program = programs["sonicParticle"], let mesh = meshes["sonicParticle"] {
            encoder.setDepthStencilState(particleDepth)
            let warm = values["uWarmCore"] ?? [1,1,1]
            sonicParticles.render(encoder:encoder,program:program,mesh:mesh,device:device,modelView:state.preset == 7 ? viewMatrix*sonicModel() : viewMatrix,projection:projection,warm:warm,accent:state.preset == 8 ? warm.map { $0*0.3+0.7 } : values["uRippleColor"] ?? [0.66,0.93,1])
        }
        if state.immersive, parameters.flag("particleLyrics",true), let program = programs["lyrics"] {
            encoder.setCullMode(.none)
            encoder.setDepthStencilState(overlayDepth)
            let pixel = view.drawableSize.width/max(1,view.bounds.width)
            let top = max(0,state.stageTop*pixel), bottom = max(0,state.stageBottom*pixel)
            let viewport = MTLViewport(originX:0,originY:top,width:view.drawableSize.width,height:max(1,view.drawableSize.height-top-bottom),znear:0,zfar:1)
            var lyricCamera = camera
            if state.preset == 8 || parameters.flag("lyricCameraLock",false) { lyricCamera.reset(preset:0) }
            lyrics.render(encoder:encoder,program:program,glow:programs["lyricGlow"],sampler:sampler,state:state,parameters:parameters,position:position,time:clock,dt:dt,viewport:viewport,camera:lyricCamera,palette:artwork.palette,beat:timeline.beat,energy:timeline.energy,sparks:programs["lyricSpark"],dot:artwork.textures["uDotTex"],bass:(timeline.bands[0]+timeline.bands[1])*0.5,mid:(timeline.bands[2]+timeline.bands[3]+timeline.bands[4])/3,pixel:Float(pixel))
        }
        if let program = programs["shelf"], let mesh = meshes["shelf"] {
            encoder.setCullMode(.none)
            encoder.setDepthStencilState(overlayDepth)
            shelf.render(encoder:encoder,program:program,mesh:mesh,sampler:sampler,state:state,parameters:parameters,camera:camera,viewport:MTLViewport(originX:0,originY:0,width:view.drawableSize.width,height:view.drawableSize.height,znear:0,zfar:1),size:SIMD2(Float(view.bounds.width),Float(view.bounds.height)),time:clock,dt:dt)
        }
        encoder.endEncoding(); command.present(drawable)
        let semaphore = slots
        command.addCompletedHandler { _ in semaphore.signal() }; command.commit(); committed = true
        if Bundle.main.bundleIdentifier?.hasPrefix("com.rmusic.mac.client.qa") == true {
            draws += 1; gaps.append(rawGap*1000); if gaps.count > 240 { gaps.removeFirst() }
            if now-metricsAt > 5 {
                metricsAt = now
                let sorted = gaps.sorted(), p95 = sorted.isEmpty ? 0 : sorted[Int(Double(sorted.count-1)*0.95)]
                logger.info("Native stage: draws=\(self.draws), playing=\(self.state.playing), position=\(position), featureAge=\(self.timeline.samplePosition.map { position - $0 } ?? -1), beat=\(self.timeline.beat), frameP95=\(p95), frameMax=\(sorted.last ?? 0), programs=\(self.programs.count), meshes=\(self.meshes.count), glyphs=\(self.lyrics.textureCount), rasters=\(self.lyrics.rasterizedCount), shelf=\(self.shelf.textureCount), meteors=\(self.sonicParticles.meteorCount), trails=\(self.sonicParticles.trailCount)")
            }
        }
    }
}
