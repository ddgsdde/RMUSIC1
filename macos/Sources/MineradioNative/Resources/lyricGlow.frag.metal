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

struct lyricGlow_frag_out
{
    float4 nativeColor [[color(0)]];
};

struct lyricGlow_frag_in
{
    float2 vUv [[user(locn0)]];
};

fragment lyricGlow_frag_out lyricGlow_frag(lyricGlow_frag_in in [[stage_in]], constant StageUniforms& _31 [[buffer(0)]], texture2d<float> uMap [[texture(1)]], sampler uMapSmplr [[sampler(1)]])
{
    lyricGlow_frag_out out = {};
    float a = uMap.sample(uMapSmplr, in.vUv).w * 0.20000000298023223876953125;
    a += ((((uMap.sample(uMapSmplr, (in.vUv + float2(_31.uRadius.x, 0.0))).w + uMap.sample(uMapSmplr, (in.vUv - float2(_31.uRadius.x, 0.0))).w) + uMap.sample(uMapSmplr, (in.vUv + float2(0.0, _31.uRadius.y))).w) + uMap.sample(uMapSmplr, (in.vUv - float2(0.0, _31.uRadius.y))).w) * 0.119999997317790985107421875);
    a += ((((uMap.sample(uMapSmplr, (in.vUv + _31.uRadius)).w + uMap.sample(uMapSmplr, (in.vUv - _31.uRadius)).w) + uMap.sample(uMapSmplr, (in.vUv + float2(_31.uRadius.x, -_31.uRadius.y))).w) + uMap.sample(uMapSmplr, (in.vUv + float2(-_31.uRadius.x, _31.uRadius.y))).w) * 0.07999999821186065673828125);
    out.nativeColor = float4(_31.uColor[0], _31.uColor[1], _31.uColor[2], a * _31.uOpacity);
    return out;
}

