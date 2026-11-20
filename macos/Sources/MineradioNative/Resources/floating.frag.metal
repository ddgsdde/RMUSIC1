// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uPulse;
    float uTime;
    float uPresence;
    float uBrilliance;
    float uAir;
    float uWarmth;
    float uBrightness;
    float uSharpness;
    float3 uBaseColor1;
    float3 uBaseColor2;
    float3 uFogColor;
    float3 uCoolCore;
    float3 uCoolEdge;
    float3 uWarmCore;
    float3 uWarmEdge;
    packed_float3 uRippleColor;
    float uGlowIntensity;
    float uGlintStrength;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
    float4x4 modelMatrix;
    float4x4 viewMatrix;
};

struct floating_frag_out
{
    float4 nativeColor [[color(0)]];
};

struct floating_frag_in
{
    float2 vUv [[user(locn0)]];
    float vElevation [[user(locn1)]];
    float vDistance [[user(locn2)]];
    float2 vRippleAnim [[user(locn3)]];
    float3 vNormal [[user(locn4)]];
    float vRelativeY [[user(locn5)]];
    float2 vInstancePos [[user(locn6)]];
};

static inline __attribute__((always_inline))
float random(thread const float2& st)
{
    return fract(sin(dot(st, float2(12.98980045318603515625, 78.233001708984375))) * 43758.546875);
}

fragment floating_frag_out floating_frag(floating_frag_in in [[stage_in]], constant StageUniforms& _65 [[buffer(0)]])
{
    floating_frag_out out = {};
    bool isTop = in.vNormal.y > 0.5;
    float distFromTop = 1.0 - in.vRelativeY;
    float2 param = in.vInstancePos;
    float rnd = random(param);
    float centerDist = length(in.vInstancePos);
    float normElevation = fast::clamp(in.vElevation / 8.0, 0.0, 1.0);
    float3 cBase1 = _65.uBaseColor1;
    float3 cBase2 = _65.uBaseColor2;
    float warmBlend = smoothstep(0.0, 1.0, (_65.uWarmth * 1.5) + (0.5 - (centerDist / 80.0)));
    float3 zoneCore = mix(_65.uCoolCore, _65.uWarmCore, float3(warmBlend));
    float3 zoneEdge = mix(_65.uCoolEdge, _65.uWarmEdge, float3(warmBlend));
    float3 targetGlow = mix(zoneCore, zoneEdge, float3(fract(rnd * 11.0)));
    float distFade = 1.0 - smoothstep(40.0, 75.0, centerDist);
    float3 brightCool = mix(_65.uCoolCore, float3(1.0), float3(0.23999999463558197021484375));
    targetGlow = mix(targetGlow, brightCool, float3(_65.uBrightness * 0.60000002384185791015625));
    float3 currentGlow = (mix(cBase2, targetGlow, float3(normElevation)) * _65.uGlowIntensity) * distFade;
    currentGlow = mix(currentGlow, float3(_65.uRippleColor), float3(fast::clamp(in.vRippleAnim.x * 0.819999992847442626953125, 0.0, 0.7200000286102294921875)));
    currentGlow = mix(currentGlow, float3(1.0), float3(in.vRippleAnim.y));
    float3 bodyColor = mix(cBase1, cBase2, float3(in.vRelativeY * distFade));
    float3 finalColor;
    if (isTop)
    {
        float topIntensity = smoothstep(0.0, 0.4000000059604644775390625, normElevation);
        float twinkleDistFalloff = smoothstep(60.0, 30.0, centerDist);
        float twinkleMultiplier = mix(twinkleDistFalloff, 1.0, smoothstep(0.00999999977648258209228515625, 0.100000001490116119384765625, normElevation));
        if ((fract(rnd * 31.0) > 0.949999988079071044921875) && (normElevation < 0.100000001490116119384765625))
        {
            topIntensity += ((_65.uAir * 2.0) * twinkleMultiplier);
        }
        finalColor = mix(cBase2, currentGlow, float3(topIntensity));
        float aaX = fast::max(0.0199999995529651641845703125, fwidth(in.vUv.x) * 0.5);
        float edgeX = 1.0 - smoothstep(0.02999999932944774627685546875 - aaX, 0.02999999932944774627685546875 + aaX, fast::min(in.vUv.x, 1.0 - in.vUv.x));
        float aaY = fast::max(0.0199999995529651641845703125, fwidth(in.vUv.y) * 0.5);
        float edgeY = 1.0 - smoothstep(0.02999999932944774627685546875 - aaY, 0.02999999932944774627685546875 + aaY, fast::min(in.vUv.y, 1.0 - in.vUv.y));
        float edge = fast::min(edgeX + edgeY, 1.0);
        finalColor += (((currentGlow * edge) * 0.800000011920928955078125) * (topIntensity + 0.300000011920928955078125));
        float flashChance = smoothstep(0.300000011920928955078125, 1.0, _65.uPresence);
        if (fract(rnd * 53.0) > (0.980000019073486328125 - (flashChance * 0.100000001490116119384765625)))
        {
            float flashSync = (sin((_65.uTime * 40.0) + (rnd * 100.0)) * 0.5) + 0.5;
            finalColor += (((((mix(float3(1.0), float3(0.5, 1.0, 1.0), float3(rnd)) * flashSync) * _65.uPresence) * (1.0 + (_65.uSharpness * 2.0))) * twinkleMultiplier) * _65.uGlintStrength);
        }
        bool _332 = edge > 0.5;
        bool _344;
        if (_332)
        {
            _344 = fract((rnd * 89.0) + (_65.uTime * 2.0)) > 0.980000019073486328125;
        }
        else
        {
            _344 = _332;
        }
        if (_344)
        {
            finalColor += ((((float3(1.0) * _65.uBrilliance) * 3.0) * twinkleMultiplier) * _65.uGlintStrength);
        }
    }
    else
    {
        float verticalFalloff = mix(1.0, 3.0, _65.uSharpness);
        float sideGlow = smoothstep(0.5 / verticalFalloff, 0.0, distFromTop) * normElevation;
        if (normElevation < 0.0199999995529651641845703125)
        {
            sideGlow = 0.0;
        }
        finalColor = mix(bodyColor, currentGlow, float3(sideGlow * 1.5));
        float aaRim = fast::max(0.014999999664723873138427734375, fwidth(distFromTop) * 0.5);
        float rimGlow = (1.0 - smoothstep(0.014999999664723873138427734375 - aaRim, 0.014999999664723873138427734375 + aaRim, distFromTop)) * normElevation;
        finalColor += (currentGlow * rimGlow);
    }
    finalColor += ((float3(_65.uRippleColor) * in.vRippleAnim.x) * 0.86000001430511474609375);
    finalColor += ((float3(1.0) * in.vRippleAnim.y) * 1.2000000476837158203125);
    float aerialFog = smoothstep(30.0, 65.0, in.vDistance);
    float3 atmosphericColor = mix(cBase1, cBase2, float3(0.4000000059604644775390625));
    finalColor = mix(finalColor, atmosphericColor, float3(aerialFog * 0.3499999940395355224609375));
    float alphaFade = 1.0 - smoothstep(55.0, 78.0, in.vDistance);
    float alphaBlend = 1.0 - alphaFade;
    finalColor = mix(finalColor, _65.uFogColor, float3(alphaBlend * 0.449999988079071044921875));
    out.nativeColor = float4(finalColor, alphaFade);
    return out;
}
