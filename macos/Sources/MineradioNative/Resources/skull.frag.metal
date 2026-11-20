// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uTime;
    float uPixel;
    float uPointScale;
    float uBloomStrength;
    float uColorBoost;
    float uBass;
    float uMid;
    float uTreble;
    float uBeat;
    float uJawOpen;
    float uSkullFlash;
    float3 uColorA;
    float3 uColorB;
    float3 uShadow;
    packed_float3 uLight;
    float uOpacity;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
    float3x3 normalMatrix;
};

struct skull_frag_out
{
    float4 nativeColor [[color(0)]];
};

struct skull_frag_in
{
    float vKind [[user(locn0)]];
    float vLight [[user(locn1)]];
    float vRim [[user(locn2)]];
    float vAmp [[user(locn3)]];
    float vDensity [[user(locn4)]];
    float vFlash [[user(locn5)]];
};

fragment skull_frag_out skull_frag(skull_frag_in in [[stage_in]], constant StageUniforms& _37 [[buffer(0)]], texture2d<float> uMap [[texture(1)]], sampler uMapSmplr [[sampler(1)]], float2 gl_PointCoord [[point_coord]])
{
    skull_frag_out out = {};
    float4 tex = uMap.sample(uMapSmplr, gl_PointCoord);
    if (tex.w < 0.070000000298023223876953125)
    {
        discard_fragment();
    }
    float contrast = fast::clamp(_37.uColorBoost, 0.5, 2.0);
    float lit = fast::clamp(powr(in.vLight, mix(1.17999994754791259765625, 0.7400000095367431640625, (contrast - 0.5) / 1.5)), 0.0, 1.2799999713897705078125);
    float3 bone = mix(_37.uColorA, _37.uColorB, float3(fast::clamp(((in.vKind - 0.3400000035762786865234375) * 2.0) + (lit * 0.180000007152557373046875), 0.0, 1.0)));
    float3 col = mix(_37.uShadow, bone, float3(fast::clamp(lit, 0.0, 1.0)));
    col = mix(col, float3(_37.uLight), float3(fast::clamp(in.vRim * ((0.14000000059604644775390625 + (_37.uBloomStrength * 0.0350000001490116119384765625)) + (in.vFlash * 0.4000000059604644775390625)), 0.0, 0.540000021457672119140625)));
    col = mix(col, float3(_37.uLight), float3(fast::clamp((in.vAmp * (0.0900000035762786865234375 + (_37.uBloomStrength * 0.02500000037252902984619140625))) + (in.vFlash * 0.560000002384185791015625), 0.0, 0.680000007152557373046875)));
    float alpha = (tex.w * _37.uOpacity) * fast::clamp((((0.20000000298023223876953125 + (lit * 0.439999997615814208984375)) + (in.vDensity * 0.4000000059604644775390625)) + (in.vRim * 0.100000001490116119384765625)) + (in.vFlash * 0.4600000083446502685546875), 0.119999997317790985107421875, 1.559999942779541015625);
    out.nativeColor = float4(col, alpha);
    return out;
}
