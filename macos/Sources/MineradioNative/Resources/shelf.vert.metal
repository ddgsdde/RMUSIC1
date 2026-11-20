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

struct shelf_vert_out
{
    float2 vUv [[user(locn0)]];
    float4 gl_Position [[position]];
};

struct shelf_vert_in
{
    float2 uv [[attribute(0)]];
    float3 position [[attribute(1)]];
};

vertex shelf_vert_out shelf_vert(shelf_vert_in in [[stage_in]], constant StageUniforms& _25 [[buffer(0)]])
{
    shelf_vert_out out = {};
    out.vUv = in.uv;
    out.gl_Position = (_25.projectionMatrix * _25.modelViewMatrix) * float4(in.position, 1.0);
    return out;
}

