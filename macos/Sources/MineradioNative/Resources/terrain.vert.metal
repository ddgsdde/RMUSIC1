// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uTime;
    float uSubBass;
    float uBass;
    float uLowMid;
    float uMid;
    float uHighMid;
    float uSmoothness;
    float uDensity;
    float uEnergy;
    float uAmplitude;
    float4 uRipples[10];
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

struct terrain_vert_out
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

struct terrain_vert_in
{
    float3 position [[attribute(0)]];
    float2 uv [[attribute(1)]];
    float3 normal [[attribute(2)]];
    float4 instanceMatrix_0 [[attribute(3)]];
    float4 instanceMatrix_1 [[attribute(4)]];
    float4 instanceMatrix_2 [[attribute(5)]];
    float4 instanceMatrix_3 [[attribute(6)]];
};

static inline __attribute__((always_inline))
float random(thread const float2& st)
{
    return fract(sin(dot(st, float2(12.98980045318603515625, 78.233001708984375))) * 43758.546875);
}

static inline __attribute__((always_inline))
float2 mod289(thread const float2& x)
{
    return x - (floor(x * 0.00346020772121846675872802734375) * 289.0);
}

static inline __attribute__((always_inline))
float3 mod289(thread const float3& x)
{
    return x - (floor(x * 0.00346020772121846675872802734375) * 289.0);
}

static inline __attribute__((always_inline))
float3 permute(thread const float3& x)
{
    float3 param = ((x * 34.0) + float3(1.0)) * x;
    return mod289(param);
}

static inline __attribute__((always_inline))
float snoise(thread const float2& v)
{
    float2 i = floor(v + float2(dot(v, float2(0.3660254180431365966796875))));
    float2 x0 = (v - i) + float2(dot(i, float2(0.211324870586395263671875)));
    float2 i1 = select(float2(0.0, 1.0), float2(1.0, 0.0), bool2(x0.x > x0.y));
    float4 x12 = x0.xyxy + float4(0.211324870586395263671875, 0.211324870586395263671875, -0.57735025882720947265625, -0.57735025882720947265625);
    float4 _104 = x12;
    float2 _106 = _104.xy - i1;
    x12.x = _106.x;
    x12.y = _106.y;
    float2 param = i;
    i = mod289(param);
    float3 param_1 = float3(i.y) + float3(0.0, i1.y, 1.0);
    float3 param_2 = (permute(param_1) + float3(i.x)) + float3(0.0, i1.x, 1.0);
    float3 p = permute(param_2);
    float3 m = fast::max(float3(0.5) - float3(dot(x0, x0), dot(x12.xy, x12.xy), dot(x12.zw, x12.zw)), float3(0.0));
    m *= m;
    m *= m;
    float3 x = (fract(p * float3(0.024390242993831634521484375)) * 2.0) - float3(1.0);
    float3 h = abs(x) - float3(0.5);
    float3 ox = floor(x + float3(0.5));
    float3 a0 = x - ox;
    m *= (float3(1.792842864990234375) - (((a0 * a0) + (h * h)) * 0.8537347316741943359375));
    float3 g;
    g.x = (a0.x * x0.x) + (h.x * x0.y);
    float2 _221 = (a0.yz * x12.xz) + (h.yz * x12.yw);
    g.y = _221.x;
    g.z = _221.y;
    return 130.0 * dot(m, g);
}

