// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uTime;
    float uBass;
    float uPixel;
    float uAlpha;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct backCover_frag_out
{
    float4 nativeColor [[color(0)]];
};

struct backCover_frag_in
{
    float3 vC [[user(locn0)]];
    float vA [[user(locn1)]];
};

fragment backCover_frag_out backCover_frag(backCover_frag_in in [[stage_in]], constant StageUniforms& _46 [[buffer(0)]], texture2d<float> uDotTex [[texture(2)]], sampler uDotTexSmplr [[sampler(2)]], float2 gl_PointCoord [[point_coord]])
{
    backCover_frag_out out = {};
    float4 tex = uDotTex.sample(uDotTexSmplr, gl_PointCoord);
    if (tex.w < 0.0199999995529651641845703125)
    {
        discard_fragment();
    }
    out.nativeColor = float4(in.vC, (tex.w * in.vA) * _46.uAlpha);
    return out;
}
