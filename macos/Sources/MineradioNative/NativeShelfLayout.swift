import Foundation
import simd

/// Side/stage poses from the pinned shelf-layout.js and shelf-manager.js.
struct NativeShelfLayout {
    let size: SIMD2<Float>
    let parameters: NativeParameters
    let preset: Int

    func model(delta: Float, index: Int, time: Float, reveal: Float, hover: Float) -> simd_float4x4 {
        let p = parameters, portrait = size.y > size.x * 1.08, narrow = size.x < 980
        let skull = preset == 6, side = (p.raw["shelf"] as? String ?? "side") != "stage"
        let distance = abs(delta), amount = p.number("shelfSize",0.92)
        let x = p.number("shelfOffsetX",-0.34), y = p.number("shelfOffsetY",-0.2), z = p.number("shelfOffsetZ",0.12)
        let angle = p.flag("shelfAngleYManual",true) ? p.number("shelfAngleY",-11) : (p.raw["shelfCameraMode"] as? String == "static" ? -15 : 0)
        let entry = (1-reveal)*(0.82+distance*0.075*p.number("shelfSummonStagger",1))*p.number("shelfSummonSlide",1.9)
        let position: SIMD3<Float>, rotation: SIMD3<Float>, scale: Float
        if side {
            let baseX: Float = skull ? (portrait ? 0.22 : narrow ? 0.46 : 0.76) : (portrait ? 1.56 : narrow ? 2.48 : 3.18)
            let baseY: Float = skull ? (portrait ? -0.22 : narrow ? -0.30 : -0.34) : 0
            position = SIMD3(baseX+x+distance*(portrait ? 0.018 : skull ? 0.034 : 0.04)+entry*(portrait ? 0.38 : 0.82)-hover*0.145,
                             baseY+y-delta*(portrait ? 0.52 : skull ? 0.62 : 0.68)+hover*0.105,
                             (portrait ? 0.78 : 0.86)+z-distance*(portrait ? 0.118 : skull ? 0.158 : 0.17)+hover*0.22)
            rotation = SIMD3(-delta*(portrait ? 0.022 : 0.042),(skull ? -0.19 : 0.28)+angle*Float.pi/180,0)
            let baseScale: Float = skull ? (portrait ? 0.84 : narrow ? 1.04 : 1.22) : (portrait ? 0.70 : narrow ? 0.86 : 1)
            scale = (distance < 0.5 ? 1.12 : max(0.55,1.04-distance*0.14))*baseScale*amount*(1-(1-reveal)*0.12)*(1+hover*0.075)
        } else {
            position = SIMD3(x+delta*(portrait ? 0.92 : narrow ? 1.22 : 1.55),y+(portrait ? -2.46 : -2.2),z+(portrait ? 0.84 : 1)-min(2,distance)*0.55)
            rotation = SIMD3(0.10-distance*0.04,-delta*0.22,0)
            scale = (distance < 0.5 ? 1.2 : max(0.45,1-distance*0.22))*(portrait ? 0.72 : narrow ? 0.86 : 1)*amount
        }
        return nativeTransform(position:position,scale:SIMD3(repeating:scale),rotation:rotation)
    }

    static func window(center: Int, total: Int) -> Range<Int> {
        let end = min(total,max(0,center-5)+11), start = max(0,end-11)
        return start..<end
    }

    static func fittedProjection(_ projection: simd_float4x4, focus: simd_float4x4) -> simd_float4x4 {
        var lo: Float = .infinity, hi: Float = -.infinity
        for x in [-1.025,Float(1.025)] { for y in [-0.5125,Float(0.5125)] {
            let clip = projection*focus*SIMD4(x,y,0,1)
            guard clip.w > 0.01 else { return projection }
            lo = min(lo,clip.x/clip.w); hi = max(hi,clip.x/clip.w)
        } }
        let s = min(1,0.78/max(0.001,hi-lo))
        lo *= s; hi *= s
        var fit = matrix_identity_float4x4
        fit[0].x = s; fit[1].y = s
        fit[3].x = hi > 0.935 ? 0.935-hi : lo < 0.165 ? 0.165-lo : 0
        return fit*projection
    }
}

struct NativeShelfHit {
    let item: NativeQueueItem
    let inverse: simd_float4x4
    func contains(_ point: SIMD2<Float>) -> Bool {
        let near = inverse*SIMD4(point.x,point.y,-1,1), far = inverse*SIMD4(point.x,point.y,1,1)
        guard abs(near.w) > 0.00001, abs(far.w) > 0.00001 else { return false }
        let a = SIMD3(near.x,near.y,near.z)/near.w, b = SIMD3(far.x,far.y,far.z)/far.w
        guard abs(b.z-a.z) > 0.00001 else { return false }
        let t = -a.z/(b.z-a.z), p = a+(b-a)*t
        return t >= 0 && t <= 1 && abs(p.x) <= 1.025 && abs(p.y) <= 0.5125
    }
}
