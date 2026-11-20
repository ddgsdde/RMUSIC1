// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#pragma clang diagnostic ignored "-Wmissing-prototypes"

#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

// Implementation of the GLSL mod() function, which is slightly different than Metal fmod()
template<typename Tx, typename Ty>
inline Tx mod(Tx x, Ty y)
{
    return x - y * floor(x / y);
}

struct StageUniforms
{
    float uTime;
    float uBass;
    float uMid;
    float uTreble;
    float uBeat;
    float uEnergy;
    float uBurstAmt;
    float uPreset;
    float uIntensity;
    float uDepth;
    float uPointScale;
    float uSpeed;
    float uTwist;
    float uVinylSpin;
    float uColorBoost;
    float uScatter;
    float uCoverRes;
    float uBgFade;
    float uHasCover;
    float uHasDepth;
    float uEdgeEnabled;
    float uAiBoost;
    float uMouseActive;
    float uPixel;
    float uColorMixT;
    float uLoading;
    float uBloomSize;
    int uRippleCount;
    float2 uMouseXY;
    float2 uHandXY;
    float uHandActive;
    float uGestureGrip;
    char _m32_pad[8];
    packed_float3 uTintColor;
    float uTintStrength;
    float uAlpha;
    float uBloomStrength;
    float uParticleDim;
    float uBackdropAdapt;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
};

struct bloom_vert_out
{
    float3 vColor [[user(locn0)]];
    float vBright [[user(locn1)]];
    float vRipple [[user(locn2)]];
    float vEdgeBoost [[user(locn3)]];
    float vAlpha [[user(locn4)]];
    float vSourceLum [[user(locn5)]];
    float4 gl_Position [[position]];
    float gl_PointSize [[point_size]];
};

struct bloom_vert_in
{
    float2 aUv [[attribute(0)]];
    float aRand [[attribute(1)]];
    float3 position [[attribute(2)]];
};

static inline __attribute__((always_inline))
float2 safeCoverUv(thread const float2& uv)
{
    return fast::clamp(uv, float2(0.00120000005699694156646728515625), float2(0.998799979686737060546875));
}

static inline __attribute__((always_inline))
float3 sampleNewCoverColor(thread const float2& uv, texture2d<float> uCoverTex, sampler uCoverTexSmplr)
{
    float2 param = uv;
    return uCoverTex.sample(uCoverTexSmplr, safeCoverUv(param), level(0.0)).xyz;
}

static inline __attribute__((always_inline))
float3 samplePrevCoverColor(thread const float2& uv, texture2d<float> uPrevCoverTex, sampler uPrevCoverTexSmplr)
{
    float2 param = uv;
    return uPrevCoverTex.sample(uPrevCoverTexSmplr, safeCoverUv(param), level(0.0)).xyz;
}

static inline __attribute__((always_inline))
float4 sampleEdgeColor(thread const float2& uv, texture2d<float> uEdgeTex, sampler uEdgeTexSmplr)
{
    float2 param = uv;
    return uEdgeTex.sample(uEdgeTexSmplr, safeCoverUv(param), level(0.0));
}

static inline __attribute__((always_inline))
float rippleSumAt(thread const float2& p, thread float& maxAmp, constant StageUniforms& _465, texture2d<float> uRippleTex, sampler uRippleTexSmplr)
{
    float sum = 0.0;
    maxAmp = 0.0;
    for (int ri = 0; ri < 12; ri++)
    {
        if (ri >= _465.uRippleCount)
        {
            break;
        }
        float vCoord = (float(ri) + 0.5) / 12.0;
        float4 rd = uRippleTex.sample(uRippleTexSmplr, float2(0.5, vCoord), level(0.0));
        float age = rd.z;
        float str = rd.w;
        if (((str < 0.004999999888241291046142578125) || (age < 0.0)) || (age > 2.0))
        {
            continue;
        }
        float dx = p.x - rd.x;
        float dy = p.y - rd.y;
        float dist = sqrt((dx * dx) + (dy * dy));
        float lifeN = age / 2.0;
        float fadeIn = smoothstep(0.0, 0.0599999986588954925537109375, age);
        float fadeOut = 1.0 - smoothstep(0.699999988079071044921875, 1.0, lifeN);
        float env = fadeIn * fadeOut;
        float bulgeW = 0.550000011920928955078125 + (age * 0.800000011920928955078125);
        float bulge = exp(((-dist) * dist) / ((2.0 * bulgeW) * bulgeW)) * (1.0 - smoothstep(0.0, 0.550000011920928955078125, lifeN));
        float waveR = age * 2.099999904632568359375;
        float ringW = 0.4000000059604644775390625 + (age * 0.2199999988079071044921875);
        float ring = exp(-powr((dist - waveR) / ringW, 2.0));
        float local = (((bulge * 2.400000095367431640625) + (ring * 1.2999999523162841796875)) * env) * str;
        sum += local;
        maxAmp = fast::max(maxAmp, abs(local));
    }
    return sum;
}

static inline __attribute__((always_inline))
float3 mod289(thread const float3& x)
{
    return x - (floor(x * 0.00346020772121846675872802734375) * 289.0);
}

static inline __attribute__((always_inline))
float4 mod289v(thread const float4& x)
{
    return x - (floor(x * 0.00346020772121846675872802734375) * 289.0);
}

static inline __attribute__((always_inline))
float4 perm(thread const float4& x)
{
    float4 param = ((x * 34.0) + float4(1.0)) * x;
    return mod289v(param);
}

static inline __attribute__((always_inline))
float snoise(thread const float3& v)
{
    float3 i = floor(v + float3(dot(v, float3(0.3333333432674407958984375))));
    float3 x0 = (v - i) + float3(dot(i, float3(0.16666667163372039794921875)));
    float3 g = step(x0.yzx, x0);
    float3 l = float3(1.0) - g;
    float3 i1 = fast::min(g, l.zxy);
    float3 i2 = fast::max(g, l.zxy);
    float3 x1 = (x0 - i1) + float3(0.16666667163372039794921875);
    float3 x2 = (x0 - i2) + float3(0.3333333432674407958984375);
    float3 x3 = x0 - float3(0.5);
    float3 param = i;
    i = mod289(param);
    float4 param_1 = float4(i.z) + float4(0.0, i1.z, i2.z, 1.0);
    float4 param_2 = (perm(param_1) + float4(i.y)) + float4(0.0, i1.y, i2.y, 1.0);
    float4 param_3 = (perm(param_2) + float4(i.x)) + float4(0.0, i1.x, i2.x, 1.0);
    float4 p = perm(param_3);
    float n_ = 0.14285714924335479736328125;
    float3 ns = (float3(2.0, 0.5, 1.0) * n_) - float3(0.0, 1.0, 0.0);
    float4 j = p - (floor((p * ns.z) * ns.z) * 49.0);
    float4 x_ = floor(j * ns.z);
    float4 y_ = floor(j - (x_ * 7.0));
    float4 x = (x_ * ns.x) + ns.yyyy;
    float4 y = (y_ * ns.x) + ns.yyyy;
    float4 h = (float4(1.0) - abs(x)) - abs(y);
    float4 b0 = float4(x.xy, y.xy);
    float4 b1 = float4(x.zw, y.zw);
    float4 s0 = (floor(b0) * 2.0) + float4(1.0);
    float4 s1 = (floor(b1) * 2.0) + float4(1.0);
    float4 sh = -step(h, float4(0.0));
    float4 a0 = b0.xzyw + (s0.xzyw * sh.xxyy);
    float4 a1 = b1.xzyw + (s1.xzyw * sh.zzww);
    float3 p0 = float3(a0.xy, h.x);
    float3 p1 = float3(a0.zw, h.y);
    float3 p2 = float3(a1.xy, h.z);
    float3 p3 = float3(a1.zw, h.w);
    float4 norm = rsqrt(float4(dot(p0, p0), dot(p1, p1), dot(p2, p2), dot(p3, p3)));
    p0 *= norm.x;
    p1 *= norm.y;
    p2 *= norm.z;
    p3 *= norm.w;
    float4 m = fast::max(float4(0.60000002384185791015625) - float4(dot(x0, x0), dot(x1, x1), dot(x2, x2), dot(x3, x3)), float4(0.0));
    m *= m;
    return 42.0 * dot(m * m, float4(dot(p0, x0), dot(p1, x1), dot(p2, x2), dot(p3, x3)));
}

