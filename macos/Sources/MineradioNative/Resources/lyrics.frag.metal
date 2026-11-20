// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#pragma clang diagnostic ignored "-Wmissing-prototypes"

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

struct lyrics_frag_out
{
    float4 nativeColor [[color(0)]];
};

struct lyrics_frag_in
{
    float2 vUv [[user(locn0)]];
};

static inline __attribute__((always_inline))
float hash(thread const float& n)
{
    return fract(sin(n) * 43758.546875);
}

static inline __attribute__((always_inline))
float hash2(thread const float2& p)
{
    return fract(sin(dot(p, float2(127.09999847412109375, 311.70001220703125))) * 43758.546875);
}

fragment lyrics_frag_out lyrics_frag(lyrics_frag_in in [[stage_in]], constant StageUniforms& _67 [[buffer(0)]], texture2d<float> uMap [[texture(1)]], sampler uMapSmplr [[sampler(1)]], bool gl_FrontFacing [[front_facing]])
{
    lyrics_frag_out out = {};
    float2 _40;
    if (gl_FrontFacing)
    {
        _40 = in.vUv;
    }
    else
    {
        _40 = float2(1.0 - in.vUv.x, in.vUv.y);
    }
    float2 uv = _40;
    float sliceRows = mix(16.0, 38.0, fast::clamp(_67.uGlitchSlice / 1.39999997615814208984375, 0.0, 1.0));
    float param = _67.uGlitchSeed;
    float row = floor((uv.y + (hash(param) * 0.0350000001490116119384765625)) * sliceRows);
    float param_1 = _67.uGlitchSeed * 1.37000000476837158203125;
    float timeSlot = floor((_67.uTime * mix(7.0, 24.0, fast::clamp(_67.uGlitchRate / 2.2000000476837158203125, 0.0, 1.0))) + (hash(param_1) * 5.0));
    float2 param_2 = float2(row + _67.uGlitchSeed, timeSlot);
    float rowRnd = hash2(param_2);
    float2 param_3 = float2(timeSlot + (_67.uGlitchSeed * 0.709999978542327880859375), row * 3.1700000762939453125);
    float phaseRnd = hash2(param_3);
    float glitchGate = smoothstep(0.7400000095367431640625, 0.9900000095367431640625, rowRnd + (_67.uGlitchBurst * 0.2800000011920928955078125)) * step(0.001000000047497451305389404296875, _67.uGlitch);
    float2 param_4 = float2(row * 5.110000133514404296875, timeSlot + _67.uGlitchSeed);
    float glitchDir = (hash2(param_4) < 0.5) ? (-1.0) : 1.0;
    float2 param_5 = float2(floor(uv.x * 19.0) + row, (timeSlot * 1.309999942779541015625) + _67.uGlitchSeed);
    float micro = hash2(param_5);
    float glitchWave = ((phaseRnd * 2.0) - 1.0) * (0.550000011920928955078125 + (micro * 0.949999988079071044921875));
    float glitchWidth = (0.00200000009499490261077880859375 + ((rowRnd * rowRnd) * 0.00850000046193599700927734375)) * (0.550000011920928955078125 + (_67.uGlitchBurst * 1.85000002384185791015625));
    float2 sampleUv = uv + float2(((((glitchGate * glitchDir) * glitchWave) * _67.uGlitch) * _67.uGlitchSlice) * glitchWidth, 0.0);
    float mask = uMap.sample(uMapSmplr, sampleUv).w;
    if (mask < 0.00999999977648258209228515625)
    {
        discard_fragment();
    }
    float activeMix = fast::clamp(_67.uActiveMix, 0.0, 1.0);
    float denom = fast::max(0.001000000047497451305389404296875, _67.uTextMax - _67.uTextMin);
    float p = fast::clamp((uv.x - _67.uTextMin) / denom, 0.0, 1.0);
    float filled = (1.0 - smoothstep(_67.uProgress, _67.uProgress + _67.uFeather, p)) * activeMix;
    float edge = (1.0 - smoothstep(0.0, _67.uFeather * 2.7999999523162841796875, abs(p - _67.uProgress))) * activeMix;
    float sweepPhase = fract(_67.uTime * (0.2800000011920928955078125 + (_67.uSweep * 0.100000001490116119384765625)));
    float sweepLine = (1.0 - smoothstep(0.0, 0.07999999821186065673828125, abs((uv.x + (uv.y * 0.4199999868869781494140625)) - ((sweepPhase * 1.41999995708465576171875) - 0.180000007152557373046875)))) * activeMix;
    float fineLine = (powr(fast::max(0.0, sin(((uv.x - (uv.y * 0.180000007152557373046875)) + (_67.uTime * 0.819999992847442626953125)) * 42.0)), 24.0) * _67.uShimmer) * activeMix;
    float chromaOffset = (((0.002799999900162220001220703125 + (phaseRnd * 0.004800000227987766265869140625)) + (_67.uGlitchBurst * 0.003800000064074993133544921875)) * _67.uGlitch) * _67.uGlitchChroma;
    float chromaR = uMap.sample(uMapSmplr, (sampleUv + float2(chromaOffset * glitchDir, 0.0))).w;
    float chromaB = uMap.sample(uMapSmplr, (sampleUv - float2(chromaOffset * glitchDir, 0.0))).w;
    float3 color = mix(_67.uBaseColor, _67.uHiColor, float3(filled * 0.87999999523162841796875));
    color += (((_67.uGlowColor * edge) * 0.14000000059604644775390625) * _67.uEdgeBoost);
    color += (((_67.uSolarColor * sweepLine) * _67.uSweep) * (0.119999997317790985107421875 + (filled * 0.300000011920928955078125)));
    color += ((_67.uGlowColor * fineLine) * (0.07999999821186065673828125 + (filled * 0.180000007152557373046875)));
    color += (((((float3(chromaR, mask * 0.180000007152557373046875, chromaB) * glitchGate) * _67.uGlitch) * _67.uGlitchChroma) * activeMix) * (0.20000000298023223876953125 + (_67.uGlitchBurst * 0.2199999988079071044921875)));
    float3 solar = _67.uSolarColor;
    color = mix(color, color + (solar * 0.3400000035762786865234375), float3((_67.uSolar * activeMix) * (0.25 + (filled * 0.449999988079071044921875))));
    color += (((solar * edge) * _67.uSolar) * 0.2199999988079071044921875);
    float lum = dot(color, float3(0.2989999949932098388671875, 0.58700001239776611328125, 0.114000000059604644775390625));
    color += float3(fast::max(0.0, 0.300000011920928955078125 - lum));
    float alpha = fast::max(mask, ((fast::max(chromaR, chromaB) * glitchGate) * _67.uGlitch) * (0.300000011920928955078125 + (_67.uGlitchBurst * 0.319999992847442626953125)));
    out.nativeColor = float4(color, alpha * _67.uOpacity);
    return out;
}
