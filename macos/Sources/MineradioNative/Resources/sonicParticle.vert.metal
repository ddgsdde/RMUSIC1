// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    packed_float3 uColor;
    float uOpacity;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct sonicParticle_vert_out
{
    float4 gl_Position [[position]];
};

struct sonicParticle_vert_in
{
    float4 instanceMatrix_0 [[attribute(0)]];
    float4 instanceMatrix_1 [[attribute(1)]];
    float4 instanceMatrix_2 [[attribute(2)]];
    float4 instanceMatrix_3 [[attribute(3)]];
    float3 position [[attribute(4)]];
};

vertex sonicParticle_vert_out sonicParticle_vert(sonicParticle_vert_in in [[stage_in]], constant StageUniforms& _20 [[buffer(0)]])
{
    sonicParticle_vert_out out = {};
    float4x4 instanceMatrix = {};
    instanceMatrix[0] = in.instanceMatrix_0;
    instanceMatrix[1] = in.instanceMatrix_1;
    instanceMatrix[2] = in.instanceMatrix_2;
    instanceMatrix[3] = in.instanceMatrix_3;
    out.gl_Position = ((_20.projectionMatrix * _20.modelViewMatrix) * instanceMatrix) * float4(in.position, 1.0);
    return out;
}
