// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uTime;
    float uBass;
    float uMid;
    float uTreble;
    float uBeat;
    float uEnergy;
    float uBurstAmt;
    float uPreset;
    float uIntensity;
    float uDepth;
    float uPointScale;
    float uSpeed;
    float uTwist;
    float uVinylSpin;
    float uColorBoost;
    float uScatter;
    float uCoverRes;
    float uBgFade;
    float uHasCover;
    float uHasDepth;
    float uEdgeEnabled;
    float uAiBoost;
    float uMouseActive;
    float uPixel;
    float uColorMixT;
    float uLoading;
    float uBloomSize;
    int uRippleCount;
    float2 uMouseXY;
    float2 uHandXY;
    float uHandActive;
    float uGestureGrip;
    char _m32_pad[8];
    packed_float3 uTintColor;
    float uTintStrength;
    float uAlpha;
    float uBloomStrength;
    float uParticleDim;
    float uBackdropAdapt;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct bloom_frag_out
{
    float4 nativeColor [[color(0)]];
};

struct bloom_frag_in
{
    float3 vColor [[user(locn0)]];
    float vBright [[user(locn1)]];
    float vRipple [[user(locn2)]];
    float vEdgeBoost [[user(locn3)]];
    float vAlpha [[user(locn4)]];
    float vSourceLum [[user(locn5)]];
};

fragment bloom_frag_out bloom_frag(bloom_frag_in in [[stage_in]], constant StageUniforms& _101 [[buffer(0)]], texture2d<float> uDotTex [[texture(5)]], sampler uDotTexSmplr [[sampler(5)]], float2 gl_PointCoord [[point_coord]])
{
    bloom_frag_out out = {};
    float4 tex = uDotTex.sample(uDotTexSmplr, gl_PointCoord);
    if (tex.w < 0.00999999977648258209228515625)
    {
        discard_fragment();
    }
    float soft = tex.w * tex.w;
    float3 col = in.vColor * (0.550000011920928955078125 + (in.vBright * 0.62000000476837158203125));
    col = mix(col, col + float3(0.2199999988079071044921875, 0.180000007152557373046875, 0.100000001490116119384765625), float3(in.vEdgeBoost * 0.3499999940395355224609375));
    col = fast::clamp(col, float3(0.0), float3(1.7999999523162841796875));
    float pulse = 1.0 + (in.vRipple * 0.64999997615814208984375);
    float keepBlack = 1.0 - smoothstep(0.02500000037252902984619140625, 0.115000002086162567138671875, in.vSourceLum);
    float bloomKeep = 1.0 - (keepBlack * 0.920000016689300537109375);
    float outLum = dot(col, float3(0.2989999949932098388671875, 0.58700001239776611328125, 0.114000000059604644775390625));
    float backdropAdapt = fast::clamp(_101.uBackdropAdapt, 0.0, 1.0);
    if (backdropAdapt > 0.001000000047497451305389404296875)
    {
        float brightAvoid = smoothstep(0.4799999892711639404296875, 0.839999973773956298828125, outLum) * backdropAdapt;
        bloomKeep *= (1.0 - (brightAvoid * 0.3400000035762786865234375));
    }
    out.nativeColor = float4(col, ((((((soft * _101.uAlpha) * _101.uBloomStrength) * _101.uParticleDim) * pulse) * 0.550000011920928955078125) * in.vAlpha) * bloomKeep);
    return out;
}
