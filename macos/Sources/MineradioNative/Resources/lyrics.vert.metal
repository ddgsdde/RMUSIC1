// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uTime;
    float uProgress;
    float uTextMin;
    float uTextMax;
    float uOpacity;
    float uFeather;
    float uSolar;
    float uSweep;
    float uShimmer;
    float uGlitch;
    float uGlitchSlice;
    float uGlitchChroma;
    float uGlitchRate;
    float uGlitchSeed;
    float uGlitchBurst;
    float uEdgeBoost;
    float uActiveMix;
    float3 uBaseColor;
    float3 uHiColor;
    float3 uGlowColor;
    float3 uSolarColor;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct lyrics_vert_out
{
    float2 vUv [[user(locn0)]];
    float4 gl_Position [[position]];
};

struct lyrics_vert_in
{
    float3 position [[attribute(0)]];
    float2 uv [[attribute(1)]];
};

vertex lyrics_vert_out lyrics_vert(lyrics_vert_in in [[stage_in]], constant StageUniforms& _26 [[buffer(0)]])
{
    lyrics_vert_out out = {};
    out.vUv = in.uv;
    out.gl_Position = (_26.projectionMatrix * _26.modelViewMatrix) * float4(in.position, 1.0);
    return out;
}
