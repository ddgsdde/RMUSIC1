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

struct backCover_vert_out
{
    float3 vC [[user(locn0)]];
    float vA [[user(locn1)]];
    float4 gl_Position [[position]];
    float gl_PointSize [[point_size]];
};

struct backCover_vert_in
{
    float2 aUv [[attribute(0)]];
    float aRand [[attribute(1)]];
    float3 position [[attribute(2)]];
};

vertex backCover_vert_out backCover_vert(backCover_vert_in in [[stage_in]], constant StageUniforms& _17 [[buffer(0)]], texture2d<float> uCoverTex [[texture(1)]], sampler uCoverTexSmplr [[sampler(1)]])
{
    backCover_vert_out out = {};
    float3 pos = in.position;
    pos.x += (sin((_17.uTime * 0.20000000298023223876953125) + (in.aRand * 8.0)) * 0.20000000298023223876953125);
    pos.y += (cos((_17.uTime * 0.180000007152557373046875) + (in.aRand * 6.0)) * 0.2199999988079071044921875);
    pos.z += ((sin((_17.uTime * 0.119999997317790985107421875) + (in.aRand * 5.0)) * 0.180000007152557373046875) + ((_17.uBass * 0.119999997317790985107421875) * sin(in.aRand * 11.0)));
    out.vC = uCoverTex.sample(uCoverTexSmplr, in.aUv, level(0.0)).xyz * 0.85000002384185791015625;
    float4 mvPos = _17.modelViewMatrix * float4(pos, 1.0);
    float dist = -mvPos.z;
    out.vA = fast::clamp(0.300000011920928955078125 + (0.4000000059604644775390625 * sin((_17.uTime * 0.60000002384185791015625) + (in.aRand * 5.0))), 0.100000001490116119384765625, 0.64999997615814208984375);
    float sz = fast::clamp(46.0 / fast::max(0.5, dist), 1.39999997615814208984375, 4.5);
    out.gl_PointSize = sz * _17.uPixel;
    out.gl_Position = _17.projectionMatrix * mvPos;
    return out;
}
