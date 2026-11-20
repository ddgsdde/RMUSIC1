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
    int uRippleCount;
    float2 uMouseXY;
    float2 uHandXY;
    float uHandActive;
    float uGestureGrip;
    char _m31_pad[8];
    packed_float3 uTintColor;
    float uTintStrength;
    float uAlpha;
    float uParticleDim;
    float uBackdropAdapt;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct cover_frag_out
{
    float4 nativeColor [[color(0)]];
};

struct cover_frag_in
{
    float3 vColor [[user(locn0)]];
    float vBright [[user(locn1)]];
    float vRipple [[user(locn2)]];
    float vEdgeBoost [[user(locn3)]];
    float vAlpha [[user(locn4)]];
    float vSourceLum [[user(locn5)]];
};

fragment cover_frag_out cover_frag(cover_frag_in in [[stage_in]], constant StageUniforms& _121 [[buffer(0)]], texture2d<float> uDotTex [[texture(5)]], sampler uDotTexSmplr [[sampler(5)]], float2 gl_PointCoord [[point_coord]])
{
    cover_frag_out out = {};
    float4 tex = uDotTex.sample(uDotTexSmplr, gl_PointCoord);
    if (tex.w < 0.0199999995529651641845703125)
    {
        discard_fragment();
    }
    float3 col = in.vColor * in.vBright;
    col = mix(col, (col * 1.2999999523162841796875) + float3(0.0500000007450580596923828125), float3(in.vEdgeBoost * 0.3499999940395355224609375));
    col = mix(col, col * 1.2000000476837158203125, float3(in.vRipple * 0.4000000059604644775390625));
    float keepBlack = 1.0 - smoothstep(0.02500000037252902984619140625, 0.115000002086162567138671875, in.vSourceLum);
    float nonBlack = 1.0 - keepBlack;
    float dotDist = length(gl_PointCoord - float2(0.5)) * 2.0;
    float readableRim = (smoothstep(0.439999997615814208984375, 0.939999997615814208984375, dotDist) * (1.0 - smoothstep(0.939999997615814208984375, 1.08000004291534423828125, dotDist))) * tex.w;
    float outLum = dot(col, float3(0.2989999949932098388671875, 0.58700001239776611328125, 0.114000000059604644775390625));
    float lightParticle = smoothstep(0.5, 0.819999992847442626953125, outLum) * nonBlack;
    float darkParticle = (1.0 - smoothstep(0.20000000298023223876953125, 0.5, outLum)) * nonBlack;
    float backdropAdapt = fast::clamp(_121.uBackdropAdapt, 0.0, 1.0);
    if (backdropAdapt > 0.001000000047497451305389404296875)
    {
        col = mix(col, float3(0.0), float3((readableRim * lightParticle) * (0.37999999523162841796875 + (backdropAdapt * 0.300000011920928955078125))));
        col = mix(col, float3(1.0), float3((readableRim * darkParticle) * (0.20000000298023223876953125 + (backdropAdapt * 0.119999997317790985107421875))));
    }
    col = fast::clamp(col, float3(0.0), float3(1.60000002384185791015625));
    out.nativeColor = float4(col, ((tex.w * _121.uAlpha) * _121.uParticleDim) * in.vAlpha);
    return out;
}
