import simd

func nativeTransform(position: SIMD3<Float> = .zero, scale: SIMD3<Float> = SIMD3(repeating: 1), rotation: SIMD3<Float> = .zero) -> simd_float4x4 {
    // THREE.Euler's default XYZ order: compose X * Y * Z, not Z * Y * X.
    let quaternion = simd_quatf(angle: rotation.x, axis: SIMD3(1,0,0)) * simd_quatf(angle: rotation.y, axis: SIMD3(0,1,0)) * simd_quatf(angle: rotation.z, axis: SIMD3(0,0,1))
    var matrix = simd_float4x4(quaternion)
    for i in 0..<3 { matrix[i] *= scale[i] }
    matrix[3] = SIMD4(position, 1)
    return matrix
}
