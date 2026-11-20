import simd

/// Fit the visible glyph bounds while preserving the authored anchor whenever it fits.
/// Centering the whole context window moves the current line at the beginning/end of a song.
func nativeLyricViewportFit(minimum: SIMD2<Float>, maximum: SIMD2<Float>) -> simd_float4x4 {
    guard minimum.x.isFinite, minimum.y.isFinite, maximum.x.isFinite, maximum.y.isFinite else { return matrix_identity_float4x4 }
    let span = maximum-minimum
    let fit = min(1,min(1.84/max(0.01,span.x),1.76/max(0.01,span.y)))
    let low = SIMD2<Float>(-0.92,-0.88)-minimum*fit
    let high = SIMD2<Float>(0.92,0.88)-maximum*fit
    var result = matrix_identity_float4x4
    result[0].x = fit; result[1].y = fit
    result[3].x = min(high.x,max(low.x,0)); result[3].y = min(high.y,max(low.y,0))
    return result
}
