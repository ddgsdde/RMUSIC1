// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float2 uRadius;
    char _m1_pad[8];
    packed_float3 uColor;
    float uOpacity;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct lyricGlow_vert_out
{
    float2 vUv [[user(locn0)]];
    float4 gl_Position [[position]];
};

struct lyricGlow_vert_in
{
    float3 position [[attribute(0)]];
    float2 uv [[attribute(1)]];
};

vertex lyricGlow_vert_out lyricGlow_vert(lyricGlow_vert_in in [[stage_in]], constant StageUniforms& _26 [[buffer(0)]])
{
    lyricGlow_vert_out out = {};
    out.vUv = in.uv;
    out.gl_Position = (_26.projectionMatrix * _26.modelViewMatrix) * float4(in.position, 1.0);
    return out;
}

