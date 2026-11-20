@preconcurrency import MetalKit
import simd

/// Bounded rings/transforms from sonic-topography.js; consumes the existing eight bands.
struct NativeSonicMotion {
    private(set) var time: Float = 0
    private(set) var pulse: Float = 0
    private(set) var yaw: Float = 0
    private var previousKick = false, previousSnare = false
    private var ripples = [SIMD4<Float>](repeating:SIMD4(0,0,-100,0),count:10)
    private var cursor = 0
    private var random: UInt64 = 0x726970706c65
    private var buffers: [MTLBuffer] = []
    private var slot = 0
    private(set) var bands = [Float](repeating:0,count:8)
    private var energy: Float = 0, smoothness: Float = 1, density: Float = 0.45, sharpness: Float = 0

    mutating func close() { self = NativeSonicMotion() }
    /// Playback reconciliation clears reactions, not the visual scene's clock or rotation.
    mutating func resetAudio() {
        pulse = 0; bands = Array(repeating:0,count:8); energy = 0; smoothness = 1; density = 0.45; sharpness = 0
        previousKick = false; previousSnare = false
        ripples = Array(repeating:SIMD4(0,0,-100,0),count:10); cursor = 0
    }
    private mutating func next() -> Float {
        random = random &* 6364136223846793005 &+ 1
        return Float((random >> 32) & 0xffffff)/Float(0x1000000)
    }
    private mutating func ripple(_ strength: Float, white: Bool) -> SIMD4<Float> {
        let angle = next()*Float.pi*2, radius = white ? 10+next()*35 : next()*20
        return SIMD4(cos(angle)*radius,sin(angle)*radius,time,(white ? -1 : 1)*strength)
    }
    func applyAudioUniforms(to values: inout [String:[Float]]) {
        for (i,key) in ["SubBass","Bass","LowMid","Mid","HighMid","Presence","Brilliance","Air"].enumerated() { values["u"+key] = [bands[i]] }
        let low = bands[0]+bands[1]+bands[2]+bands[3], high = bands[5]+bands[6]+bands[7], sum = max(0.001,low+high)
        values["uWarmth"] = [low/sum]; values["uBrightness"] = [high/sum]
        values["uEnergy"] = [energy]; values["uSmoothness"] = [smoothness]; values["uDensity"] = [density]; values["uSharpness"] = [sharpness]
    }
    mutating func impact(_ value: SIMD3<Float>) {
        ripples[cursor] = SIMD4(value.x,value.y,time,-value.z)
        cursor = (cursor+1)%10
    }
    mutating func step(_ timeline: NativeFeatureTimeline, parameters p: NativeParameters, dt: Float) {
        time += dt*(0.45+p.number("sonicGroundMotionSpeed",50)*0.017)
        yaw += dt*p.number("sonicGroundAutoRotate",50)*0.003*min(1.8,max(0.35,p.number("speed",1)))
        let kick = min(0.75,timeline.beat)/0.75
        let defaults: [Float] = [90,92,50,50,50,50,50,48]
        var eqSum: Float = 0
        for (i,name) in ["SubBass","Bass","LowMid","Mid","HighMid","Presence","Brilliance","Air"].enumerated() {
            let input = i == 0 ? timeline.bands[0]*0.22+kick*1.28 : i == 1 ? timeline.bands[1]*0.20+kick*1.15 : timeline.bands[i]
            let eq = max(0,min(100,p.number("sonicGround"+name,defaults[i]))); eqSum += eq
            let delta = (eq-50)/50
            let target = max(0,min(i == 0 ? 1.2 : i == 1 ? 1.15 : 1,delta >= 0 ? input*(1+delta*1.8) : max(0,input+delta*0.35)*(1+delta*0.35)))
            bands[i] += (target-bands[i])*(1-exp(-dt*(2.2+p.number("sonicGroundMotionSpeed",50)*0.578)))
        }
        let treble = (timeline.bands[5]+timeline.bands[6]+timeline.bands[7])/3
        energy = max(0,min(1,timeline.energy*(0.25+eqSum/8/50*0.75)))
        smoothness = max(0,min(1,1-treble*0.42+timeline.bands[3]*0.14))
        density = max(0,min(1,0.45+treble*0.35+timeline.beat*0.10))
        sharpness = max(0,min(1,treble*0.70+timeline.beat*0.20))
        pulse += (timeline.beat-pulse)*(1-exp(-dt*(3+p.number("sonicGroundFloatingSpeed",77)*0.33)))
        if timeline.beat > 0.58 && !previousKick {
            let value = ripple(min(3,timeline.beat*2),white:false)
            ripples[cursor] = value; cursor = (cursor+1)%10
        }
        previousKick = timeline.beat > 0.32
        if (timeline.bands[5] > 0.52 || timeline.bands[6] > 0.56) && !previousSnare && next() < 0.55 {
            let value = ripple(min(3,(timeline.bands[5]+timeline.bands[6])*1.2),white:true)
            ripples[cursor] = value; cursor = (cursor+1)%10
        }
        previousSnare = timeline.bands[5] > 0.38 || timeline.bands[6] > 0.42
    }
    func rippleValues(workshop: Bool) -> [Float] {
        ripples.flatMap { r -> [Float] in
            let age = time-r.z, active = age >= 0 && age < 4.8 && abs(r.w) > 0.001
            let t = min(1,max(0,(age-2.1)/2.7)), strength = active ? r.w*(1-t*t*(3-2*t)) : 0
            return workshop ? [r.x,r.y,r.z,abs(strength),active ? 1 : 0,r.w < 0 ? 1 : 0,0,0] : [r.x,r.y,r.z,strength]
        }
    }
    mutating func floatingInstances(device: MTLDevice, count: Int, parameters p: NativeParameters) -> MTLBuffer? {
        guard count > 0, p.flag("sonicGroundFloatingEnabled",true) else { return nil }
        if buffers.isEmpty {
            buffers = (0..<2).compactMap { _ in device.makeBuffer(length:100*64,options:.storageModeShared) }
        }
        guard buffers.count == 2 else { return nil }
        let intensity = p.number("sonicGroundFloatingIntensity",55)/100
        let small = 0.12+p.number("sonicGroundFloatingMinSize",9)*0.0063
        let large = max(small+0.05,0.45+p.number("sonicGroundFloatingMaxSize",26)*0.0275)
        let size = small+(large-small)*min(1,pulse*(0.5+intensity*1.7))
        var transforms: [simd_float4x4] = []; transforms.reserveCapacity(count)
        for i in 0..<min(100,count) {
            let f = Float(i), angle = f/Float(count)*Float.pi*10+sin(f*12.9898)*0.7
            let radius = Float(14+(i*37)%62), height = Float(6+(i*17)%19)
            let speed = 0.18+Float((i*7)%10)*0.035, phase = f*0.73
            let position = SIMD3(cos(angle)*radius,height+sin(time*(0.55+speed)+phase)*0.45+pulse*intensity*1.4,sin(angle)*radius)
            transforms.append(nativeTransform(position:position,scale:SIMD3(repeating:(0.75+Float((i*11)%9)*0.05)*size),rotation:SIMD3(time*speed+phase,time*speed*0.7+phase,time*speed*0.45)))
        }
        let buffer = buffers[slot]; slot = (slot+1)%2
        transforms.withUnsafeBytes { buffer.contents().copyMemory(from:$0.baseAddress!,byteCount:$0.count) }
        return buffer
    }
}
