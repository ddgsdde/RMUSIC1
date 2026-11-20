// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uSize;
    float uPixel;
    char _m2_pad[8];
    packed_float3 uColor;
    float uOpacity;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct lyricSpark_vert_out
{
    float vSeed [[user(locn0)]];
    float4 gl_Position [[position]];
    float gl_PointSize [[point_size]];
};

struct lyricSpark_vert_in
{
    float seed [[attribute(0)]];
    float3 position [[attribute(1)]];
};

vertex lyricSpark_vert_out lyricSpark_vert(lyricSpark_vert_in in [[stage_in]], constant StageUniforms& _19 [[buffer(0)]])
{
    lyricSpark_vert_out out = {};
    out.vSeed = in.seed;
    float4 mv = _19.modelViewMatrix * float4(in.position, 1.0);
    float jitter = 0.579999983310699462890625 + (fract(sin(in.seed * 19.1700000762939453125) * 43758.546875) * 1.17999994754791259765625);
    float depth = fast::clamp(2.2000000476837158203125 / fast::max(0.3499999940395355224609375, -mv.z), 0.540000021457672119140625, 1.5499999523162841796875);
    out.gl_PointSize = (((_19.uSize * jitter) * depth) * _19.uPixel) * 120.0;
    out.gl_Position = _19.projectionMatrix * mv;
    return out;
}
