// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    packed_float3 uColor;
    float uOpacity;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct sonicParticle_frag_out
{
    float4 nativeColor [[color(0)]];
};

fragment sonicParticle_frag_out sonicParticle_frag(constant StageUniforms& _14 [[buffer(0)]])
{
    sonicParticle_frag_out out = {};
    out.nativeColor = float4(_14.uColor[0], _14.uColor[1], _14.uColor[2], _14.uOpacity);
    return out;
}
