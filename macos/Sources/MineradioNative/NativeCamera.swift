import simd

struct NativeCamera {
    private var cinemaMotion = NativeCinemaMotion()
    private var cinemaOffset = SIMD3<Float>.zero
    private var fov: Float = 45
    private var workshop = false, manual = false
    private var yaw: Float = 0, pitch: Float = 0.08, radius: Float = 6.6
    private var targetYaw: Float = 0, targetPitch: Float = 0.08, targetRadius: Float = 6.6
    mutating func reset(preset: Int) {
        resetAudio()
        workshop = preset == 8; manual = false
        let values: (Float, Float, Float)
        switch preset {
        case 1: values = (0,0.03,6.2)
        case 2: values = (0,0.15,7)
        case 3: values = (0,0.05,8)
        case 4: values = (0,0.04,6.5)
        case 6: values = (0.18,0.1,7.4)
        case 7: values = (0,0.18,8.4)
        case 8: values = (150*Float.pi/180,30*Float.pi/180,80)
        case 9: values = (-0.08,0.12,7.4)
        case 10: values = (0,0.02,7.15)
        case 11: values = (0.1,0.11,7)
        case 12: values = (-0.12,0.18,7.35)
        default: values = (0,0.08,6.6)
        }
        yaw = values.0; targetYaw = yaw; pitch = values.1; targetPitch = pitch; radius = values.2; targetRadius = radius
    }
    mutating func resetAudio() { cinemaMotion.reset(); cinemaOffset = .zero; fov = 45 }
    mutating func animate(_ timeline: NativeFeatureTimeline, position: Double, playing: Bool, time: Float, dt: Float, parameters p: NativeParameters) {
        cinemaMotion.step(timeline,position:position,playing:playing,dt:dt)
        let shake = !workshop && p.flag("cinema",true) ? max(0,min(1.8,p.number("cinemaShake",0.5))) : 0
        cinemaOffset = SIMD3(sin(time*0.08)*0.012,sin(time*0.06+1)*0.010+cinemaMotion.phi,sin(time*0.04+2)*0.080-cinemaMotion.radius*1.18)*shake
        let target = 45-cinemaMotion.punch*0.54*shake*2.35
        fov += (target-fov)*(1-pow(1-(target < fov ? 0.24 : 0.12),dt*60))
    }
    mutating func drag(x: Float, y: Float) { manual = true;targetYaw -= x * (workshop ? 0.25*Float.pi/180 : 0.006); targetPitch = max(-1.3,min(1.3,targetPitch+y*(workshop ? 0.2*Float.pi/180 : 0.005))) }
    mutating func zoom(_ delta: Float) { targetRadius = max(2.8,min(180,targetRadius*exp(delta*0.0018))) }
    mutating func step(_ dt: Float) {
        if workshop && !manual { targetYaw += dt*7*Float.pi/180 }
        yaw += (targetYaw-yaw)*(1-exp(-dt*6.32)); pitch += (targetPitch-pitch)*(1-exp(-dt*6.32)); radius += (targetRadius-radius)*(1-exp(-dt*4.35))
    }
    func shelfCamera(preset: Int, fixed: Bool) -> NativeCamera {
        var value = self
        if fixed || preset == 8 { value.reset(preset:0) }
        else {
            var baseline = NativeCamera(); baseline.reset(preset:preset)
            let ratio = 6.6/baseline.radius
            value.radius *= ratio; value.targetRadius *= ratio
        }
        return value
    }
    func view(time: Float = 0, cinema: Bool = true) -> simd_float4x4 {
        let theta = yaw + (cinema ? cinemaOffset.x : 0), phi = pitch + (cinema ? cinemaOffset.y : 0)
        let r = max(2.4,radius+(cinema ? cinemaOffset.z : 0))
        let eye = SIMD3<Float>(sin(theta)*cos(phi)*r,sin(phi)*r,cos(theta)*cos(phi)*r)
        let z = normalize(eye), x = normalize(cross(SIMD3<Float>(0,1,0),z)), y = cross(z,x)
        return simd_float4x4(SIMD4(x.x,y.x,z.x,0),SIMD4(x.y,y.y,z.y,0),SIMD4(x.z,y.z,z.z,0),SIMD4(-dot(x,eye),-dot(y,eye),-dot(z,eye),1))
    }
    func projection(aspect: Float) -> simd_float4x4 {
        let f: Float = 1/tan(fov*Float.pi/360), n: Float = 0.1, z: Float = 1000
        return simd_float4x4(SIMD4(f/aspect,0,0,0),SIMD4(0,f,0,0),SIMD4(0,0,-(z+n)/(z-n),-1),SIMD4(0,0,-2*z*n/(z-n),0))
    }
}
