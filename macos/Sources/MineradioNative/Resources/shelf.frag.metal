// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uOpacity;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct shelf_frag_out
{
    float4 nativeColor [[color(0)]];
};

struct shelf_frag_in
{
    float2 vUv [[user(locn0)]];
};

fragment shelf_frag_out shelf_frag(shelf_frag_in in [[stage_in]], constant StageUniforms& _23 [[buffer(0)]], texture2d<float> uMap [[texture(1)]], sampler uMapSmplr [[sampler(1)]])
{
    shelf_frag_out out = {};
    out.nativeColor = uMap.sample(uMapSmplr, in.vUv) * _23.uOpacity;
    return out;
}

