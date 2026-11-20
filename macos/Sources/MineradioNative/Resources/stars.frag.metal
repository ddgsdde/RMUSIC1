// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uTime;
    float uBass;
    float uTreble;
    float uBeat;
    float uEnergy;
    float uPixel;
    float uPointScale;
    float uAlpha;
    float uParticleDim;
    float3 uTintColor;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct stars_frag_out
{
    float4 nativeColor [[color(0)]];
};

struct stars_frag_in
{
    float3 vColor [[user(locn0)]];
    float vAlpha [[user(locn1)]];
    float vTwinkle [[user(locn2)]];
};

fragment stars_frag_out stars_frag(stars_frag_in in [[stage_in]], texture2d<float> uDotTex [[texture(1)]], sampler uDotTexSmplr [[sampler(1)]], float2 gl_PointCoord [[point_coord]])
{
    stars_frag_out out = {};
    float4 tex = uDotTex.sample(uDotTexSmplr, gl_PointCoord);
    if (tex.w < 0.0199999995529651641845703125)
    {
        discard_fragment();
    }
    float3 col = fast::clamp(in.vColor * (0.660000026226043701171875 + (in.vTwinkle * 0.7200000286102294921875)), float3(0.0), float3(1.4500000476837158203125));
    out.nativeColor = float4(col, tex.w * in.vAlpha);
    return out;
}
