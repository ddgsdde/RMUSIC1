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

struct workshop_vert_out
{
    float2 vUv [[user(locn0)]];
    float vElevation [[user(locn1)]];
    float vDistance [[user(locn2)]];
    float2 vRippleAnim [[user(locn3)]];
    float3 vNormal [[user(locn4)]];
    float vRelativeY [[user(locn5)]];
    float2 vInstancePos [[user(locn6)]];
    float vPeakIntensity [[user(locn7)]];
    float4 gl_Position [[position]];
};

struct workshop_vert_in
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
    float4 _113 = x12;
    float2 _115 = _113.xy - i1;
    x12.x = _115.x;
    x12.y = _115.y;
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
    float2 _230 = (a0.yz * x12.xz) + (h.yz * x12.yw);
    g.y = _230.x;
    g.z = _230.y;
    return 130.0 * dot(m, g);
}

static inline __attribute__((always_inline))
float easeLift(thread const float& raw, thread const float& maxHeight)
{
    float x = fast::clamp(raw, 0.0, 1.0);
    float eased = 1.0 - powr(1.0 - x, 2.5);
    float overshoot = (sin((x * 6.28299999237060546875) * 1.5) * exp((-x) * 4.0)) * 0.1500000059604644775390625;
    return (eased + overshoot) * maxHeight;
}

static inline __attribute__((always_inline))
float flowLift(thread const float& raw, thread const float& maxHeight)
{
    float x = fast::clamp(raw, 0.0, 1.0);
    float eased = powr(x, 0.75);
    float breathe = sin(x * 3.141590118408203125) * 0.119999997317790985107421875;
    return (eased + breathe) * maxHeight;
}