static inline __attribute__((always_inline))
float hash11(thread const float& p)
{
    return fract(sin(p * 127.09999847412109375) * 43758.546875);
}

vertex bloom_vert_out bloom_vert(bloom_vert_in in [[stage_in]], constant StageUniforms& _465 [[buffer(0)]], texture2d<float> uCoverTex [[texture(1)]], texture2d<float> uPrevCoverTex [[texture(2)]], texture2d<float> uEdgeTex [[texture(3)]], texture2d<float> uRippleTex [[texture(4)]], sampler uCoverTexSmplr [[sampler(1)]], sampler uPrevCoverTexSmplr [[sampler(2)]], sampler uEdgeTexSmplr [[sampler(3)]], sampler uRippleTexSmplr [[sampler(4)]])
{
    bloom_vert_out out = {};
    float t = _465.uTime * _465.uSpeed;
    float2 param = in.aUv;
    float2 sampleUv = safeCoverUv(param);
    float2 param_1 = sampleUv;
    float3 newCol = sampleNewCoverColor(param_1, uCoverTex, uCoverTexSmplr);
    float2 param_2 = sampleUv;
    float3 prevCol = samplePrevCoverColor(param_2, uPrevCoverTex, uPrevCoverTexSmplr);
    float3 coverColor = mix(prevCol, newCol, float3(fast::clamp(_465.uColorMixT, 0.0, 1.0)));
    float2 param_3 = sampleUv;
    float4 edge = sampleEdgeColor(param_3, uEdgeTex, uEdgeTexSmplr);
    float depthVal = edge.x;
    float edgeVal = edge.y;
    float fgMask = edge.z;
    float lumVal = edge.w;
    float maxRippleAmp = 0.0;
    float rippleZ = 0.0;
    float3 defaultColor = mix(float3(0.36000001430511474609375, 0.2800000011920928955078125, 0.7200000286102294921875), mix(float3(0.85000002384185791015625, 0.550000011920928955078125, 0.949999988079071044921875), float3(0.449999988079071044921875, 0.7799999713897705078125, 0.949999988079071044921875), float3(in.aUv.x)), float3(in.aUv.y));
    out.vColor = mix(defaultColor, coverColor, float3(_465.uHasCover));
    out.vAlpha = 1.0;
    float K = _465.uIntensity * 1.60000002384185791015625;
    float3 pos;
    if (_465.uPreset < 0.5)
    {
        pos = in.position;
        float2 param_4 = pos.xy;
        float param_5;
        float _706 = rippleSumAt(param_4, param_5, _465, uRippleTex, uRippleTexSmplr);
        maxRippleAmp = param_5;
        rippleZ = _706;
        float3 param_6 = float3(pos.x * 1.39999997615814208984375, pos.y * 1.39999997615814208984375, t * 0.550000011920928955078125);
        float3 param_7 = float3((pos.x * 2.7999999523162841796875) + 5.0, (pos.y * 2.7999999523162841796875) - 3.0, t * 0.85000002384185791015625);
        float midN = (snoise(param_6) * 0.60000002384185791015625) + (snoise(param_7) * 0.4000000059604644775390625);
        float3 param_8 = float3(pos.x * 0.4000000059604644775390625, pos.y * 0.4000000059604644775390625, t * 0.180000007152557373046875);
        float midMask = 0.550000011920928955078125 + (0.449999988079071044921875 * snoise(param_8));
        float midDisp = (((midN * _465.uMid) * 0.550000011920928955078125) * midMask) * K;
        float3 param_9 = float3(pos.x * 6.5, pos.y * 6.5, (t * 3.5) + (in.aRand * 4.0));
        float trebleJ = ((snoise(param_9) * _465.uTreble) * 0.180000007152557373046875) * K;
        float3 param_10 = float3(pos.x * 0.3499999940395355224609375, pos.y * 0.3499999940395355224609375, t * 0.4000000059604644775390625);
        float bassBreath = ((snoise(param_10) * _465.uBass) * 0.4199999868869781494140625) * K;
        float depthZ = ((((depthVal - 0.5) * _465.uAiBoost) * _465.uDepth) * 1.39999997615814208984375) * _465.uHasDepth;
        pos.z = ((((rippleZ * 1.2999999523162841796875) + midDisp) + trebleJ) + bassBreath) + depthZ;
    }
    else
    {
        if (_465.uPreset < 1.5)
        {
            float spin = t * 0.119999997317790985107421875;
            float angle = ((in.aUv.x * 2.0) * 3.1415927410125732421875) + spin;
            float flow = in.aUv.y - ((t * 0.07999999821186065673828125) * (1.0 + (_465.uBass * 0.550000011920928955078125)));
            flow = fract(flow);
            float zPos = (flow - 0.5) * 9.0;
            float baseR = 2.0 - ((_465.uBass * 0.2800000011920928955078125) * K);
            float ripG = ((sin(((angle * 5.0) + (zPos * 1.39999997615814208984375)) + (t * 2.2000000476837158203125)) * 0.100000001490116119384765625) * (_465.uMid + _465.uTreble)) * K;
            float r = baseR + ripG;
            pos.x = cos(angle) * r;
            pos.y = sin(angle) * r;
            pos.z = zPos;
            sampleUv = float2(in.aUv.x, flow);
            float2 param_11 = sampleUv;
            sampleUv = safeCoverUv(param_11);
            float2 param_12 = sampleUv;
            newCol = sampleNewCoverColor(param_12, uCoverTex, uCoverTexSmplr);
            float2 param_13 = sampleUv;
            prevCol = samplePrevCoverColor(param_13, uPrevCoverTex, uPrevCoverTexSmplr);
            coverColor = mix(prevCol, newCol, float3(fast::clamp(_465.uColorMixT, 0.0, 1.0)));
            out.vColor = mix(defaultColor, coverColor, float3(_465.uHasCover));
            float depthFade = smoothstep(-4.5, 4.5, zPos);
            out.vColor *= (0.4000000059604644775390625 + (depthFade * 0.699999988079071044921875));
        }
        else
        {
            if (_465.uPreset < 2.5)
            {
                float theta = (in.aUv.x * 2.0) * 3.1415927410125732421875;
                float phi = (in.aUv.y - 0.5) * 3.1415927410125732421875;
                float baseR_1 = 2.2000000476837158203125;
                float3 param_14 = float3(theta * 1.5, phi * 1.5, t * 0.699999988079071044921875);
                float trebFlare = ((snoise(param_14) * _465.uTreble) * 0.85000002384185791015625) * K;
                float bassExpand = (_465.uBass * 0.3499999940395355224609375) * K;
                float r_1 = (baseR_1 * (1.0 + bassExpand)) + trebFlare;
                pos.x = (r_1 * cos(phi)) * cos(theta);
                pos.y = r_1 * sin(phi);
                pos.z = (r_1 * cos(phi)) * sin(theta);
                float yaw = t * 0.180000007152557373046875;
                float cy = cos(yaw);
                float sy = sin(yaw);
                float3 _1043 = pos;
                float2 _1045 = float2x2(float2(cy, -sy), float2(sy, cy)) * _1043.xz;
                pos.x = _1045.x;
                pos.z = _1045.y;
            }
            else
            {
                if (_465.uPreset < 3.5)
                {
                    pos = float3((in.aUv.x - 0.5) * 0.00999999977648258209228515625, (in.aUv.y - 0.5) * 0.00999999977648258209228515625, -90.0);
                    out.vAlpha = 0.0;
                    out.vColor = float3(0.0);
                    maxRippleAmp = 0.0;
                }
                else
                {
                    if (_465.uPreset < 4.5)
                    {
                        float bassDrive = smoothstep(0.07999999821186065673828125, 0.7799999713897705078125, _465.uBass + (_465.uBeat * 0.819999992847442626953125));
                        float highDrive = smoothstep(0.0500000007450580596923828125, 0.4600000083446502685546875, _465.uTreble);
                        float hiResGuard = smoothstep(1.08000004291534423828125, 1.5499999523162841796875, _465.uCoverRes);
                        float edgeGuard = mix(1.0, 0.37999999523162841796875, hiResGuard);
                        float depthGuard = mix(1.0, 0.439999997615814208984375, hiResGuard);
                        float grooveGuard = mix(1.0, 0.4799999892711639404296875, hiResGuard);
                        float beatGuard = mix(1.0, 0.36000001430511474609375, hiResGuard);
                        float2 p = (in.aUv - float2(0.5)) * 5.11999988555908203125;
                        float spin_1 = _465.uVinylSpin;
                        float cs = cos(spin_1);
                        float sn = sin(spin_1);
                        float2 rp = float2x2(float2(cs, -sn), float2(sn, cs)) * p;
                        float d = length(p);
                        float angle0 = precise::atan2(p.y, p.x);
                        float recordR = 2.46000003814697265625;
                        float coverR = 1.17999994754791259765625;
                        float recordAlpha = 1.0 - smoothstep(recordR - 0.0199999995529651641845703125, recordR + 0.0500000007450580596923828125, d);
                        float coverMask = 1.0 - smoothstep(coverR - 0.01200000010430812835693359375, coverR + 0.017999999225139617919921875, d);
                        float border = exp(-powr((d - coverR) / 0.064000003039836883544921875, 2.0)) * edgeGuard;
                        float outerRim = exp(-powr((d - (recordR - 0.0500000007450580596923828125)) / 0.054999999701976776123046875, 2.0)) * edgeGuard;
                        float vinylN = fast::clamp((d - coverR) / fast::max(0.001000000047497451305389404296875, recordR - coverR), 0.0, 1.0);
                        pos = float3(rp * ((1.0 + ((bassDrive * 0.01200000010430812835693359375) * beatGuard)) + ((_465.uBeat * 0.026000000536441802978515625) * beatGuard)), 0.0);
                        out.vAlpha = recordAlpha;
                        if (coverMask > 0.0199999995529651641845703125)
                        {
                            float2 coverUv = (p / float2(coverR * 2.0)) + float2(0.5);
                            float2 param_15 = coverUv;
                            newCol = sampleNewCoverColor(param_15, uCoverTex, uCoverTexSmplr);
                            float2 param_16 = coverUv;
                            prevCol = samplePrevCoverColor(param_16, uPrevCoverTex, uPrevCoverTexSmplr);
                            coverColor = mix(prevCol, newCol, float3(fast::clamp(_465.uColorMixT, 0.0, 1.0)));
                            if (hiResGuard > 0.001000000047497451305389404296875)
                            {
                                float2 sx = float2(0.00260000000707805156707763671875, 0.0);
                                float2 sy_1 = float2(0.0, 0.00260000000707805156707763671875);
                                float2 param_17 = coverUv + sx;
                                float2 param_18 = coverUv - sx;
                                float2 param_19 = coverUv + sy_1;
                                float2 param_20 = coverUv - sy_1;
                                float3 softNew = (((sampleNewCoverColor(param_17, uCoverTex, uCoverTexSmplr) + sampleNewCoverColor(param_18, uCoverTex, uCoverTexSmplr)) + sampleNewCoverColor(param_19, uCoverTex, uCoverTexSmplr)) + sampleNewCoverColor(param_20, uCoverTex, uCoverTexSmplr)) * 0.25;
                                float2 param_21 = coverUv + sx;
                                float2 param_22 = coverUv - sx;
                                float2 param_23 = coverUv + sy_1;
                                float2 param_24 = coverUv - sy_1;
                                float3 softPrev = (((samplePrevCoverColor(param_21, uPrevCoverTex, uPrevCoverTexSmplr) + samplePrevCoverColor(param_22, uPrevCoverTex, uPrevCoverTexSmplr)) + samplePrevCoverColor(param_23, uPrevCoverTex, uPrevCoverTexSmplr)) + samplePrevCoverColor(param_24, uPrevCoverTex, uPrevCoverTexSmplr)) * 0.25;
                                coverColor = mix(coverColor, mix(softPrev, softNew, float3(fast::clamp(_465.uColorMixT, 0.0, 1.0))), float3(hiResGuard * 0.4199999868869781494140625));
                            }
                            out.vColor = mix(defaultColor, coverColor, float3(_465.uHasCover));
                            float coverShade = 1.019999980926513671875 + (0.100000001490116119384765625 * (1.0 - smoothstep(0.0, coverR, d)));
                            out.vColor *= coverShade;
                            out.vColor = mix(out.vColor, float3(1.0), float3(border * 0.540000021457672119140625));
                            pos.z = (0.039999999105930328369140625 + ((border * 0.026000000536441802978515625) * depthGuard)) + ((_465.uBeat * 0.017999999225139617919921875) * beatGuard);
                            maxRippleAmp = fast::max(maxRippleAmp, ((border * 0.300000011920928955078125) + ((bassDrive * 0.07500000298023223876953125) * beatGuard)) + ((_465.uBeat * 0.07500000298023223876953125) * beatGuard));
                        }
                        else
                        {
                            float groove = 0.5 + (0.5 * sin((d - coverR) * mix(98.0, 58.0, hiResGuard)));
                            float fineGroove = 0.5 + (0.5 * sin(((d - coverR) * mix(170.0, 92.0, hiResGuard)) + (in.aRand * 3.0)));
                            float param_25 = floor((angle0 + 3.1415927410125732421875) * 38.0) + (floor(d * 72.0) * 2.099999904632568359375);
                            float tick = smoothstep(0.819999992847442626953125, 0.99500000476837158203125, hash11(param_25));
                            float3 vinyl = (float3(0.05200000107288360595703125, 0.0540000014007091522216796875, 0.0579999983310699462890625) + (float3(0.05200000107288360595703125 * grooveGuard) * groove)) + (float3(0.026000000536441802978515625 * grooveGuard) * fineGroove);
                            vinyl = mix(vinyl, coverColor * 0.319999992847442626953125, float3(0.180000007152557373046875 * (1.0 - vinylN)));
                            float whiteRing = fast::max(border * 0.920000016689300537109375, outerRim * 0.2599999904632568359375);
                            out.vColor = mix(vinyl, float3(0.920000016689300537109375, 0.939999997615814208984375, 0.939999997615814208984375), float3(whiteRing));
                            out.vColor = mix(out.vColor, float3(1.0), float3(((tick * highDrive) * (0.0599999986588954925537109375 + (border * 0.119999997317790985107421875))) * grooveGuard));
                            pos.z = ((((groove * 0.00999999977648258209228515625) * grooveGuard) + ((border * 0.0240000002086162567138671875) * depthGuard)) + ((((bassDrive * vinylN) * 0.01600000075995922088623046875) * K) * beatGuard)) + (((tick * highDrive) * 0.00999999977648258209228515625) * grooveGuard);
                            maxRippleAmp = fast::max(maxRippleAmp, ((((border * 0.319999992847442626953125) + (outerRim * 0.119999997317790985107421875)) + (((bassDrive * vinylN) * 0.10999999940395355224609375) * beatGuard)) + (((tick * highDrive) * 0.100000001490116119384765625) * grooveGuard)) + (((_465.uBeat * vinylN) * 0.07999999821186065673828125) * beatGuard));
                        }
                    }
                    else
                    {
                        if (_465.uPreset < 8.5)
                        {
                            float bassGlow = (smoothstep(0.070000000298023223876953125, 0.7799999713897705078125, _465.uBass) * 0.3400000035762786865234375) + (_465.uBeat * 0.01400000043213367462158203125);
                            float midGlow = smoothstep(0.070000000298023223876953125, 0.62000000476837158203125, _465.uMid) * 0.4199999868869781494140625;
                            float highGlow = smoothstep(0.039999999105930328369140625, 0.4600000083446502685546875, _465.uTreble) * 0.4600000083446502685546875;
                            float lane = in.aUv.y;
                            float transition = fast::clamp(_465.uBurstAmt, 0.0, 1.0);
                            if (lane < 0.800000011920928955078125)
                            {
                                float3 param_26 = float3(in.aUv.x * 0.4199999868869781494140625, lane * 1.7000000476837158203125, t * 0.026000000536441802978515625);
                                float param_27 = in.aRand * 73.09999847412109375;
                                float laneWarp = (snoise(param_26) * 0.10999999940395355224609375) + ((hash11(param_27) - 0.5) * 0.04500000178813934326171875);
                                float warpedLane = fast::clamp(lane + laneWarp, 0.0, 0.800000011920928955078125);
                                float3 param_28 = float3(in.aUv.x * 0.819999992847442626953125, lane * 2.25, t * 0.0320000015199184417724609375);
                                float bandCoord = ((warpedLane / 0.800000011920928955078125) * 5.650000095367431640625) + (snoise(param_28) * 0.62000000476837158203125);
                                float band = floor(bandCoord);
                                float param_29 = (band * 9.13000011444091796875) + (in.aRand * 2.400000095367431640625);
                                float local = fract(bandCoord + (hash11(param_29) * 0.180000007152557373046875));
                                float bandN = fast::clamp((band + 0.5) / 5.650000095367431640625, 0.0, 1.0);
                                float param_30 = (band * 19.1700000762939453125) + (in.aRand * 31.0);
                                float seed = hash11(param_30);
                                float flow_1 = fract((in.aUv.x + (t * ((0.00340000004507601261138916015625 + (bandN * 0.003800000064074993133544921875)) + (seed * 0.002199999988079071044921875)))) + (seed * 0.5299999713897705078125));
                                float arc = ((flow_1 - 0.5) * 3.1415927410125732421875) * ((1.35000002384185791015625 + (bandN * 0.7200000286102294921875)) + (seed * 0.23999999463558197021484375));
                                float armCurve = sin((arc + (bandN * 2.2000000476837158203125)) + (seed * 5.30000019073486328125));
                                float spiralRadius = ((9.19999980926513671875 + (bandN * 11.80000019073486328125)) + (seed * 6.0)) + (local * 2.900000095367431640625);
                                float x = (cos(((arc * 0.7200000286102294921875) + (bandN * 0.920000016689300537109375)) + (seed * 1.2999999523162841796875)) * spiralRadius) + ((flow_1 - 0.5) * (13.5 + (bandN * 9.5)));
                                float ribbonPhase = ((((flow_1 * 3.1415927410125732421875) * 2.0) * ((0.550000011920928955078125 + (bandN * 0.23999999463558197021484375)) + (seed * 0.100000001490116119384765625))) + (t * (0.00999999977648258209228515625 + (bandN * 0.007000000216066837310791015625)))) + (seed * 5.69999980926513671875);
                                float broadWave = sin(ribbonPhase) * 0.920000016689300537109375;
                                float fineWave = sin(((ribbonPhase * (1.36000001430511474609375 + (seed * 0.62000000476837158203125))) - (t * 0.0439999997615814208984375)) + (seed * 5.0)) * 0.04500000178813934326171875;
                                float3 param_31 = float3(bandN * 2.0, flow_1 * 0.62000000476837158203125, seed);
                                float yBase = ((((bandN - 0.5) * 13.19999980926513671875) + (armCurve * (2.2999999523162841796875 + (bandN * 1.60000002384185791015625)))) + ((seed - 0.5) * 1.85000002384185791015625)) + (snoise(param_31) * 0.920000016689300537109375);
                                float ridgeCenter = 0.430000007152557373046875 + ((seed - 0.5) * 0.180000007152557373046875);
                                float ridge = exp(-powr((local - ridgeCenter) / (0.25 + (seed * 0.039999999105930328369140625)), 2.0));
                                float softMask = smoothstep(0.00999999977648258209228515625, 0.119999997317790985107421875, lane) * (1.0 - smoothstep(0.7200000286102294921875, 0.810000002384185791015625, lane));
                                float3 param_32 = float3((flow_1 * 1.17999994754791259765625) + seed, bandN * 2.0, t * 0.017999999225139617919921875);
                                float ribbonNoise = snoise(param_32) * 0.7400000095367431640625;
                                float zLayer = mix(-23.5, 15.5, bandN) + ((seed - 0.5) * 6.0);
                                pos.x = (x + (ribbonNoise * 1.39999997615814208984375)) + (sin((t * 0.01200000010430812835693359375) + (seed * 8.0)) * 0.2199999988079071044921875);
                                pos.y = ((yBase + broadWave) + fineWave) + ((local - 0.5) * (0.579999983310699462890625 + (ridge * 0.14000000059604644775390625)));
                                pos.z = (zLayer + (broadWave * 1.35000002384185791015625)) + (ribbonNoise * 1.85000002384185791015625);
                                float pulseLine = 0.5 + (0.5 * sin(((ribbonPhase * (1.7000000476837158203125 + (seed * 0.89999997615814208984375))) - (t * 0.319999992847442626953125)) + (seed * 6.0)));
                                float3 aurora = mix(float3(0.519999980926513671875, 0.86000001430511474609375, 1.0), float3(0.699999988079071044921875, 0.579999983310699462890625, 1.0), float3(bandN));
                                aurora = mix(aurora, float3(0.959999978542327880859375, 0.980000019073486328125, 0.920000016689300537109375), float3(bassGlow * 0.0500000007450580596923828125));
                                out.vAlpha = ((((0.180000007152557373046875 + (ridge * 0.7799999713897705078125)) + ((pulseLine * highGlow) * 0.0350000001490116119384765625)) + (bassGlow * 0.02500000037252902984619140625)) * softMask) * (0.959999978542327880859375 + (transition * 0.0199999995529651641845703125));
                                out.vColor = mix(coverColor, aurora, float3(0.62000000476837158203125 + (ridge * 0.2199999988079071044921875))) * (((0.7599999904632568359375 + (ridge * 0.86000001430511474609375)) + ((pulseLine * highGlow) * 0.0500000007450580596923828125)) + (bassGlow * 0.039999999105930328369140625));
                                maxRippleAmp = fast::max(maxRippleAmp, ((ridge * (0.119999997317790985107421875 + (midGlow * 0.0500000007450580596923828125))) + ((pulseLine * highGlow) * 0.04500000178813934326171875)) + (bassGlow * 0.02999999932944774627685546875));
                            }
                            else
                            {
                                float q = (lane - 0.800000011920928955078125) / 0.20000000298023223876953125;
                                float param_33 = (in.aRand * 917.0) + floor(q * 130.0);
                                float seed_1 = hash11(param_33);
                                float depth = mix(-32.0, 18.0, seed_1);
                                float drift = fract((in.aUv.x + (t * (0.0013999999500811100006103515625 + (seed_1 * 0.004800000227987766265869140625)))) + (seed_1 * 0.62999999523162841796875));
                                float3 param_34 = float3(seed_1 * 2.0, q * 3.2000000476837158203125, t * 0.007000000216066837310791015625);
                                float cluster = snoise(param_34);
                                float x_1 = ((drift - 0.5) * (45.0 + (seed_1 * 22.0))) + (cluster * 3.400000095367431640625);
                                float param_35 = (in.aRand * 331.0) + (seed_1 * 5.0);
                                float y = ((hash11(param_35) - 0.5) * 22.0) + (sin((t * (0.017999999225139617919921875 + (seed_1 * 0.0280000008642673492431640625))) + (seed_1 * 7.0)) * 0.86000001430511474609375);
                                float z = depth + (sin((t * (0.0199999995529651641845703125 + (seed_1 * 0.0320000015199184417724609375))) + (in.aRand * 8.0)) * 1.0499999523162841796875);
                                float twinkle = powr(0.5 + (0.5 * sin((t * (0.23999999463558197021484375 + (seed_1 * 0.4199999868869781494140625))) + (in.aRand * 17.0))), 5.0);
                                float param_36 = (in.aRand * 661.0) + floor(q * 160.0);
                                float dust = smoothstep(0.2199999988079071044921875, 0.980000019073486328125, hash11(param_36));
                                pos = float3(x_1, y, z);
                                out.vAlpha = (dust * (((0.1599999964237213134765625 + (twinkle * 0.4600000083446502685546875)) + (highGlow * 0.02500000037252902984619140625)) + (bassGlow * 0.017999999225139617919921875))) * (1.0 - (q * 0.0599999986588954925537109375));
                                out.vColor = mix(coverColor, float3(0.920000016689300537109375, 0.9700000286102294921875, 1.0), float3(0.62000000476837158203125 + (twinkle * 0.14000000059604644775390625))) * ((0.7200000286102294921875 + (twinkle * 0.62000000476837158203125)) + (bassGlow * 0.02500000037252902984619140625));
                                maxRippleAmp = fast::max(maxRippleAmp, ((twinkle * highGlow) * 0.054999999701976776123046875) + ((dust * bassGlow) * 0.02999999932944774627685546875));
                            }
                            if (transition > 0.001000000047497451305389404296875)
                            {
                                float bloom = smoothstep(0.0, 1.0, transition);
                                float param_37 = in.aRand * 31.0;
                                float param_38 = in.aRand * 47.0;
                                float2 burstVec = pos.xy + (float2(hash11(param_37) - 0.5, hash11(param_38) - 0.5) * 0.75);
                                float2 burstDir = burstVec / float2(fast::max(length(burstVec), 0.001000000047497451305389404296875));
                                float3 _2173 = pos;
                                float2 _2175 = _2173.xy + ((burstDir * bloom) * 0.026000000536441802978515625);
                                pos.x = _2175.x;
                                pos.y = _2175.y;
                                float3 param_39 = float3(in.aRand, t * 0.01400000043213367462158203125, 1.0);
                                float3 param_40 = float3(in.aRand, t * 0.01400000043213367462158203125, 5.0);
                                float3 _2196 = pos;
                                float2 _2198 = _2196.xy + ((float2(snoise(param_39), snoise(param_40)) * bloom) * 0.0599999986588954925537109375);
                                pos.x = _2198.x;
                                pos.y = _2198.y;
                                float3 _2206 = pos;
                                float2 _2208 = _2206.xy * (1.0 + (bloom * 0.01400000043213367462158203125));
                                pos.x = _2208.x;
                                pos.y = _2208.y;
                                float param_41 = in.aRand * 123.0;
                                pos.z += (((hash11(param_41) - 0.5) * bloom) * 0.180000007152557373046875);
                                out.vAlpha *= (0.86000001430511474609375 + (bloom * 0.2199999988079071044921875));
                                maxRippleAmp = fast::max(maxRippleAmp, bloom * 0.100000001490116119384765625);
                            }
                        }
                        else
                        {
                            if (_465.uPreset < 9.5)
                            {
                                float ringIndex = floor(in.aUv.y * 8.0);
                                float ringLocal = fract(in.aUv.y * 8.0);
                                float ringN = (ringIndex + 0.5) / 8.0;
                                float param_42 = (ringIndex * 19.729999542236328125) + 2.099999904632568359375;
                                float ringSeed = hash11(param_42);
                                float theta_1 = (((in.aUv.x * 2.0) * 3.1415927410125732421875) + (ringIndex * 0.4699999988079071044921875)) + (t * (0.0350000001490116119384765625 + (ringN * 0.026000000536441802978515625)));
                                float eclipseDrive = (smoothstep(0.0599999986588954925537109375, 0.7599999904632568359375, _465.uBass) * 0.17000000178813934326171875) + (_465.uBeat * 0.07999999821186065673828125);
                                float ridge_1 = exp(-powr((ringLocal - 0.5) / (0.17000000178813934326171875 + (ringSeed * 0.04500000178813934326171875)), 2.0));
                                float radius = ((0.920000016689300537109375 + (ringN * 3.1800000667572021484375)) + ((ringLocal - 0.5) * 0.23999999463558197021484375)) + (eclipseDrive * (0.180000007152557373046875 + (ringN * 0.3400000035762786865234375)));
                                float eccentric = 0.4900000095367431640625 + (ringN * 0.20000000298023223876953125);
                                float xh = (cos(theta_1) * radius) * (1.17999994754791259765625 + (ringN * 0.100000001490116119384765625));
                                float yh = (sin(theta_1) * radius) * eccentric;
                                float zh = ((ringN - 0.5) * 1.34000003337860107421875) + (sin((theta_1 * 2.0) + (ringSeed * 6.0)) * (0.100000001490116119384765625 + (ringN * 0.0900000035762786865234375)));
                                float tilt = (-0.3400000035762786865234375) + (ringN * 0.7200000286102294921875);
                                float ct = cos(tilt);
                                float st = sin(tilt);
                                pos = float3(xh, (yh * ct) - (zh * st), (yh * st) + (zh * ct));
                                float3 coldRim = float3(0.4799999892711639404296875, 0.819999992847442626953125, 1.0);
                                float3 antiqueGold = float3(0.930000007152557373046875, 0.7200000286102294921875, 0.37999999523162841796875);
                                float3 haloColor = mix(coldRim, antiqueGold, float3(smoothstep(0.180000007152557373046875, 0.839999973773956298828125, ringN + (sin(theta_1) * 0.07999999821186065673828125))));
                                haloColor = mix(haloColor, float3(0.980000019073486328125, 0.959999978542327880859375, 0.87999999523162841796875), float3(ridge_1 * (0.2800000011920928955078125 + (_465.uTreble * 0.180000007152557373046875))));
                                out.vColor = mix(coverColor * 0.7200000286102294921875, haloColor, float3(0.62000000476837158203125 + (ridge_1 * 0.20000000298023223876953125)));
                                float corona = powr(0.5 + (0.5 * sin(((theta_1 * (13.0 + ringIndex)) - (t * 0.519999980926513671875)) + (ringSeed * 9.0))), 8.0);
                                out.vAlpha = (((0.180000007152557373046875 + (ridge_1 * 0.7400000095367431640625)) + (corona * (0.07999999821186065673828125 + (_465.uTreble * 0.180000007152557373046875)))) * smoothstep(0.0199999995529651641845703125, 0.14000000059604644775390625, in.aUv.y)) * (1.0 - smoothstep(0.930000007152557373046875, 1.0, in.aUv.y));
                                maxRippleAmp = fast::max(maxRippleAmp, (ridge_1 * (0.119999997317790985107421875 + (eclipseDrive * 0.20000000298023223876953125))) + ((corona * _465.uTreble) * 0.12999999523162841796875));
                            }
                            else
                            {
                                if (_465.uPreset < 10.5)
                                {
                                    float column = floor(in.aUv.x * 52.0);
                                    float columnN = (column + 0.5) / 52.0;
                                    float columnLocal = fract(in.aUv.x * 52.0);
                                    float param_43 = (column * 41.1699981689453125) + (floor(columnLocal * 5.0) * 7.900000095367431640625);
                                    float rainSeed = hash11(param_43);
                                    float rainSpeed = (0.026000000536441802978515625 + (rainSeed * 0.05200000107288360595703125)) + (smoothstep(0.07999999821186065673828125, 0.7599999904632568359375, _465.uBass) * 0.01400000043213367462158203125);
                                    float fall = fract(((1.0 - in.aUv.y) + (t * rainSpeed)) + (rainSeed * 0.87000000476837158203125));
                                    float xRain = ((columnN - 0.5) * 11.80000019073486328125) + ((columnLocal - 0.5) * 0.10999999940395355224609375);
                                    float yRain = (0.5 - fall) * 8.80000019073486328125;
                                    float param_44 = (column * 11.30000019073486328125) + 0.699999988079071044921875;
                                    float cityDepth = mix(-3.7999999523162841796875, 2.599999904632568359375, hash11(param_44));
                                    float wind = sin(((t * 0.17000000178813934326171875) + (column * 0.62999999523162841796875)) + (fall * 3.400000095367431640625)) * (0.07999999821186065673828125 + (rainSeed * 0.119999997317790985107421875));
                                    float perspective = 0.7599999904632568359375 + (smoothstep(-3.7999999523162841796875, 2.599999904632568359375, cityDepth) * 0.3400000035762786865234375);
                                    pos = float3((xRain + wind) * perspective, yRain, cityDepth + (sin(((fall * 3.1415927410125732421875) * 2.0) + (rainSeed * 7.0)) * 0.0500000007450580596923828125));
                                    float dash = powr(0.5 + (0.5 * sin((fall * (52.0 + (rainSeed * 38.0))) - (t * 0.319999992847442626953125))), 10.0);
                                    float dropHead = powr(0.5 + (0.5 * sin((fall * 17.0) + (rainSeed * 12.0))), 18.0);
                                    float3 neonA = float3(0.20000000298023223876953125, 0.87999999523162841796875, 1.0);
                                    float3 neonB = float3(1.0, 0.2599999904632568359375, 0.62999999523162841796875);
                                    float3 neonC = float3(1.0, 0.7200000286102294921875, 0.300000011920928955078125);
                                    float3 rainColor = mix(neonA, neonB, float3(smoothstep(0.180000007152557373046875, 0.7799999713897705078125, rainSeed)));
                                    rainColor = mix(rainColor, neonC, float3(smoothstep(0.839999973773956298828125, 0.9900000095367431640625, rainSeed)));
                                    out.vColor = mix(coverColor * 0.819999992847442626953125, rainColor, float3(0.579999983310699462890625 + (dash * 0.180000007152557373046875)));
                                    out.vAlpha = (0.0900000035762786865234375 + (dash * 0.4000000059604644775390625)) + (dropHead * (0.2199999988079071044921875 + (_465.uTreble * 0.300000011920928955078125)));
                                    out.vAlpha *= (0.579999983310699462890625 + (perspective * 0.4199999868869781494140625));
                                    pos.x += (((dropHead * sin((t * 0.800000011920928955078125) + (rainSeed * 8.0))) * _465.uTreble) * 0.054999999701976776123046875);
                                    maxRippleAmp = fast::max(maxRippleAmp, ((dash * _465.uMid) * 0.100000001490116119384765625) + (dropHead * (0.100000001490116119384765625 + (_465.uTreble * 0.2199999988079071044921875))));
                                }
                                else
                                {
                                    if (_465.uPreset < 11.5)
                                    {
                                        float flockIndex = floor(in.aUv.y * 12.0);
                                        float wingY = (fract(in.aUv.y * 12.0) * 2.0) - 1.0;
                                        float wingX = (in.aUv.x * 2.0) - 1.0;
                                        float param_45 = (flockIndex * 23.729999542236328125) + 4.0;
                                        float flockSeed = hash11(param_45);
                                        float gx = mod(flockIndex, 4.0) - 1.5;
                                        float gy = floor(flockIndex / 4.0) - 1.0;
                                        float migrate = t * (0.017999999225139617919921875 + (flockSeed * 0.017999999225139617919921875));
                                        float2 flockCenter = float2((gx * 2.4200000762939453125) + (sin(migrate + (flockSeed * 8.0)) * 0.4799999892711639404296875), (gy * 2.099999904632568359375) + (cos((migrate * 0.829999983310699462890625) + (flockSeed * 9.0)) * 0.36000001430511474609375));
                                        float ax = abs(wingX);
                                        float wingMask = 1.0 - smoothstep(0.7200000286102294921875, 1.17999994754791259765625, abs(wingY) + (ax * 0.37999999523162841796875));
                                        float bodyMask = exp(-powr(wingX / 0.07500000298023223876953125, 2.0)) * (1.0 - smoothstep(0.62000000476837158203125, 1.0, abs(wingY)));
                                        float flap = sin((t * (0.7200000286102294921875 + (flockSeed * 0.3499999940395355224609375))) + (flockIndex * 1.7000000476837158203125)) * ((0.3400000035762786865234375 + (_465.uMid * 0.4199999868869781494140625)) + (_465.uBeat * 0.1599999964237213134765625));
                                        float lx = sign(wingX) * (0.119999997317790985107421875 + (powr(ax, 0.7599999904632568359375) * 0.87999999523162841796875));
                                        float ly = wingY * (0.4799999892711639404296875 + ((1.0 - ax) * 0.2199999988079071044921875));
                                        float lz = (ax * (0.4199999868869781494140625 + flap)) + ((1.0 - abs(wingY)) * 0.07999999821186065673828125);
                                        float heading = (-0.20000000298023223876953125) + ((flockSeed - 0.5) * 0.699999988079071044921875);
                                        float ch = cos(heading);
                                        float sh = sin(heading);
                                        float2 folded = float2x2(float2(ch, -sh), float2(sh, ch)) * float2(lx, ly);
                                        pos = float3(flockCenter + folded, ((flockSeed - 0.5) * 3.400000095367431640625) + lz);
                                        float3 prismLeft = float3(0.4199999868869781494140625, 0.939999997615814208984375, 0.819999992847442626953125);
                                        float3 prismRight = float3(0.959999978542327880859375, 0.62000000476837158203125, 0.920000016689300537109375);
                                        float3 prism = mix(prismLeft, prismRight, float3(smoothstep(-0.800000011920928955078125, 0.800000011920928955078125, wingX)));
                                        prism = mix(prism, float3(1.0, 0.910000026226043701171875, 0.63999998569488525390625), float3(smoothstep(0.7200000286102294921875, 1.0, ax)));
                                        out.vColor = mix(coverColor * 0.7799999713897705078125, prism, float3(0.60000002384185791015625 + (wingMask * 0.180000007152557373046875)));
                                        out.vAlpha = (wingMask * ((0.2199999988079071044921875 + ((1.0 - ax) * 0.4199999868869781494140625)) + (_465.uTreble * 0.1599999964237213134765625))) + (bodyMask * 0.680000007152557373046875);
                                        maxRippleAmp = fast::max(maxRippleAmp, (wingMask * (0.070000000298023223876953125 + (abs(flap) * 0.119999997317790985107421875))) + ((bodyMask * _465.uBass) * 0.119999997317790985107421875));
                                    }
                                    else
                                    {
                                        float bloomR = powr(in.aUv.y, 0.7200000286102294921875);
                                        float bloomTheta = (((in.aUv.x * 2.0) * 3.1415927410125732421875) + (bloomR * 0.7200000286102294921875)) + (t * 0.017999999225139617919921875);
                                        float petalWave = 0.5 + (0.5 * cos((bloomTheta * 9.0) - (bloomR * 3.7999999523162841796875)));
                                        float bloomDrive = (smoothstep(0.0599999986588954925537109375, 0.7400000095367431640625, _465.uBass) * 0.2599999904632568359375) + (_465.uBeat * 0.100000001490116119384765625);
                                        float radiusBloom = (bloomR * (3.2999999523162841796875 + bloomDrive)) * (0.7799999713897705078125 + (petalWave * 0.3400000035762786865234375));
                                        float petalFold = sin(((bloomTheta * 9.0) - (bloomR * 5.400000095367431640625)) + (t * 0.12999999523162841796875));
                                        float bowl = ((1.0 - bloomR) * 1.12000000476837158203125) - ((bloomR * bloomR) * 0.519999980926513671875);
                                        pos.x = cos(bloomTheta) * radiusBloom;
                                        pos.y = (sin(bloomTheta) * radiusBloom) * 0.819999992847442626953125;
                                        pos.z = (bowl + (petalFold * (0.12999999523162841796875 + (bloomR * 0.4699999988079071044921875)))) + (sin((bloomTheta * 3.0) + (t * 0.070000000298023223876953125)) * 0.07999999821186065673828125);
                                        float bloomTilt = -0.300000011920928955078125;
                                        float cb = cos(bloomTilt);
                                        float sb = sin(bloomTilt);
                                        float3 _3010 = pos;
                                        float2 _3012 = float2x2(float2(cb, -sb), float2(sb, cb)) * _3010.yz;
                                        pos.y = _3012.x;
                                        pos.z = _3012.y;
                                        float3 abyss = float3(0.119999997317790985107421875, 0.579999983310699462890625, 0.560000002384185791015625);
                                        float3 violet = float3(0.37999999523162841796875, 0.319999992847442626953125, 0.949999988079071044921875);
                                        float3 pearl = float3(0.819999992847442626953125, 1.0, 0.939999997615814208984375);
                                        float3 bloomColor = mix(abyss, violet, float3(smoothstep(0.180000007152557373046875, 0.87999999523162841796875, bloomR + (petalWave * 0.100000001490116119384765625))));
                                        float luminousEdge = smoothstep(0.63999998569488525390625, 0.980000019073486328125, bloomR) * (0.550000011920928955078125 + (petalWave * 0.449999988079071044921875));
                                        bloomColor = mix(bloomColor, pearl, float3(luminousEdge * (0.319999992847442626953125 + (_465.uTreble * 0.2199999988079071044921875))));
                                        out.vColor = mix(coverColor * 0.7200000286102294921875, bloomColor, float3(0.680000007152557373046875));
                                        float filament = powr(0.5 + (0.5 * sin(((bloomTheta * 18.0) + (bloomR * 19.0)) - (t * 0.2599999904632568359375))), 7.0);
                                        out.vAlpha = (((0.1599999964237213134765625 + (petalWave * 0.310000002384185791015625)) + (filament * 0.2599999904632568359375)) + (luminousEdge * 0.300000011920928955078125)) * smoothstep(0.00999999977648258209228515625, 0.100000001490116119384765625, bloomR);
                                        maxRippleAmp = fast::max(maxRippleAmp, (((petalWave * bloomDrive) * 0.180000007152557373046875) + ((filament * _465.uTreble) * 0.17000000178813934326171875)) + ((luminousEdge * _465.uMid) * 0.0900000035762786865234375));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    bool _3104 = _465.uMouseActive > 0.5;
    bool _3110;
    if (_3104)
    {
        _3110 = _465.uPreset < 0.5;
    }
    else
    {
        _3110 = _3104;
    }
    if (_3110)
    {
        float mdx = pos.x - _465.uMouseXY.x;
        float mdy = pos.y - _465.uMouseXY.y;
        float md = sqrt((mdx * mdx) + (mdy * mdy));
        if (md < 1.0)
        {
            float push = (1.0 - md) * (1.0 - md);
            pos.z += (push * 0.550000011920928955078125);
        }
    }
    if (_465.uHandActive > 0.00999999977648258209228515625)
    {
        float hdx = pos.x - _465.uHandXY.x;
        float hdy = pos.y - _465.uHandXY.y;
        float hd = sqrt((hdx * hdx) + (hdy * hdy));
        float rad = 1.5499999523162841796875;
        if (hd < rad)
        {
            float push_1 = (rad - hd) / rad;
            push_1 = (push_1 * push_1) * _465.uHandActive;
            pos.z += (push_1 * 1.10000002384185791015625);
            float2 outDir = float2(hdx, hdy) / float2(fast::max(0.001000000047497451305389404296875, hd));
            float3 _3216 = pos;
            float2 _3218 = _3216.xy + ((outDir * push_1) * 0.2800000011920928955078125);
            pos.x = _3218.x;
            pos.y = _3218.y;
        }
    }
    if (_465.uGestureGrip > 0.001000000047497451305389404296875)
    {
        float grip = fast::clamp(_465.uGestureGrip, 0.0, 1.0);
        float gripWave = 0.5 + (0.5 * sin((_465.uTime * 2.2000000476837158203125) + (in.aRand * 6.283100128173828125)));
        float3 _3250 = pos;
        float2 _3252 = _3250.xy * mix(1.0, 0.660000026226043701171875 + (gripWave * 0.0350000001490116119384765625), grip);
        pos.x = _3252.x;
        pos.y = _3252.y;
        pos.z += (grip * ((0.180000007152557373046875 + (_465.uBass * 0.2199999988079071044921875)) + (gripWave * 0.100000001490116119384765625)));
    }
    if (_465.uScatter > 0.001000000047497451305389404296875)
    {
        float2 jdir = float2(cos(in.aRand * 6.283100128173828125), sin(in.aRand * 6.283100128173828125));
        float3 _3293 = pos;
        float2 _3295 = _3293.xy + ((jdir * _465.uScatter) * (0.0500000007450580596923828125 + (_465.uTreble * 0.100000001490116119384765625)));
        pos.x = _3295.x;
        pos.y = _3295.y;
    }
    bool _3302 = _465.uTwist > 0.001000000047497451305389404296875;
    bool _3308;
    if (_3302)
    {
        _3308 = _465.uPreset < 0.5;
    }
    else
    {
        _3308 = _3302;
    }
    if (_3308)
    {
        float ta = (_465.uTwist * pos.z) * 0.60000002384185791015625;
        float cs_1 = cos(ta);
        float sn_1 = sin(ta);
        float3 _3332 = pos;
        float2 _3334 = float2x2(float2(cs_1, -sn_1), float2(sn_1, cs_1)) * _3332.xy;
        pos.x = _3334.x;
        pos.y = _3334.y;
    }
    float vinylHiResGuard = (smoothstep(1.08000004291534423828125, 1.5499999523162841796875, _465.uCoverRes) * step(3.5, _465.uPreset)) * (1.0 - step(4.5, _465.uPreset));
    float edgeBoost = (_465.uEdgeEnabled * edgeVal) * mix(1.0, 0.4199999868869781494140625, vinylHiResGuard);
    out.vSourceLum = dot(fast::max(out.vColor, float3(0.0)), float3(0.2989999949932098388671875, 0.58700001239776611328125, 0.114000000059604644775390625));
    float blackParticleGuard = 1.0 - smoothstep(0.02500000037252902984619140625, 0.115000002086162567138671875, out.vSourceLum);
    out.vEdgeBoost = (edgeBoost * ((_465.uPreset > 3.5) ? 0.2199999988079071044921875 : 1.0)) * (1.0 - blackParticleGuard);
    out.vColor = powr(fast::max(out.vColor, float3(0.0)), float3(1.0 / fast::max(0.3499999940395355224609375, _465.uColorBoost)));
    float edgeColorMix = (edgeBoost * ((_465.uPreset > 3.5) ? 0.20000000298023223876953125 : 0.5)) * (1.0 - blackParticleGuard);
    out.vColor = mix(out.vColor, out.vColor + float3(0.20000000298023223876953125), float3(edgeColorMix));
    float tintLum = fast::max(fast::max(out.vColor.x, out.vColor.y), out.vColor.z);
    float3 tintedColor = float3(_465.uTintColor) * fast::max(0.23999999463558197021484375, tintLum * 1.12000000476837158203125);
    out.vColor = mix(out.vColor, tintedColor, float3(fast::clamp(_465.uTintStrength, 0.0, 1.0) * (1.0 - blackParticleGuard)));
    out.vBright = ((((0.819999992847442626953125 + (maxRippleAmp * 0.550000011920928955078125)) + (_465.uBass * 0.100000001490116119384765625)) + (edgeBoost * 0.300000011920928955078125)) + (_465.uEnergy * 0.0500000007450580596923828125)) + (_465.uBurstAmt * 0.4000000059604644775390625);
    if (_465.uPreset > 4.5)
    {
        out.vBright = (((0.939999997615814208984375 + (maxRippleAmp * 0.3400000035762786865234375)) + (_465.uBass * 0.0199999995529651641845703125)) + (_465.uEnergy * 0.026000000536441802978515625)) + (_465.uBurstAmt * 0.02500000037252902984619140625);
        if (_465.uPreset > 8.5)
        {
            out.vBright = ((0.86000001430511474609375 + (maxRippleAmp * 0.519999980926513671875)) + (_465.uEnergy * 0.04500000178813934326171875)) + (_465.uBeat * 0.054999999701976776123046875);
        }
    }
    else
    {
        if (_465.uPreset > 3.5)
        {
            out.vBright = (((((0.939999997615814208984375 + (maxRippleAmp * 0.63999998569488525390625)) + (_465.uBass * 0.07999999821186065673828125)) + (edgeBoost * 0.119999997317790985107421875)) + (_465.uEnergy * 0.0500000007450580596923828125)) + (_465.uBeat * 0.1599999964237213134765625)) + (_465.uBurstAmt * 0.1599999964237213134765625);
        }
    }
    out.vRipple = fast::clamp(maxRippleAmp * 1.5, 0.0, 1.0);
    bool _3529 = _465.uHasDepth > 0.5;
    bool _3535;
    if (_3529)
    {
        _3535 = _465.uPreset < 0.5;
    }
    else
    {
        _3535 = _3529;
    }
    if (_3535)
    {
        float bgMul = mix(1.0, 0.550000011920928955078125, _465.uBgFade * (1.0 - fgMask));
        out.vBright *= bgMul;
    }
    out.vBright += (_465.uGestureGrip * 0.2199999988079071044921875);
    float loadingMistSize = 1.0;
    if (_465.uLoading > 0.001000000047497451305389404296875)
    {
        float param_46 = in.aRand * 931.70001220703125;
        float mistSeed = hash11(param_46);
        float mistLayer = floor(mistSeed * 4.0);
        float layerN = (mistLayer + 0.5) / 4.0;
        float3 param_47 = float3(in.aRand * 2.099999904632568359375, _465.uTime * 0.23999999463558197021484375, 2.0);
        float mistAngle = ((in.aRand * 6.283100128173828125) + (_465.uTime * (0.1599999964237213134765625 + (mistSeed * 0.180000007152557373046875)))) + (snoise(param_47) * 1.85000002384185791015625);
        float param_48 = in.aRand * 127.3000030517578125;
        float mistR = mix(1.35000002384185791015625, 3.150000095367431640625, sqrt(hash11(param_48))) * (1.0 + (sin((_465.uTime * 0.4199999868869781494140625) + (in.aRand * 7.0)) * 0.12999999523162841796875));
        float3 param_49 = float3(in.aRand * 4.099999904632568359375, _465.uTime * 0.319999992847442626953125, 3.0);
        float3 param_50 = float3(in.aRand * 4.69999980926513671875, _465.uTime * 0.300000011920928955078125, 8.0);
        float2 mistCurl = float2(snoise(param_49), snoise(param_50));
        float mistBreath = 0.5 + (0.5 * sin((_465.uTime * (0.819999992847442626953125 + (mistSeed * 0.550000011920928955078125))) + (in.aRand * 17.0)));
        float mistRibbon = sin(((mistAngle * (1.35000002384185791015625 + (layerN * 0.550000011920928955078125))) + (_465.uTime * 0.3400000035762786865234375)) + (mistSeed * 4.0));
        float param_51 = (in.aRand * 1501.0) + (mistLayer * 17.0);
        float glowPick = smoothstep(0.87999999523162841796875, 0.9969999790191650390625, hash11(param_51));
        float dustPick = 0.3400000035762786865234375 + (glowPick * 0.660000026226043701171875);
        float3 mistPos = float3(((cos(mistAngle) * mistR) * (1.2400000095367431640625 + (mistCurl.x * 0.1599999964237213134765625))) + (mistCurl.x * 0.7200000286102294921875), ((sin((mistAngle * 0.819999992847442626953125) + (mistRibbon * 0.25)) * mistR) * (0.560000002384185791015625 + (layerN * 0.100000001490116119384765625))) + (mistCurl.y * 0.62000000476837158203125), ((((layerN - 0.5) * 4.849999904632568359375) + (mistCurl.x * 0.560000002384185791015625)) + (mistBreath * 0.36000001430511474609375)) + (mistRibbon * 0.23999999463558197021484375));
        float3 mistCol = mix(float3(0.62000000476837158203125, 0.86000001430511474609375, 0.839999973773956298828125), float3(0.36000001430511474609375, 0.4600000083446502685546875, 0.7799999713897705078125), float3(mistSeed));
        mistCol = mix(mistCol, float3(0.939999997615814208984375, 1.0, 0.9700000286102294921875), float3(glowPick * (0.449999988079071044921875 + (mistBreath * 0.3499999940395355224609375))));
        out.vColor = mix(out.vColor, mistCol, float3(_465.uLoading * 0.7799999713897705078125));
        out.vBright = mix(out.vBright, ((0.20000000298023223876953125 + (mistBreath * 0.180000007152557373046875)) + (abs(mistCurl.x) * 0.0599999986588954925537109375)) + (glowPick * (0.7200000286102294921875 + (abs(mistRibbon) * 0.23999999463558197021484375))), _465.uLoading);
        out.vAlpha = mix(out.vAlpha, ((0.07999999821186065673828125 + (mistBreath * 0.10999999940395355224609375)) + (dustPick * 0.10999999940395355224609375)) + (glowPick * 0.300000011920928955078125), _465.uLoading);
        pos = mix(pos, mistPos, float3(_465.uLoading));
        loadingMistSize = ((1.2599999904632568359375 + (mistBreath * 0.23999999463558197021484375)) + (abs(mistRibbon) * 0.14000000059604644775390625)) + (glowPick * 0.7799999713897705078125);
    }
    float4 mvPos = _465.modelViewMatrix * float4(pos, 1.0);
    float depthSize = 36.0 / fast::max(0.5, -mvPos.z);
    float audioBoost = (((1.0 + (maxRippleAmp * 0.699999988079071044921875)) + (edgeBoost * 0.550000011920928955078125)) + (_465.uBeat * 0.300000011920928955078125)) + (_465.uBurstAmt * 0.5);
    float sz = fast::clamp(depthSize * audioBoost, 1.0499999523162841796875, 4.94999980926513671875);
    if (_465.uPreset > 4.5)
    {
        float flowDrive = ((((_465.uBass * 0.070000000298023223876953125) + (_465.uMid * 0.0460000000894069671630859375)) + (_465.uTreble * 0.0599999986588954925537109375)) + (_465.uBurstAmt * 0.0900000035762786865234375)) + (_465.uBeat * 0.054999999701976776123046875);
        sz = fast::clamp(depthSize * (1.0499999523162841796875 + flowDrive), 1.0, 5.44999980926513671875);
        if (_465.uPreset > 8.5)
        {
            float authoredDrive = (((_465.uBass * 0.100000001490116119384765625) + (_465.uMid * 0.07999999821186065673828125)) + (_465.uTreble * 0.119999997317790985107421875)) + (_465.uBeat * 0.100000001490116119384765625);
            sz = fast::clamp(depthSize * (0.980000019073486328125 + authoredDrive), 1.0, 4.849999904632568359375);
        }
    }
    else
    {
        if (_465.uPreset > 3.5)
        {
            float ringDrive = (((_465.uBass * 0.300000011920928955078125) + (_465.uMid * 0.180000007152557373046875)) + (_465.uTreble * 0.2199999988079071044921875)) + (_465.uBeat * 0.300000011920928955078125);
            sz = fast::clamp(depthSize * (0.89999997615814208984375 + (ringDrive * 0.62000000476837158203125)), 1.0499999523162841796875, 3.900000095367431640625);
        }
    }
    sz = mix(sz, sz * loadingMistSize, _465.uLoading);
    out.gl_PointSize = ((sz * _465.uPixel) * _465.uPointScale) * _465.uBloomSize;
    out.gl_Position = _465.projectionMatrix * mvPos;
    return out;
}
