// Mineradio d43de565, GPL-3.0. Generated native MSL; see NOTICE.md.
#include <metal_stdlib>
#include <simd/simd.h>

using namespace metal;

struct StageUniforms
{
    float uTime;
    float uPixel;
    float uPointScale;
    float uBloomStrength;
    float uColorBoost;
    float uBass;
    float uMid;
    float uTreble;
    float uBeat;
    float uJawOpen;
    float uSkullFlash;
    float3 uColorA;
    float3 uColorB;
    float3 uShadow;
    packed_float3 uLight;
    float uOpacity;
    float4x4 modelViewMatrix;
    float4x4 projectionMatrix;
    float3x3 normalMatrix;
};

struct skull_vert_out
{
    float vKind [[user(locn0)]];
    float vLight [[user(locn1)]];
    float vRim [[user(locn2)]];
    float vAmp [[user(locn3)]];
    float vDensity [[user(locn4)]];
    float vFlash [[user(locn5)]];
    float4 gl_Position [[position]];
    float gl_PointSize [[point_size]];
};

struct skull_vert_in
{
    float seed [[attribute(0)]];
    float kind [[attribute(1)]];
    float3 position [[attribute(2)]];
};

vertex skull_vert_out skull_vert(skull_vert_in in [[stage_in]], constant StageUniforms& _220 [[buffer(0)]])
{
    skull_vert_out out = {};
    float3 pos = in.position;
    float jawGroup = step(1.0, in.kind);
    float boneKind = fract(in.kind);
    out.vKind = boneKind;
    float3 n = fast::normalize(float3(in.position.x * 0.819999992847442626953125, in.position.y * 0.680000007152557373046875, (in.position.z * 1.2200000286102294921875) + 0.1599999964237213134765625));
    float toothBand = (smoothstep(0.4799999892711639404296875, 0.699999988079071044921875, in.position.z) * (1.0 - smoothstep(0.2700000107288360595703125, 0.4799999892711639404296875, abs(in.position.x)))) * (1.0 - smoothstep(0.180000007152557373046875, 0.4600000083446502685546875, abs(in.position.y + 0.7200000286102294921875)));
    float toothNoise = fract(sin((in.seed * 21.7310009002685546875) + (floor((in.position.x + 0.519999980926513671875) * 21.0) * 5.13700008392333984375)) * 43758.546875);
    pos.y += ((toothBand * (toothNoise - 0.5)) * 0.0199999995529651641845703125);
    pos.z += ((toothBand * (fract(sin((in.seed * 17.923000335693359375) + (in.position.y * 31.0)) * 24634.634765625) - 0.5)) * 0.01200000010430812835693359375);
    float jawSidePull = (((jawGroup * smoothstep(-0.4199999868869781494140625, -1.059999942779541015625, in.position.y)) * smoothstep(0.23999999463558197021484375, 0.62000000476837158203125, abs(in.position.x))) * (1.0 - smoothstep(0.7799999713897705078125, 1.03999996185302734375, abs(in.position.x)))) * smoothstep(0.1599999964237213134765625, 0.699999988079071044921875, in.position.z);
    pos.x *= (1.0 - (jawSidePull * 0.100000001490116119384765625));
    float fallbackJaw = (smoothstep(-0.4799999892711639404296875, -0.89999997615814208984375, in.position.y) * smoothstep(0.07999999821186065673828125, 0.519999980926513671875, in.position.z)) * (1.0 - smoothstep(0.62000000476837158203125, 0.959999978542327880859375, abs(in.position.x)));
    float jawMask = jawGroup;
    float jawSideAnchor = ((smoothstep(0.36000001430511474609375, 0.660000026226043701171875, abs(in.position.x)) * (1.0 - smoothstep(0.7799999713897705078125, 0.980000019073486328125, abs(in.position.x)))) * smoothstep(-0.3400000035762786865234375, -0.7400000095367431640625, in.position.y)) * (1.0 - smoothstep(0.62000000476837158203125, 0.86000001430511474609375, in.position.z));
    float jawMotion = jawMask * (1.0 - (jawSideAnchor * 0.319999992847442626953125));
    float2 jawHinge = float2(-0.449999988079071044921875, 0.180000007152557373046875);
    float jawAngle = (_220.uJawOpen * 0.519999980926513671875) * jawMotion;
    float jc = cos(jawAngle);
    float js = sin(jawAngle);
    float2 jr = pos.yz - jawHinge;
    float2 openedJaw = float2((jr.x * jc) - (jr.y * js), (jr.x * js) + (jr.y * jc)) + jawHinge;
    float3 _262 = pos;
    float2 _267 = mix(_262.yz, openedJaw, float2(jawMotion));
    pos.y = _267.x;
    pos.z = _267.y;
    float jawDrop = (jawMotion * smoothstep(-0.319999992847442626953125, -0.87999999523162841796875, in.position.y)) * (0.579999983310699462890625 + (smoothstep(0.180000007152557373046875, 0.62000000476837158203125, abs(in.position.x)) * 0.039999999105930328369140625));
    float openDrive = fast::clamp(_220.uJawOpen, 0.0, 1.25);
    pos.y -= (jawDrop * (0.0379999987781047821044921875 + (openDrive * 0.100000001490116119384765625)));
    pos.z += (jawDrop * (0.0030000000260770320892333984375 + (openDrive * 0.01400000043213367462158203125)));
    float ampDrive = smoothstep(0.20000000298023223876953125, 0.819999992847442626953125, ((_220.uBass * 0.439999997615814208984375) + (_220.uMid * 0.2199999988079071044921875)) + (_220.uBeat * 0.7200000286102294921875));
    float ampPhase = 0.5 + (0.5 * sin((_220.uTime * (1.0499999523162841796875 + (_220.uMid * 0.300000011920928955078125))) + (in.seed * 6.283100128173828125)));
    out.vFlash = fast::clamp(_220.uSkullFlash * (0.680000007152557373046875 + (ampPhase * 0.319999992847442626953125)), 0.0, 1.0);
    out.vAmp = fast::clamp(((ampDrive * 0.04500000178813934326171875) + (out.vFlash * 0.920000016689300537109375)) + (_220.uTreble * 0.01200000010430812835693359375), 0.0, 1.0);
    float4 mv = _220.modelViewMatrix * float4(pos, 1.0);
    float dist = fast::max(0.550000011920928955078125, -mv.z);
    float3 vn = fast::normalize(_220.normalMatrix * n);
    float3 keyDir = float3(-0.4799999892711639404296875, 0.63999998569488525390625, 0.60000002384185791015625);
    float3 lowDir = float3(-0.116721801459789276123046875, -0.910430014133453369140625, 0.396854102611541748046875);
    float3 fillDir = float3(0.48953545093536376953125, -0.05439282953739166259765625, 0.8702852725982666015625);
    float3 rimDir = float3(0.87982404232025146484375, 0.179964005947113037109375, -0.439912021160125732421875);
    float key = powr(fast::max(dot(vn, keyDir), 0.0), 1.17999994754791259765625);
    float low = powr(fast::max(dot(vn, lowDir), 0.0), 1.34000003337860107421875) * 0.100000001490116119384765625;
    float fill = fast::max(dot(vn, fillDir), 0.0) * 0.054999999701976776123046875;
    float gothicShadow = smoothstep(-0.100000001490116119384765625, 0.36000001430511474609375, dot(vn, float3(0.6023442745208740234375, -0.0821378529071807861328125, -0.793999254703521728515625)));
    float dentalLift = ((smoothstep(0.4799999892711639404296875, 0.7200000286102294921875, in.position.z) * (1.0 - smoothstep(0.300000011920928955078125, 0.540000021457672119140625, abs(in.position.x)))) * (1.0 - smoothstep(0.180000007152557373046875, 0.4799999892711639404296875, abs(in.position.y + 0.699999988079071044921875)))) * (0.62000000476837158203125 + (toothNoise * 0.20000000298023223876953125));
    out.vRim = powr(fast::max(dot(vn, rimDir), 0.0), 2.5) * ((0.23999999463558197021484375 + (_220.uBloomStrength * 0.07999999821186065673828125)) + (out.vFlash * 0.62000000476837158203125));
    float dust = fract(sin(((in.seed * 13.8710002899169921875) + (in.position.x * 19.700000762939453125)) + (in.position.y * 7.099999904632568359375)) * 43758.546875);
    out.vDensity = fast::clamp(((((0.300000011920928955078125 + (key * 0.699999988079071044921875)) + (out.vRim * 0.23999999463558197021484375)) - (gothicShadow * 0.23999999463558197021484375)) + (dust * 0.02500000037252902984619140625)) + (out.vFlash * 0.07999999821186065673828125), 0.1599999964237213134765625, 1.2000000476837158203125);
    out.vLight = fast::clamp(((((((0.115000002086162567138671875 + (key * 1.019999980926513671875)) + low) + fill) + (dentalLift * 0.20000000298023223876953125)) + (boneKind * 0.070000000298023223876953125)) + (out.vAmp * 0.560000002384185791015625)) - (gothicShadow * 0.07999999821186065673828125), 0.0350000001490116119384765625, 1.7200000286102294921875);
    float scaleCtl = fast::clamp(_220.uPointScale, 0.4799999892711639404296875, 2.349999904632568359375);
    float size = (0.0350000001490116119384765625 + (boneKind * 0.026000000536441802978515625)) * ((((0.839999973773956298828125 + (out.vDensity * 0.2199999988079071044921875)) + (out.vLight * 0.12999999523162841796875)) + (_220.uBloomStrength * 0.02999999932944774627685546875)) + (out.vFlash * 0.180000007152557373046875));
    out.gl_PointSize = fast::clamp((((size * _220.uPixel) * scaleCtl) * 128.0) / dist, 0.949999988079071044921875, 7.599999904632568359375);
    out.gl_Position = _220.projectionMatrix * mv;
    return out;
}
