// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

template <typename T, int stride>
struct spvPaddedArrayElement { T data; char padding[stride - sizeof(T)]; };

struct Ripple
{
    float2 pos;
    float time;
    float strength;
    float isActive;
    float rippleType;
};

struct StageUniforms
{
    float uTime;
    float uIdleWave;
    float uAudioIntensity;
    float uResponseRange;
    float uHalfExtent;
    float uSubBass;
    float uBass;
    float uLowMid;
    float uMid;
    float uHighMid;
    float uSmoothness;
    float uDensity;
    float uEnergy;
    char _m13_pad[8];
    spvPaddedArrayElement<Ripple, 32> uRipples[10];
    float uPresence;
    float uBrilliance;
    float uAir;
    float uWarmth;
    float uBrightness;
    float uSharpness;
    float3 uBaseColor1;
    float3 uBaseColor2;
    float3 uCoolCore;
    float3 uCoolEdge;
    float3 uWarmCore;
    float3 uWarmEdge;
    float3 uRippleColor;
    packed_float3 uPeakColor;
    float uGlowIntensity;
    float uPeakEnabled;
    float uPeakIntensity;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
    float4x4 viewMatrix;
};

struct workshop_frag_out
{
    float4 nativeColor [[color(0)]];
};

struct workshop_frag_in
{
    float2 vUv [[user(locn0)]];
    float vElevation [[user(locn1)]];
    float vDistance [[user(locn2)]];
    float2 vRippleAnim [[user(locn3)]];
    float3 vNormal [[user(locn4)]];
    float vRelativeY [[user(locn5)]];
    float2 vInstancePos [[user(locn6)]];
    float vPeakIntensity [[user(locn7)]];
};

static inline __attribute__((always_inline))
float random(thread const float2& st)
{
    return fract(sin(dot(st, float2(12.98980045318603515625, 78.233001708984375))) * 43758.546875);
}

