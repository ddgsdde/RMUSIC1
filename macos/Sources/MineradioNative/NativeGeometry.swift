@preconcurrency import MetalKit
import simd

struct NativeMesh {
    let vertices: MTLBuffer
    let count: Int
    var primitive: MTLPrimitiveType = .point
    var instances: MTLBuffer?
    var instanceCount = 1
}

extension NativeProgram {
    func backCoverGeometry(device: MTLDevice) throws -> NativeMesh {
        let count = 3000
        var random: UInt64 = 0x6261636b
        func next() -> Float { random = random &* 6364136223846793005 &+ 1;return Float((random>>32)&0xffffff)/Float(0x1000000) }
        var data = [Float](repeating:0,count:count*stride/4)
        for i in 0..<count {
            let u = next(), v = next(), z = -1.5-next()*0.4, r = next()
            for field in fields {
                let at = i*stride/4+field.offset/4
                switch field.input.name {
                case "position": data[at] = (u-0.5)*4.8;data[at+1] = (v-0.5)*4.8;data[at+2] = z
                case "aUv": data[at] = 1-u;data[at+1] = v
                case "aRand": data[at] = r
                default: break
                }
            }
        }
        guard let buffer = device.makeBuffer(bytes:data,length:data.count*4,options:.storageModeShared) else { throw NativeStageFailure.device }
        return NativeMesh(vertices:buffer,count:count)
    }
    func plane(device: MTLDevice, width: Float, height: Float) throws -> NativeMesh {
        let corners: [(Float,Float)] = [(-1,-1),(1,-1),(1,1),(-1,-1),(1,1),(-1,1)]
        var data = [Float](repeating:0,count:6*stride/4)
        for (i,corner) in corners.enumerated() { for field in fields {
            let at = i*stride/4+field.offset/4
            if field.input.name == "position" { data[at] = corner.0*width/2; data[at+1] = corner.1*height/2 }
            if field.input.name == "uv" { data[at] = (corner.0+1)/2; data[at+1] = (1-corner.1)/2 }
        } }
        guard let buffer = device.makeBuffer(bytes:data,length:data.count*4,options:.storageModeShared) else { throw NativeStageFailure.device }
        return NativeMesh(vertices:buffer,count:6,primitive:.triangle)
    }
    func pointGeometry(device: MTLDevice, grid: Int, stars: Bool = false, skull: Bool = false) throws -> NativeMesh {
        let asset: [Float]
        if skull {
            guard let path = Bundle.module.url(forResource: "Resources", withExtension: nil)?.appendingPathComponent("skull.bin") else { throw NativeStageFailure.resource("skull") }
            let data = try Data(contentsOf: path)
            asset = data.withUnsafeBytes { Array($0.bindMemory(to: Float.self)) }
        } else { asset = [] }
        let count = skull ? asset.count / 5 : stars ? 1400 : grid * grid
        let width = stride / 4
        var data = [Float](repeating: 0, count: count * width)
        var random: UInt64 = 0x1a25
        func next() -> Float {
            random = random &* 6364136223846793005 &+ 1
            return Float((random >> 32) & 0xffffff) / Float(0x1000000)
        }
        for i in 0..<count {
            let x = Float(i % grid) / Float(max(1, grid - 1)), y = Float(i / grid) / Float(max(1, grid - 1))
            let seed = next()
            for field in fields {
                let at = i * width + field.offset / 4
                switch field.input.name {
                case "position":
                    if skull { data[at] = asset[i * 5]; data[at + 1] = asset[i * 5 + 1]; data[at + 2] = asset[i * 5 + 2] }
                    else if !stars { data[at] = (x - 0.5) * 4.8; data[at + 1] = (y - 0.5) * 4.8 }
                case "aUv": data[at] = (Float(i % grid) + 0.5) / Float(grid); data[at + 1] = (Float(i / grid) + 0.5) / Float(grid)
                case "aLane", "aDepthSeed": data[at] = next()
                case "aSeed": data[at] = seed * 1000 + Float(i) * 0.37
                case "seed": data[at] = skull ? asset[i * 5 + 4] : seed
                case "kind": data[at] = asset[i * 5 + 3]
                case "aRand": data[at] = seed
                default: break
                }
            }
        }
        guard let buffer = device.makeBuffer(bytes: data, length: data.count * 4, options: .storageModeShared) else { throw NativeStageFailure.device }
        return NativeMesh(vertices: buffer, count: count)
    }

    func boxes(device: MTLDevice, grid: Int, extent: Float) throws -> NativeMesh {
        let spacing = extent / Float(grid), side = spacing * (0.9 / 1.05)
        var positions: [(SIMD3<Float>, SIMD3<Float>, SIMD2<Float>)] = []
        for normal in [SIMD3<Float>(1,0,0), SIMD3<Float>(-1,0,0), SIMD3<Float>(0,1,0), SIMD3<Float>(0,-1,0), SIMD3<Float>(0,0,1), SIMD3<Float>(0,0,-1)] {
            let tangent = abs(normal.y) > 0 ? SIMD3<Float>(1,0,0) : normalize(cross(SIMD3<Float>(0,1,0),normal))
            let bitangent = cross(normal, tangent)
            for uv in [SIMD2<Float>(0,0),SIMD2<Float>(1,0),SIMD2<Float>(1,1),SIMD2<Float>(0,0),SIMD2<Float>(1,1),SIMD2<Float>(0,1)] {
                let p = normal * 0.5 + tangent * (uv.x - 0.5) + bitangent * (uv.y - 0.5)
                positions.append((p * SIMD3<Float>(side,1,side),normal,uv))
            }
        }
        var data = [Float](repeating: 0, count: positions.count * stride / 4)
        for (i,p) in positions.enumerated() { for field in fields {
            let at = i * stride / 4 + field.offset / 4
            let values: [Float]
            switch field.input.name {
            case "position": values = [p.0.x,p.0.y,p.0.z]
            case "normal": values = [p.1.x,p.1.y,p.1.z]
            case "uv": values = [p.2.x,p.2.y]
            default: values = []
            }
            for (j,value) in values.enumerated() { data[at+j] = value }
        } }
        var transforms: [simd_float4x4] = []
        transforms.reserveCapacity(grid * grid)
        for x in 0..<grid { for z in 0..<grid {
            var matrix = matrix_identity_float4x4
            matrix.columns.3 = SIMD4(Float(x)*spacing-extent/2,0.5,Float(z)*spacing-extent/2,1)
            transforms.append(matrix)
        } }
        guard let vertices = device.makeBuffer(bytes: data, length: data.count*4, options: .storageModeShared),
              let instances = device.makeBuffer(bytes: transforms, length: transforms.count*64, options: .storageModeShared) else { throw NativeStageFailure.device }
        return NativeMesh(vertices: vertices, count: positions.count, primitive: .triangle, instances: instances, instanceCount: transforms.count)
    }
}