vertex terrain_vert_out terrain_vert(terrain_vert_in in [[stage_in]], constant StageUniforms& _285 [[buffer(0)]])
{
    terrain_vert_out out = {};
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
    float centerDist = length(pos2D);
    out.vDistance = centerDist;
    float2 param = pos2D;
    float rnd = random(param);
    float2 movingPos = (pos2D * 0.0500000007450580596923828125) + float2(_285.uTime * 0.100000001490116119384765625, _285.uTime * 0.0500000007450580596923828125);
    float2 param_1 = movingPos;
    float baseNoise = (snoise(param_1) + 1.0) * 0.5;
    float wave = (sin(((pos2D.x * 0.1500000059604644775390625) + (pos2D.y * 0.100000001490116119384765625)) - (_285.uTime * 0.60000002384185791015625)) * 0.5) + 0.5;
    float globalFalloff = smoothstep(60.0, 30.0, centerDist);
    float idleElevation = (mix(baseNoise, wave, (_285.uSmoothness * 0.5) + 0.20000000298023223876953125) * 0.800000011920928955078125) * globalFalloff;
    float subRegion = smoothstep(25.0, 0.0, centerDist);
    float subLift = (_285.uSubBass * subRegion) * 5.0;
    float2 param_2 = (pos2D * 0.100000001490116119384765625) - float2(0.0, _285.uTime * 0.20000000298023223876953125);
    float bassNoise = snoise(param_2);
    float bassRegion = smoothstep(35.0, 5.0, centerDist + (bassNoise * 5.0));
    float bassLift = ((_285.uBass * bassRegion) * smoothstep(0.0, 1.0, rnd + (_285.uDensity * 0.5))) * 4.0;
    float2 param_3 = (pos2D * 0.0500000007450580596923828125) + float2(_285.uTime * 0.100000001490116119384765625, 0.0);
    float lowMidNoise = snoise(param_3);
    float lowMidLift = (_285.uLowMid * ((lowMidNoise * 0.5) + 0.5)) * 2.5;
    float2 param_4 = pos2D * 0.100000001490116119384765625;
    float riverFlow = sin((((pos2D.x * 0.20000000298023223876953125) + (pos2D.y * 0.20000000298023223876953125)) + (snoise(param_4) * 2.0)) - (_285.uTime * 2.0));
    float midLift = (_285.uMid * fast::max(0.0, riverFlow)) * 3.0;
    float highMidRegion = smoothstep(10.0, 45.0, centerDist);
    float highMidLift = 0.0;
    if (fract(rnd * 13.30000019073486328125) > 0.800000011920928955078125)
    {
        highMidLift = ((_285.uHighMid * highMidRegion) * fract(rnd * 7.69999980926513671875)) * 2.5;
    }
    float audioElevation = (((subLift + bassLift) + lowMidLift) + midLift) + highMidLift;
    if (rnd > 0.9900000095367431640625)
    {
        audioElevation += (_285.uEnergy * 5.0);
    }
    audioElevation *= globalFalloff;
    audioElevation = fast::max(0.0, audioElevation - 0.20000000298023223876953125);
    audioElevation *= _285.uAmplitude;
    float elevation = idleElevation + audioElevation;
    float rippleElevation = 0.0;
    float rippleIntensityNormal = 0.0;
    float rippleIntensityWhite = 0.0;
    for (int i = 0; i < 10; i++)
    {
        float4 rd = _285.uRipples[i];
        if (rd.w != 0.0)
        {
            float strength = abs(rd.w);
            bool whiteRipple = rd.w < 0.0;
            float dist = length(pos2D - rd.xy);
            float timeSince = _285.uTime - rd.z;
            float curSpeed = whiteRipple ? 18.0 : 13.0;
            float curWidth = whiteRipple ? 1.35000002384185791015625 : 5.5;
            float curFadeDist = whiteRipple ? 12.0 : 26.0;
            float elevationScale = whiteRipple ? 1.14999997615814208984375 : 3.349999904632568359375;
            float waveRadius = timeSince * curSpeed;
            float d = dist - waveRadius;
            float rippleWave = exp(((-d) * d) / curWidth);
            float fade = exp((-waveRadius) / curFadeDist);
            float lifeFade = 1.0 - smoothstep(2.099999904632568359375, 4.80000019073486328125, timeSince);
            float rPulse = ((rippleWave * fade) * lifeFade) * strength;
            rippleElevation += (rPulse * elevationScale);
            if (whiteRipple)
            {
                rippleIntensityWhite += rPulse;
            }
            else
            {
                rippleIntensityNormal += rPulse;
            }
        }
    }
    elevation += rippleElevation;
    out.vRippleAnim = float2(fast::clamp(rippleIntensityNormal, 0.0, 1.0), fast::clamp(rippleIntensityWhite, 0.0, 1.0));
    out.vElevation = elevation;
    float yPos = in.position.y + 0.5;
    out.vRelativeY = yPos;
    float totalHeight = 1.0 + elevation;
    float3 pos = in.position;
    pos.y = (-0.5) + (yPos * totalHeight);
    float4 worldPosition = (_285.modelMatrix * instanceMatrix) * float4(pos, 1.0);
    out.gl_Position = (_285.projectionMatrix * _285.viewMatrix) * worldPosition;
    return out;
}
