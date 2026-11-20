@preconcurrency import MetalKit
import CoreText
import Foundation
import simd

private struct NativeGlyphBitmap: Sendable {
    let stops: [Float]
    let index: Int
    let translation: Bool
    let width: Int
    let height: Int
    let pixels: [UInt8]
}
private struct NativeGlyphRow {
    let stops: [Float]
    let index: Int
    let translation: Bool
    let texture: MTLTexture
    let mesh: NativeMesh
    let width: Float
    let height: Float
}

/// CoreText masks are bounded to the visible context. No whole-song GPU cache or per-frame text layout.
@MainActor final class NativeLyrics {
    private let device: MTLDevice
    private var cache: [String: NativeGlyphRow] = [:]
    private var rows: [NativeGlyphRow] = []
    private(set) var rasterizedCount = 0
    var textureCount: Int { cache.count }
    var pending: Bool { task != nil }
    private var task: Task<Void,Never>?
    private var lastSession = "", lastSettings = ""
    private var lastIndex = -2
    private var generation = 0
    private var motion = NativeLyricMotion()
    private var cursor = NativeLyricCursor()
    private var effects = NativeLyricEffects()
    private var light = NativeLyricLight()
    private let sparkBuffers = NativeLyricSparks()
    private var layout = NativeLyricLayout()
    private var layoutSettings: String?
    init(device: MTLDevice) { self.device = device }
    func close() { generation += 1; task?.cancel(); task = nil; rows = []; cache = [:]; lastIndex = -2; motion.reset(); cursor.reset(); effects = NativeLyricEffects(); light = NativeLyricLight();sparkBuffers.close();layoutSettings = nil }

