// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uTime;
    float uBass;
    float uTreble;
    float uBeat;
    float uEnergy;
    float uPixel;
    float uPointScale;
    float uAlpha;
    float uParticleDim;
    float3 uTintColor;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct stars_vert_out
{
    float3 vColor [[user(locn0)]];
    float vAlpha [[user(locn1)]];
    float vTwinkle [[user(locn2)]];
    float4 gl_Position [[position]];
    float gl_PointSize [[point_size]];
};

struct stars_vert_in
{
    float aSeed [[attribute(0)]];
    float aLane [[attribute(1)]];
    float aDepthSeed [[attribute(2)]];
};

static inline __attribute__((always_inline))
float hash11(thread const float& p)
{
    return fract(sin(p * 127.09999847412109375) * 43758.546875);
}

vertex stars_vert_out stars_vert(stars_vert_in in [[stage_in]], constant StageUniforms& _55 [[buffer(0)]])
{
    stars_vert_out out = {};
    float band = floor(in.aLane * 6.0);
    float local = fract(in.aLane * 6.0);
    float bandN = (band + 0.5) / 6.0;
    float seed = in.aSeed + (band * 19.1700000762939453125);
    float param = seed * 2.13000011444091796875;
    float param_1 = seed * 5.099999904632568359375;
    float flow = fract(hash11(param) + (_55.uTime * ((0.002199999988079071044921875 + (bandN * 0.002799999900162220001220703125)) + (hash11(param_1) * 0.00340000004507601261138916015625))));
    float param_2 = seed;
    float arc = ((((flow - 0.5) * 6.283185482025146484375) * (0.680000007152557373046875 + (bandN * 0.4600000083446502685546875))) + (bandN * 2.400000095367431640625)) + (hash11(param_2) * 6.283185482025146484375);
    float wave = sin(((arc * (1.17999994754791259765625 + (bandN * 0.2800000011920928955078125))) + (_55.uTime * (0.01400000043213367462158203125 + (bandN * 0.01200000010430812835693359375)))) + (seed * 0.070000000298023223876953125));
    float param_3 = seed * 3.7000000476837158203125;
    float radius = ((7.19999980926513671875 + (bandN * 15.80000019073486328125)) + (hash11(param_3) * 6.19999980926513671875)) + (local * 1.7999999523162841796875);
    float3 pos;
    pos.x = (cos((arc * 0.7599999904632568359375) + (bandN * 0.839999973773956298828125)) * radius) + ((flow - 0.5) * (18.0 + (bandN * 14.0)));
    pos.y = (((bandN - 0.5) * 13.19999980926513671875) + (wave * (1.5 + (bandN * 1.39999997615814208984375)))) + ((local - 0.5) * 1.2000000476837158203125);
    float param_4 = seed;
    pos.z = (mix(-31.0, -4.80000019073486328125, in.aDepthSeed) + (wave * 1.2000000476837158203125)) + (sin((_55.uTime * (0.017999999225139617919921875 + (hash11(param_4) * 0.0320000015199184417724609375))) + seed) * 1.0);
    float param_5 = seed * 4.0;
    float twinkle = powr(0.5 + (0.5 * sin((_55.uTime * (0.2199999988079071044921875 + (hash11(param_5) * 0.439999997615814208984375))) + (seed * 9.0))), 5.0);
    float param_6 = seed * 6.0;
    float param_7 = seed * 7.0;
    float ridge = exp(-powr((local - (0.4199999868869781494140625 + (hash11(param_6) * 0.1599999964237213134765625))) / (0.2199999988079071044921875 + (hash11(param_7) * 0.100000001490116119384765625)), 2.0));
    float param_8 = (seed * 8.0) + band;
    float dust = smoothstep(0.20000000298023223876953125, 0.980000019073486328125, hash11(param_8));
    float3 cool = mix(float3(0.3400000035762786865234375, 0.7599999904632568359375, 1.0), float3(0.60000002384185791015625, 0.439999997615814208984375, 1.0), float3(bandN));
    float3 warm = float3(1.0, 0.7799999713897705078125, 0.579999983310699462890625);
    float3 tint = fast::max(_55.uTintColor, float3(0.07999999821186065673828125));
    out.vColor = mix(cool, warm, float3((ridge * 0.3499999940395355224609375) + (_55.uBass * 0.0599999986588954925537109375)));
    out.vColor = mix(out.vColor, tint, float3(0.2199999988079071044921875));
    out.vTwinkle = twinkle;
    out.vAlpha = (((_55.uAlpha * _55.uParticleDim) * dust) * (((0.100000001490116119384765625 + (ridge * 0.519999980926513671875)) + (twinkle * 0.319999992847442626953125)) + (_55.uBeat * 0.0500000007450580596923828125))) * (0.87999999523162841796875 + (_55.uEnergy * 0.180000007152557373046875));
    float4 mv = _55.modelViewMatrix * float4(pos, 1.0);
    float depthSize = 30.0 / fast::max(0.64999997615814208984375, -mv.z);
    float size = (((1.10000002384185791015625 + (ridge * 2.400000095367431640625)) + (twinkle * 2.7999999523162841796875)) + (_55.uTreble * 0.800000011920928955078125)) + (_55.uBeat * 0.5);
    out.gl_PointSize = fast::clamp(((size * depthSize) * _55.uPixel) * _55.uPointScale, 0.75, 5.599999904632568359375);
    out.gl_Position = _55.projectionMatrix * mv;
    return out;
}
