// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
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

struct floating_vert_out
{
    float2 vUv [[user(locn0)]];
    float vElevation [[user(locn1)]];
    float vDistance [[user(locn2)]];
    float2 vRippleAnim [[user(locn3)]];
    float3 vNormal [[user(locn4)]];
    float vRelativeY [[user(locn5)]];
    float2 vInstancePos [[user(locn6)]];
    float4 gl_Position [[position]];
};

struct floating_vert_in
{
    float3 position [[attribute(0)]];
    float2 uv [[attribute(1)]];
    float3 normal [[attribute(2)]];
    float4 instanceMatrix_0 [[attribute(3)]];
    float4 instanceMatrix_1 [[attribute(4)]];
    float4 instanceMatrix_2 [[attribute(5)]];
    float4 instanceMatrix_3 [[attribute(6)]];
};

vertex floating_vert_out floating_vert(floating_vert_in in [[stage_in]], constant StageUniforms& _43 [[buffer(0)]])
{
    floating_vert_out out = {};
    float4x4 instanceMatrix = {};
    instanceMatrix[0] = in.instanceMatrix_0;
    instanceMatrix[1] = in.instanceMatrix_1;
    instanceMatrix[2] = in.instanceMatrix_2;
    instanceMatrix[3] = in.instanceMatrix_3;
    out.vUv = in.uv;
    out.vNormal = in.normal;
    float4 instancePos = instanceMatrix * float4(0.0, 0.0, 0.0, 1.0);
    float2 pos2D = instancePos.xz;
    out.vInstancePos = pos2D;
    out.vDistance = length(pos2D);
    out.vRippleAnim = float2(_43.uPulse * 0.800000011920928955078125, _43.uPulse * 0.300000011920928955078125);
    out.vElevation = _43.uPulse * 20.0;
    out.vRelativeY = in.position.y + 0.5;
    float4 worldPosition = (_43.modelMatrix * instanceMatrix) * float4(in.position, 1.0);
    out.gl_Position = (_43.projectionMatrix * _43.viewMatrix) * worldPosition;
    return out;
}