    private func count(_ p: NativeParameters) -> Int {
        switch p.raw["lyricDisplayMode"] as? String ?? "cinema" {
        case "single": 1
        case "dual": 2
        case "triple": 3
        case "custom": max(1,min(10,Int(p.number("lyricCustomLineCount",10))))
        default: 5
        }
    }
    func prepare(lines: [NativeLyricLine], position: Double, session: String, parameters p: NativeParameters, settings: String, program: NativeProgram, playing: Bool = true) -> Int {
        if session != lastSession { cursor.reset() }
        let index = cursor.select(lines: lines, position: position, playing: playing)
        guard index >= 0 else { return index }
        guard index != lastIndex || session != lastSession || settings != lastSettings else { return index }
        let changedStyle = session != lastSession || settings != lastSettings
        if changedStyle { cache = [:]; rows = []; motion.reset() }
        lastIndex = index; lastSession = session; lastSettings = settings; generation += 1
        let generation = generation
        task?.cancel(); task = nil
        let total = count(p), activeSlot = total == 2 ? 0 : total/2
        // Keep the previous edge while scrolling and prewarm two future lines.
        let start = max(0,index-activeSlot-1), end = min(lines.count,index+(total-activeSlot)+2)
        let translations = p.raw["lyricTranslationMode"] as? String ?? "multi"
        var needed: [(Int,Bool,String)] = []
        for i in start..<end {
            needed.append((i,false,lines[i].text))
            if translations != "off", let text = lines[i].translation, !text.isEmpty { needed.append((i,true,text)) }
        }
        let keys = Set(needed.map { "\($0.0):\($0.1)" })
        cache = cache.filter { keys.contains($0.key) }
        motion.retain(Set(needed.map { $0.0*2+($0.1 ? 1 : 0) }))
        refreshRows(index:index,total:total,activeSlot:activeSlot,translations:translations)
        let missing = needed.filter { cache["\($0.0):\($0.1)"] == nil }
        guard !missing.isEmpty else { return index }
        let font = p.raw["lyricFont"] as? String ?? "sans"
        let weight = p.number("lyricWeight",750), spacing = p.number("lyricLetterSpacing",0)
        task = Task { [weak self] in
            for (i,translation,text) in missing {
                let worker = Task.detached(priority:.utility) {
                    try Task.checkCancellation()
                    return Self.mask(text,index:i,translation:translation,font:font,weight:weight,spacing:spacing)
                }
                do {
                    let bitmap = try await withTaskCancellationHandler { try await worker.value } onCancel: { worker.cancel() }
                    try Task.checkCancellation()
                    guard let self, self.generation == generation else { return }
                    if let bitmap, let row = self.row(bitmap,program:program) {
                        self.cache["\(i):\(translation)"] = row; self.rasterizedCount += 1
                        self.refreshRows(index:index,total:total,activeSlot:activeSlot,translations:translations)
                    }
                } catch { return }
            }
            guard let self, self.generation == generation else { return }; self.task = nil
        }
        return index
    }
    private func refreshRows(index: Int, total: Int, activeSlot: Int, translations: String) {
        let first = max(0,index-activeSlot-1), end = index+(total-activeSlot)+1
        rows = cache.values.filter { row in
            guard row.index >= first, row.index < end else { return false }
            return !row.translation || translations == "multi" || (translations == "current" && row.index == index) || (translations == "dual" && (row.index == index || row.index == index+1))
        }.sorted { $0.index == $1.index ? !$0.translation && $1.translation : $0.index < $1.index }
    }
    nonisolated private static func mask(_ text: String, index: Int, translation: Bool, font name: String, weight: Float, spacing: Float) -> NativeGlyphBitmap? {
        let fontName = name == "song" ? "SongtiSC-Bold" : name == "mono" ? "Menlo-Bold" : weight >= 650 ? "PingFangSC-Semibold" : "PingFangSC-Regular"
        var size: CGFloat = 120
        func line(_ size: CGFloat) -> CTLine {
            let font = CTFontCreateWithName(fontName as CFString,size,nil)
            let attributes: [NSAttributedString.Key: Any] = [NSAttributedString.Key(kCTFontAttributeName as String): font,NSAttributedString.Key(kCTForegroundColorAttributeName as String): CGColor(gray:1,alpha:1),NSAttributedString.Key(kCTKernAttributeName as String): CGFloat(spacing)*size]
            return CTLineCreateWithAttributedString(NSAttributedString(string:String(text.prefix(2000)),attributes:attributes))
        }
        var shaped = line(size), bounds = CTLineGetImageBounds(shaped,nil)
        if bounds.width > 2000 { size *= 2000/bounds.width; shaped = line(size); bounds = CTLineGetImageBounds(shaped,nil) }
        let width = min(2048,max(16,Int(ceil(bounds.width))+24)), height = min(256,max(16,Int(ceil(bounds.height))+24))
        var pixels = [UInt8](repeating:0,count:width*height*4)
        let okay = pixels.withUnsafeMutableBytes { bytes -> Bool in
            guard let context = CGContext(data:bytes.baseAddress,width:width,height:height,bitsPerComponent:8,bytesPerRow:width*4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            context.textPosition = CGPoint(x:12-bounds.minX,y:12-bounds.minY)
            CTLineDraw(shaped,context)
            return true
        }
        let length = CTLineGetStringRange(shaped).length
        let stops = (0...length).map { Float(max(0,min(1,(CTLineGetOffsetForStringIndex(shaped,$0,nil)-bounds.minX)/max(1,bounds.width)))) }
        return okay ? NativeGlyphBitmap(stops:stops,index:index,translation:translation,width:width,height:height,pixels:pixels) : nil
    }
    private func row(_ bitmap: NativeGlyphBitmap, program: NativeProgram) -> NativeGlyphRow? {
        let descriptor = MTLTextureDescriptor.texture2DDescriptor(pixelFormat:.rgba8Unorm,width:bitmap.width,height:bitmap.height,mipmapped:false)
        descriptor.usage = .shaderRead; descriptor.storageMode = .shared
        guard let texture = device.makeTexture(descriptor:descriptor) else { return nil }
        bitmap.pixels.withUnsafeBytes { texture.replace(region:MTLRegionMake2D(0,0,bitmap.width,bitmap.height),mipmapLevel:0,withBytes:$0.baseAddress!,bytesPerRow:bitmap.width*4) }
        // Match the reference's readable glyph size. Fit the whole row in the
        // projection below; clamping width here squeezed long CJK lines.
        let height: Float = bitmap.translation ? 0.54 : 0.90
        let width = height*Float(bitmap.width)/Float(bitmap.height)
        let corners: [(Float,Float,Float,Float)] = [(-1,-1,0,0),(1,-1,1,0),(1,1,1,1),(-1,-1,0,0),(1,1,1,1),(-1,1,0,1)]
        var vertices = [Float](repeating:0,count:6*program.stride/4)
        for (i,corner) in corners.enumerated() { for field in program.fields {
            let at = i*program.stride/4+field.offset/4
            if field.input.name == "position" { vertices[at] = corner.0*width/2; vertices[at+1] = corner.1*height/2 }
            else if field.input.name == "uv" { vertices[at] = corner.2; vertices[at+1] = 1-corner.3 }
        } }
        guard let buffer = device.makeBuffer(bytes:vertices,length:vertices.count*4,options:.storageModeShared) else { return nil }
        return NativeGlyphRow(stops:bitmap.stops,index:bitmap.index,translation:bitmap.translation,texture:texture,mesh:NativeMesh(vertices:buffer,count:6,primitive:.triangle),width:width,height:height)
    }
    func render(encoder: MTLRenderCommandEncoder, program: NativeProgram, glow: NativeProgram?, sampler: MTLSamplerState, state: NativeStageState, parameters p: NativeParameters, position: Double, time: Float, dt: Float, viewport: MTLViewport, camera: NativeCamera, palette: NativeCoverPalette, beat: Float = 0, energy: Float = 0, sparks: NativeProgram? = nil, dot: MTLTexture? = nil, bass: Float = 0, mid: Float = 0, pixel: Float = 1) {
        if !state.playing && !p.flag("lyricPauseHold",true) {
            if !rows.isEmpty || task != nil { close() };return
        }
        light.step(time:time,dt:dt,energy:energy,beat:beat,parameters:p)
        if !p.flag("lyricGlowParticles",false) || state.shelf { sparkBuffers.close() }
        let index = prepare(lines:state.lyrics,position:position-state.lyricOffset,session:state.session,parameters:p,settings:p.lyricRasterKey,program:program,playing:state.playing)
        guard index >= 0 else { return }
        if layoutSettings != state.parameters {
            layout = NativeLyricLayout(lines:state.lyrics,parameters:p); layoutSettings = state.parameters
        }
        let slot = max(0.25,layout.primary[index+1]-layout.primary[index])
        motion.step(target:layout.primary[index],slot:slot,dt:dt)
        guard !rows.isEmpty else { return }
        let offset = motion.offset ?? layout.primary[index]
        let style = NativeLyricStyle(p)
        let jitter = effects.step(time:time,dt:dt,beat:beat,parameters:p)
        let view = camera.view(), scale = p.number("lyricScale",1)*(state.shelf ? 0.72 : 1)
        let translationScale = p.number("lyricTranslationScale",0.65)/0.65
        let primary = p.raw["lyricColorMode"] as? String == "custom" ? p.color("lyricColor",palette.lyric) : palette.lyric
        let highlight = p.raw["lyricHighlightMode"] as? String == "custom" ? p.color("lyricHighlightColor",palette.highlight) : palette.highlight
        let tint = p.flag("lyricGlowLinked",true) ? palette.secondary : p.color("lyricGlowColor",palette.secondary)
        var projection = camera.projection(aspect:Float(viewport.width/max(1,viewport.height)))
        let total = count(p), activeSlot = total == 2 ? 0 : total/2
        // Fit the authored track envelope once, not the changing set of visible glyphs.
        // Otherwise adding a long row or losing an edge row zooms the entire scene on a line boundary.
        if p.flag("lyricLiveViewportFit",true) {
            let reserve = layout.translationGap*layout.translationStep
            let height = (Float(min(total,state.lyrics.count)-1)*layout.envelopeSlot*layout.lineStep+reserve+0.9)*scale
            let top = projection*view*SIMD4<Float>(0,height/2+0.2,1.515,1)
            let bottom = projection*view*SIMD4<Float>(0,-height/2+0.2,1.515,1)
            let fit = min(1,1.76/max(0.01,abs(top.y/top.w-bottom.y/bottom.w)))
            projection[0] *= fit; projection[1] *= fit
        }
        var models: [simd_float4x4] = [], poses: [NativeLyricMotion.Pose] = []
        for row in rows {
            let active = row.index == index
            let distance = min(5.5,abs(Float(index-row.index)))
            let virtual = layout.primary[row.index]
            let y = (offset-virtual)*layout.lineStep-(row.translation ? layout.translationGap*layout.translationStep : 0)
            let floatOn = p.flag("lyricVerticalFloat",true)
            let drift = active || !floatOn ? 0 : sin(time*0.42+Float(row.index)*0.71)*style.contextDrift*0.5
            var size = scale*(row.translation ? translationScale : 1)*(active ? 1 : max(0.84,1-distance*0.026))
            let position = SIMD3(p.number("lyricOffsetX",0)+jitter.x-(state.shelf ? 1.36 : 0),(y+drift)*scale+0.20+p.number("lyricOffsetY",0)+jitter.y,1.515-pow(active ? 0 : distance,1.06)*0.145+p.number("lyricOffsetZ",0)+(state.shelf ? 0.72 : 0))
            let rotation = SIMD3(p.number("lyricTiltX",0)*Float.pi/180,p.number("lyricTiltY",0)*Float.pi/180,floatOn ? sin(time*0.34)*0.018*style.floatAmplitude : 0)
            if p.flag("lyricLiveViewportFit",true) {
                let model = nativeTransform(position:position,scale:SIMD3(repeating:size),rotation:rotation)
                var extent: Float = 0
                for x in [Float(-1),1] {
                    let clip = projection*view*model*SIMD4<Float>(x*row.width/2,0,0,1)
                    if clip.w > 0.01 { extent = max(extent,abs(clip.x/clip.w)) }
                }
                size *= min(1,0.92/max(0.01,extent))
            }
            let allowed = row.index >= index-activeSlot && row.index < index+(total-activeSlot)
            let opacity: Float = !allowed ? 0 : active ? 1 : p.number("lyricContextOpacity",0.54)/pow(1.45,distance)
            let pose = motion.pose(key:row.index*2+(row.translation ? 1 : 0),y:position.y,z:position.z,scale:size,opacity:opacity,active:active,slot:slot,lineStep:layout.lineStep*scale,dt:dt)
            poses.append(pose)
            models.append(nativeTransform(position:SIMD3(position.x,pose.y,pose.z),scale:SIMD3(repeating:pose.scale),rotation:rotation))
        }
        encoder.setViewport(viewport); encoder.setRenderPipelineState(program.pipeline)
        encoder.setFragmentSamplerState(sampler,index:1)
        for (i,row) in rows.enumerated() {
            guard state.lyrics.indices.contains(row.index) else { continue }
            let active = row.index == index, line = state.lyrics[row.index]
            let progress = nativeLyricProgress(line:line,position:position-state.lyricOffset,stops:row.stops,translation:row.translation)
            let pose = poses[i]
            let center = projection*view*models[i]*SIMD4<Float>(0,0,0,1)
            let feather = max(0,min(1,p.number("lyricEdgeFade",0.32)))
            let fade = feather == 0 ? 1 : max(0,min(1,(0.98-abs(center.y/max(0.01,center.w)))/(0.08+feather*0.125)))
            let opacity = pose.opacity*fade*fade*(3-2*fade)
            if let glow, p.flag("lyricGlow",true), p.number("lyricGlowStrength",0.28) > 0 {
                let values = ["uOpacity":[opacity*p.number("lyricGlowStrength",0.28)*style.glow*(active ? 1.8+light.beat*0.24 : 0.45)],"uColor":[tint.x,tint.y,tint.z],"uRadius":[6/Float(row.texture.width),6/Float(row.texture.height)]]
                let data = glow.uniformData(values,modelView:view*models[i],projection:projection)
                encoder.setRenderPipelineState(glow.pipeline)
                data.withUnsafeBytes { encoder.setVertexBytes($0.baseAddress!,length:$0.count,index:0); encoder.setFragmentBytes($0.baseAddress!,length:$0.count,index:0) }
                encoder.setVertexBuffer(row.mesh.vertices,offset:0,index:16); encoder.setFragmentTexture(row.texture,index:1)
                encoder.drawPrimitives(type:.triangle,vertexStart:0,vertexCount:row.mesh.count)
            }
            encoder.setRenderPipelineState(program.pipeline)
            let values: [String:[Float]] = ["uTime":[time],"uProgress":[active ? progress : 0],"uSolar":[active ? light.solar : 0],"uTextMin":[12/Float(row.texture.width)],"uTextMax":[1-12/Float(row.texture.width)],"uOpacity":[opacity*(row.translation ? p.number("lyricTranslationOpacity",0.86) : 1)],"uFeather":[line.words.isEmpty ? 0.055 : 0.03],"uBaseColor":[primary.x,primary.y,primary.z],"uHiColor":[highlight.x,highlight.y,highlight.z],"uGlowColor":[tint.x,tint.y,tint.z],"uSolarColor":[highlight.x,highlight.y,highlight.z],"uActiveMix":[pose.active],"uEdgeBoost":[style.edge],"uShimmer":[style.shimmer],"uSweep":[style.sweep],"uGlitch":[style.name == "glitch" ? p.number("lyricGlitchIntensity",1) : 0],"uGlitchSlice":[p.number("lyricGlitchSlice",0.72)],"uGlitchChroma":[p.number("lyricGlitchChroma",0.86)],"uGlitchRate":[p.number("lyricGlitchRate",1)],"uGlitchSeed":[effects.seed],"uGlitchBurst":[effects.pulse]]
            let bytes = program.uniformData(values,modelView:view*models[i],projection:projection)
            bytes.withUnsafeBytes { encoder.setVertexBytes($0.baseAddress!,length:$0.count,index:0); encoder.setFragmentBytes($0.baseAddress!,length:$0.count,index:0) }
            encoder.setVertexBuffer(row.mesh.vertices,offset:0,index:16); encoder.setFragmentTexture(row.texture,index:1)
            encoder.drawPrimitives(type:.triangle,vertexStart:0,vertexCount:row.mesh.count)
            if active, !row.translation, !state.shelf, p.flag("lyricGlow",true), p.flag("lyricGlowParticles",false), let sparks, let dot,
               let buffer = sparkBuffers.prepare(device:device,program:sparks,width:row.width,height:row.height,time:time,beat:light.beat,bass:bass,mid:mid) {
                let drive = min(1.6,p.number("lyricGlowStrength",0.28)/0.5)
                let data = sparks.uniformData(["uOpacity":[opacity*min(0.42,(0.10+light.bloom*0.14+light.beat*0.10)*drive)],"uSize":[0.050+light.bloom*0.016+light.beat*0.026+bass*0.008],"uPixel":[Float(pixel)],"uColor":[highlight.x,highlight.y,highlight.z]],modelView:view*models[i],projection:projection)
                encoder.setRenderPipelineState(sparks.pipeline);encoder.setVertexBuffer(buffer,offset:0,index:16)
                encoder.setFragmentTexture(dot,index:1);encoder.setFragmentSamplerState(sampler,index:1)
                data.withUnsafeBytes { encoder.setVertexBytes($0.baseAddress!,length:$0.count,index:0);encoder.setFragmentBytes($0.baseAddress!,length:$0.count,index:0) }
                encoder.drawPrimitives(type:.point,vertexStart:0,vertexCount:132)
            }
        }
    }
}
