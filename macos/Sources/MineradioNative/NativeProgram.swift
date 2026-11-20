@preconcurrency import MetalKit
import Foundation
import simd

enum NativeStageFailure: Error { case resource(String), device }
struct ShaderManifest: Decodable {
    struct Texture: Decodable { let name: String; let binding: Int }
    struct Input: Decodable { let type: String; let name: String; let location: Int }
    struct Member: Decodable {
        let name: String
        let type: String
        let offset: Int
        private enum CodingKeys: String, CodingKey { case name, type, offset }
        init(from decoder: Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            name = try values.decode(String.self,forKey:.name)
            type = try values.decode(String.self,forKey:.type)
            // Reflection also includes built-in output structs, which have no byte offsets.
            offset = try values.decodeIfPresent(Int.self,forKey:.offset) ?? 0
        }
    }
    struct TypeInfo: Decodable { let members: [Member]? }
    struct UBO: Decodable { let type: String; let block_size: Int }
    struct Stage: Decodable { let inputs: [Input]?; let types: [String: TypeInfo]; let ubos: [UBO] }
    let name: String
    let textures: [Texture]
    let stages: [String: Stage]
}

/// Immutable GPU program. Reflection keeps the exact std140 offsets produced by the pinned port.
final class NativeProgram: @unchecked Sendable {
    let pipeline: MTLRenderPipelineState
    let cullMode: MTLCullMode
    let manifest: ShaderManifest
    let members: [String: ShaderManifest.Member]
    let byteCount: Int
    let fields: [(input: ShaderManifest.Input, offset: Int)]
    let stride: Int

    init(device: MTLDevice, name: String, additive: Bool = false, premultiplied: Bool = false, sampleCount: Int = 1) throws {
        // The pinned box materials use FrontSide; drawing their backs doubles faded rims.
        cullMode = ["terrain","floating","workshop","sonicParticle"].contains(name) ? .back : .none
        guard let root = Bundle.module.url(forResource: "Resources", withExtension: nil) else { throw NativeStageFailure.resource(name) }
        manifest = try JSONDecoder().decode(ShaderManifest.self, from: Data(contentsOf: root.appendingPathComponent(name + ".json")))
        guard let stage = manifest.stages["vert"], let ubo = stage.ubos.first else { throw NativeStageFailure.resource(name) }
        members = Dictionary(uniqueKeysWithValues: (stage.types[ubo.type]?.members ?? []).map { ($0.name, $0) })
        byteCount = ubo.block_size
        var layout: [(ShaderManifest.Input, Int)] = [], offset = 0
        let vertex = MTLVertexDescriptor()
        for input in (stage.inputs ?? []).sorted(by: { $0.location < $1.location }) {
            if input.type == "mat4" {
                for i in 0..<4 {
                    vertex.attributes[input.location + i].format = .float4
                    vertex.attributes[input.location + i].offset = i * 16
                    vertex.attributes[input.location + i].bufferIndex = 17
                }
                vertex.layouts[17].stride = 64
                vertex.layouts[17].stepFunction = .perInstance
                continue
            }
            let count = input.type.last.flatMap { Int(String($0)) } ?? 1
            layout.append((input, offset))
            vertex.attributes[input.location].format = count == 3 ? .float3 : count == 2 ? .float2 : count == 4 ? .float4 : .float
            vertex.attributes[input.location].offset = offset
            vertex.attributes[input.location].bufferIndex = 16
            offset += count * 4
        }
        fields = layout; stride = offset
        vertex.layouts[16].stride = offset
        func library(_ suffix: String) throws -> MTLLibrary {
            var source = try String(contentsOf: root.appendingPathComponent(name + suffix + ".metal"), encoding: .utf8)
            // OpenGL [-w,w] and Metal [0,w] use different clipping depth conventions.
            if suffix == ".vert" { source = source.replacingOccurrences(of: "return out;", with: "out.gl_Position.z = (out.gl_Position.z + out.gl_Position.w) * 0.5; return out;") }
            return try device.makeLibrary(source: source, options: nil)
        }
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.vertexFunction = try library(".vert").makeFunction(name: name + "_vert")
        descriptor.fragmentFunction = try library(".frag").makeFunction(name: name + "_frag")
        descriptor.vertexDescriptor = vertex
        descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
        descriptor.depthAttachmentPixelFormat = .depth32Float
        descriptor.rasterSampleCount = sampleCount
        let attachment = descriptor.colorAttachments[0]!
        attachment.isBlendingEnabled = true
        attachment.sourceRGBBlendFactor = premultiplied ? .one : .sourceAlpha
        attachment.destinationRGBBlendFactor = additive ? .one : .oneMinusSourceAlpha
        attachment.sourceAlphaBlendFactor = .one
        attachment.destinationAlphaBlendFactor = .oneMinusSourceAlpha
        pipeline = try device.makeRenderPipelineState(descriptor: descriptor)
    }

    func uniformData(_ values: [String: [Float]], modelView: simd_float4x4, projection: simd_float4x4,
                     model: simd_float4x4 = matrix_identity_float4x4, view: simd_float4x4 = matrix_identity_float4x4) -> Data {
        var data = Data(count: byteCount)
        data.withUnsafeMutableBytes { (raw: UnsafeMutableRawBufferPointer) in
            for (name, values) in values {
                guard let member = members[name] else { continue }
                for (i, value) in values.enumerated() where member.offset + i * 4 + 4 <= byteCount {
                    if member.type == "int" { raw.storeBytes(of: Int32(value), toByteOffset: member.offset + i * 4, as: Int32.self) }
                    else { raw.storeBytes(of: value, toByteOffset: member.offset + i * 4, as: Float.self) }
                }
            }
            for (name, var matrix) in [("modelViewMatrix", modelView), ("projectionMatrix", projection), ("modelMatrix", model), ("viewMatrix", view)] {
                guard let member = members[name] else { continue }
                withUnsafeBytes(of: &matrix) { raw.baseAddress!.advanced(by: member.offset).copyMemory(from: $0.baseAddress!, byteCount: 64) }
            }
            if let member = members["normalMatrix"] {
                let normal = simd_transpose(simd_inverse(simd_float3x3(SIMD3(modelView[0].x,modelView[0].y,modelView[0].z), SIMD3(modelView[1].x,modelView[1].y,modelView[1].z), SIMD3(modelView[2].x,modelView[2].y,modelView[2].z))))
                for column in 0..<3 { for row in 0..<3 { raw.storeBytes(of: normal[column][row], toByteOffset: member.offset + column * 16 + row * 4, as: Float.self) } }
            }
        }
        return data
    }
}
