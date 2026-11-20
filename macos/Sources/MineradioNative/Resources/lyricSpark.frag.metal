// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uSize;
    float uPixel;
    char _m2_pad[8];
    packed_float3 uColor;
    float uOpacity;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct lyricSpark_frag_out
{
    float4 nativeColor [[color(0)]];
};

struct lyricSpark_frag_in
{
    float vSeed [[user(locn0)]];
};

fragment lyricSpark_frag_out lyricSpark_frag(lyricSpark_frag_in in [[stage_in]], constant StageUniforms& _41 [[buffer(0)]], texture2d<float> uMap [[texture(1)]], sampler uMapSmplr [[sampler(1)]], float2 gl_PointCoord [[point_coord]])
{
    lyricSpark_frag_out out = {};
    float4 tex = uMap.sample(uMapSmplr, gl_PointCoord);
    float twinkle = 0.7200000286102294921875 + (fract(sin(in.vSeed * 7.309999942779541015625) * 91.6999969482421875) * 0.2800000011920928955078125);
    out.nativeColor = float4(float3(_41.uColor) * twinkle, tex.w * _41.uOpacity);
    return out;
}