fragment workshop_frag_out workshop_frag(workshop_frag_in in [[stage_in]], constant StageUniforms& _68 [[buffer(0)]])
{
    workshop_frag_out out = {};
    bool isTop = in.vNormal.y > 0.5;
    float distFromTop = 1.0 - in.vRelativeY;
    float2 param = in.vInstancePos;
    float rnd = random(param);
    float centerDist = length(in.vInstancePos);
    float normElevation = fast::clamp(in.vElevation / 8.0, 0.0, 1.0);
    float3 cBase1 = _68.uBaseColor1;
    float3 cBase2 = _68.uBaseColor2;
    float3 coolCore = _68.uCoolCore;
    float3 coolEdge = _68.uCoolEdge;
    float3 warmCore = _68.uWarmCore;
    float3 warmEdge = _68.uWarmEdge;
    float warmBlend = smoothstep(0.0, 1.0, (_68.uWarmth * 1.5) + (0.5 - (centerDist / 80.0)));
    float3 zoneCore = mix(coolCore, warmCore, float3(warmBlend));
    float3 zoneEdge = mix(coolEdge, warmEdge, float3(warmBlend));
    float3 targetGlow = mix(zoneCore, zoneEdge, float3(fract(rnd * 11.0)));
    float distFade = 1.0 - smoothstep(40.0, 75.0, centerDist);
    targetGlow = mix(targetGlow, float3(0.4000000059604644775390625, 0.800000011920928955078125, 1.0), float3(_68.uBrightness * 0.60000002384185791015625));
    float3 currentGlow = (mix(cBase2, targetGlow, float3(normElevation)) * _68.uGlowIntensity) * distFade;
    float normalBlend = in.vRippleAnim.x * in.vRippleAnim.x;
    float whiteBlend = in.vRippleAnim.y * in.vRippleAnim.y;
    currentGlow = mix(currentGlow, _68.uRippleColor, float3(normalBlend * 0.85000002384185791015625));
    currentGlow = mix(currentGlow, float3(1.0), float3(whiteBlend * 0.89999997615814208984375));
    float peakBlend = (powr(in.vPeakIntensity, 0.85000002384185791015625) * _68.uPeakEnabled) * _68.uPeakIntensity;
    currentGlow = mix(currentGlow, float3(_68.uPeakColor), float3(fast::clamp(peakBlend, 0.0, 1.0) * 0.699999988079071044921875));
    float3 bodyColor = mix(cBase1, cBase2, float3(in.vRelativeY * distFade));
    float3 finalColor;
    if (isTop)
    {
        float topIntensity = smoothstep(0.0, 0.4000000059604644775390625, normElevation);
        topIntensity += fast::clamp(peakBlend * 0.4000000059604644775390625, 0.0, 1.0);
        float twinkleDistFalloff = smoothstep(60.0, 30.0, centerDist);
        float twinkleMultiplier = mix(twinkleDistFalloff, 1.0, smoothstep(0.00999999977648258209228515625, 0.100000001490116119384765625, normElevation));
        bool isSparkleTarget = fract(rnd * 31.0) > 0.949999988079071044921875;
        if (isSparkleTarget && (normElevation < 0.100000001490116119384765625))
        {
            topIntensity += ((_68.uAir * 2.0) * twinkleMultiplier);
        }
        finalColor = mix(cBase2, currentGlow, float3(topIntensity));
        float edgeX = smoothstep(0.0500000007450580596923828125, 0.00999999977648258209228515625, in.vUv.x) + smoothstep(0.949999988079071044921875, 0.9900000095367431640625, in.vUv.x);
        float edgeY = smoothstep(0.0500000007450580596923828125, 0.00999999977648258209228515625, in.vUv.y) + smoothstep(0.949999988079071044921875, 0.9900000095367431640625, in.vUv.y);
        float edge = fast::min(edgeX + edgeY, 1.0);
        finalColor += (((currentGlow * edge) * 0.800000011920928955078125) * (topIntensity + 0.300000011920928955078125));
        float flashChance = smoothstep(0.5, 1.0, _68.uPresence);
        if (fract(rnd * 53.0) > (0.98500001430511474609375 - (flashChance * 0.0500000007450580596923828125)))
        {
            float flashSync = (sin((_68.uTime * 8.0) + (rnd * 50.0)) * 0.5) + 0.5;
            finalColor += ((((mix(float3(1.0), float3(0.5, 1.0, 1.0), float3(rnd)) * flashSync) * _68.uPresence) * (1.0 + (_68.uSharpness * 1.5))) * twinkleMultiplier);
        }
        float brilliancePhase = (sin((_68.uTime * 1.5) + (rnd * 30.0)) * 0.5) + 0.5;
        bool _360 = edge > 0.60000002384185791015625;
        bool _369;
        if (_360)
        {
            _369 = fract(rnd * 89.0) > 0.99199998378753662109375;
        }
        else
        {
            _369 = _360;
        }
        if (_369 && (brilliancePhase > 0.699999988079071044921875))
        {
            finalColor += ((((float3(1.0) * _68.uBrilliance) * 2.0) * twinkleMultiplier) * brilliancePhase);
        }
    }
    else
    {
        float verticalFalloff = mix(1.0, 3.0, _68.uSharpness);
        float sideGlow = smoothstep(0.5 / verticalFalloff, 0.0, distFromTop) * normElevation;
        if (normElevation < 0.0199999995529651641845703125)
        {
            sideGlow = 0.0;
        }
        float3 sideGlowColor = mix(currentGlow, float3(_68.uPeakColor), float3(fast::clamp(peakBlend * 0.4000000059604644775390625, 0.0, 1.0)));
        finalColor = mix(bodyColor, sideGlowColor, float3(sideGlow * 1.5));
        float rimGlow = smoothstep(0.02999999932944774627685546875, 0.0, distFromTop) * normElevation;
        finalColor += (mix(currentGlow, float3(_68.uPeakColor), float3(fast::clamp(peakBlend * 0.3499999940395355224609375, 0.0, 1.0))) * rimGlow);
    }
    finalColor = mix(finalColor, float3(_68.uPeakColor), float3(fast::clamp(peakBlend * 0.1500000059604644775390625, 0.0, 1.0)));
    finalColor += ((_68.uRippleColor * normalBlend) * 0.4000000059604644775390625);
    finalColor += ((float3(1.0) * whiteBlend) * 0.699999988079071044921875);
    float aerialFog = smoothstep(30.0, 65.0, in.vDistance);
    float3 atmosphericColor = mix(cBase1, cBase2, float3(0.4000000059604644775390625));
    finalColor = mix(finalColor, atmosphericColor, float3(aerialFog * 0.5));
    float alphaFade = 1.0 - smoothstep(55.0, 78.0, in.vDistance);
    out.nativeColor = float4(finalColor, alphaFade);
    return out;
}