vertex workshop_vert_out workshop_vert(workshop_vert_in in [[stage_in]], constant StageUniforms& _347 [[buffer(0)]])
{
    workshop_vert_out out = {};
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
    float2 movingPos = (pos2D * 0.0500000007450580596923828125) + float2(_347.uTime * 0.100000001490116119384765625, _347.uTime * 0.0500000007450580596923828125);
    float2 param_1 = movingPos;
    float baseNoise = (snoise(param_1) + 1.0) * 0.5;
    float wave = (sin(((pos2D.x * 0.1500000059604644775390625) + (pos2D.y * 0.100000001490116119384765625)) - (_347.uTime * 0.60000002384185791015625)) * 0.5) + 0.5;
    float range = _347.uResponseRange;
    float globalFalloff = smoothstep((_347.uHalfExtent * 0.709999978542327880859375) * range, (_347.uHalfExtent * 0.36000001430511474609375) * range, centerDist);
    float idleElevation = (mix(baseNoise, wave, (_347.uSmoothness * 0.5) + 0.20000000298023223876953125) * 0.800000011920928955078125) * globalFalloff;
    float subRegion = smoothstep((_347.uHalfExtent * 0.300000011920928955078125) * range, 0.0, centerDist);
    float param_2 = _347.uSubBass;
    float param_3 = 6.0;
    float subLift = easeLift(param_2, param_3) * subRegion;
    out.vPeakIntensity = fast::clamp(subLift / 6.0, 0.0, 1.0);
    float2 param_4 = (pos2D * 0.100000001490116119384765625) - float2(0.0, _347.uTime * 0.20000000298023223876953125);
    float bassNoise = snoise(param_4);
    float bassRegion = smoothstep((_347.uHalfExtent * 0.4199999868869781494140625) * range, (_347.uHalfExtent * 0.0599999986588954925537109375) * range, centerDist + (bassNoise * 5.0));
    float bassRnd = smoothstep(0.0, 1.0, rnd + (_347.uDensity * 0.5));
    float param_5 = _347.uBass;
    float param_6 = 5.0;
    float bassLift = (easeLift(param_5, param_6) * bassRegion) * bassRnd;
    float2 param_7 = (pos2D * 0.0500000007450580596923828125) + float2(_347.uTime * 0.100000001490116119384765625, 0.0);
    float lowMidNoise = snoise(param_7);
    float param_8 = _347.uLowMid;
    float param_9 = 3.0;
    float lowMidLift = flowLift(param_8, param_9) * ((lowMidNoise * 0.5) + 0.5);
    float2 param_10 = pos2D * 0.100000001490116119384765625;
    float riverFlow = sin((((pos2D.x * 0.20000000298023223876953125) + (pos2D.y * 0.20000000298023223876953125)) + (snoise(param_10) * 2.0)) - (_347.uTime * 2.0));
    float param_11 = _347.uMid;
    float param_12 = 4.0;
    float midLift = flowLift(param_11, param_12) * fast::max(0.0, riverFlow);
    float highMidRegion = smoothstep((_347.uHalfExtent * 0.119999997317790985107421875) * range, (_347.uHalfExtent * 0.540000021457672119140625) * range, centerDist);
    float highMidLift = 0.0;
    if (fract(rnd * 13.30000019073486328125) > 0.800000011920928955078125)
    {
        float param_13 = _347.uHighMid;
        float param_14 = 3.0;
        float hmRaw = easeLift(param_13, param_14);
        highMidLift = (hmRaw * highMidRegion) * fract(rnd * 7.69999980926513671875);
    }
    float audioElevation = ((((subLift + bassLift) + lowMidLift) + midLift) + highMidLift) * _347.uAudioIntensity;
    if (rnd > 0.9900000095367431640625)
    {
        float energyRaw = fast::clamp(_347.uEnergy, 0.0, 1.0);
        float energyBounce = 1.0 - powr(1.0 - energyRaw, 1.5);
        energyBounce += ((sin((energyRaw * 6.28299999237060546875) * 2.0) * exp((-energyRaw) * 5.0)) * 0.20000000298023223876953125);
        audioElevation += ((energyBounce * 6.0) * _347.uAudioIntensity);
    }
    audioElevation *= globalFalloff;
    float2 param_15 = (pos2D * 0.07999999821186065673828125) + float2(_347.uTime * 0.119999997317790985107421875, 0.0);
    float hillNoise = snoise(param_15);
    float2 param_16 = (pos2D * 0.0599999986588954925537109375) + float2(0.0, _347.uTime * 0.07999999821186065673828125);
    float hillNoise2 = snoise(param_16);
    float2 param_17 = (pos2D * 0.1500000059604644775390625) + float2(_347.uTime * 0.20000000298023223876953125, _347.uTime * 0.1500000059604644775390625);
    float rippleNoise = snoise(param_17);
    float2 param_18 = (pos2D * 0.4000000059604644775390625) + float2(_347.uTime * 0.300000011920928955078125);
    float textureNoise = snoise(param_18) * 0.300000011920928955078125;
    float baseUndulation = (hillNoise * 0.60000002384185791015625) + (hillNoise2 * 0.4000000059604644775390625);
    baseUndulation = (baseUndulation * 0.5) + 0.5;
    float rippleUndulation = (rippleNoise * 0.300000011920928955078125) + 0.5;
    float blockVariation = (rnd - 0.5) * 0.1500000059604644775390625;
    float combinedWave = (((baseUndulation * 0.5) + (rippleUndulation * 0.3499999940395355224609375)) + textureNoise) + blockVariation;
    combinedWave = smoothstep(0.100000001490116119384765625, 0.89999997615814208984375, combinedWave);
    float idleBlockWave = ((combinedWave * _347.uIdleWave) * 2.5) * globalFalloff;
    float elevation = (idleElevation + audioElevation) + idleBlockWave;
    float rippleElevation = 0.0;
    float rippleIntensityNormal = 0.0;
    float rippleIntensityWhite = 0.0;
    float speed = 14.0;
    float width = 5.0;
    for (int i = 0; i < 10; i++)
    {
        if (_347.uRipples[i].data.isActive > 0.0)
        {
            float dist = length(pos2D - _347.uRipples[i].data.pos);
            float timeSince = _347.uTime - _347.uRipples[i].data.time;
            float curSpeed = speed;
            float curWidth = width;
            float curFadeDist = 22.0;
            float elevationScale = 3.0;
            if (_347.uRipples[i].data.rippleType > 0.5)
            {
                curSpeed = 18.0;
                curWidth = 2.5;
                curFadeDist = 18.0;
                elevationScale = 1.7999999523162841796875;
            }
            float waveRadius = timeSince * curSpeed;
            float d = dist - waveRadius;
            float rippleWave = exp(((-d) * d) / curWidth);
            float fade = exp((-waveRadius) / curFadeDist);
            float strengthCurve = fast::clamp(_347.uRipples[i].data.strength * 0.4000000059604644775390625, 0.0, 1.0);
            float rPulse = (rippleWave * fade) * strengthCurve;
            rippleElevation += (rPulse * elevationScale);
            if (_347.uRipples[i].data.rippleType > 0.5)
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
    out.vRippleAnim = float2(fast::clamp(sqrt(rippleIntensityNormal), 0.0, 1.0), fast::clamp(sqrt(rippleIntensityWhite), 0.0, 1.0));
    out.vElevation = elevation;
    float yPos = in.position.y + 0.5;
    out.vRelativeY = yPos;
    float totalHeight = 1.0 + elevation;
    float3 pos = in.position;
    pos.y = (-0.5) + (yPos * totalHeight);
    float4 worldPosition = instanceMatrix * float4(pos, 1.0);
    out.gl_Position = (_347.projectionMatrix * _347.viewMatrix) * worldPosition;
    return out;
}
