// Mineradio d43de565acabfdc1a9c9820a27e81a98ccbebcef, GPL-3.0.
// R-Music port: selected visual declarations. Upstream DOM events, network and startup hooks excluded.
// ============================================================
var FLOAT_COUNT = 1300;

var floatGroup = null;

var floatPositionsArr = null, floatBaseArr = null, floatPhaseArr = null, floatColorArr = null;


function createFloatLayer() {
  fx.floatLayer = false;
  uniforms.uFloatAlpha.value = 0;
  if (floatGroup) destroyFloatLayer();
  return;
  if (floatGroup) return;
  var fgeo = new THREE.BufferGeometry();
  floatPositionsArr = new Float32Array(FLOAT_COUNT * 3);
  floatBaseArr = new Float32Array(FLOAT_COUNT * 3);  // 基准位置
  floatPhaseArr = new Float32Array(FLOAT_COUNT * 3);  // 每粒子相位 (0..2π)
  floatColorArr = new Float32Array(FLOAT_COUNT * 3);
  var floatRandArr = new Float32Array(FLOAT_COUNT);
  var floatAmpArr = new Float32Array(FLOAT_COUNT);      // 漂移幅度 (0.15-0.45)
  for (var i = 0; i < FLOAT_COUNT; i++) {
    var halo = i < FLOAT_COUNT * 0.76;
    var bx, by, bz;
    if (halo) {
      var a = Math.random() * Math.PI * 2;
      var r = 0.62 + Math.pow(Math.random(), 0.72) * 2.75;
      var lane = (Math.random() - 0.5) * 0.62;
      bx = Math.cos(a) * r;
      by = Math.sin(a) * r * 0.54 + lane;
      bz = (Math.random() - 0.5) * 2.4 - 0.25;
    } else {
      bx = (Math.random() - 0.5) * 8.4;
      by = (Math.random() - 0.5) * 5.8;
      bz = (Math.random() - 0.5) * 5.6;
    }
    floatBaseArr[i * 3] = bx; floatBaseArr[i * 3 + 1] = by; floatBaseArr[i * 3 + 2] = bz;
    floatPositionsArr[i * 3] = bx;
    floatPositionsArr[i * 3 + 1] = by;
    floatPositionsArr[i * 3 + 2] = bz;
    floatPhaseArr[i * 3] = Math.random() * Math.PI * 2;
    floatPhaseArr[i * 3 + 1] = Math.random() * Math.PI * 2;
    floatPhaseArr[i * 3 + 2] = Math.random() * Math.PI * 2;
    floatAmpArr[i] = 0.15 + Math.random() * 0.35;
    var white = 0.88 + Math.random() * 0.12;
    floatColorArr[i * 3] = white;
    floatColorArr[i * 3 + 1] = white;
    floatColorArr[i * 3 + 2] = white;
    floatRandArr[i] = Math.random();
  }
  fgeo.setAttribute('position', new THREE.BufferAttribute(floatPositionsArr, 3));
  fgeo.setAttribute('aColor', new THREE.BufferAttribute(floatColorArr, 3));
  fgeo.setAttribute('aRand', new THREE.BufferAttribute(floatRandArr, 1));

  // 把 amp + phase 存到 attribute 让 shader 端做漂移 (避免 JS 每帧改 buffer)
  fgeo.setAttribute('aAmp', new THREE.BufferAttribute(floatAmpArr, 1));
  fgeo.setAttribute('aPhase', new THREE.BufferAttribute(floatPhaseArr, 3));

  var fvs = `
precision highp float;
uniform float uTime, uBass, uPixel, uFloatAlpha;
attribute vec3 aColor;
attribute vec3 aPhase;
attribute float aRand, aAmp;
varying vec3 vC;
varying float vA;
void main(){
  vec3 pos = position;
  float orbit = uTime * (0.030 + aRand * 0.034);
  float cs = cos(orbit), sn = sin(orbit);
  pos.xy = mat2(cs, -sn, sn, cs) * pos.xy;
  float breathe = 1.0 + sin(uTime * 0.34 + aPhase.x) * 0.045;
  pos.xy *= breathe;
  pos.x += sin(uTime * (0.18 + aRand * 0.05) + aPhase.x) * aAmp * 0.34;
  pos.y += cos(uTime * (0.15 + aRand * 0.06) + aPhase.y) * aAmp * 0.30;
  pos.z += sin(uTime * (0.11 + aRand * 0.04) + aPhase.z) * aAmp * 0.68 + uBass * 0.10 * sin(aRand * 12.0);
  vC = aColor;
  vec4 mvPos = modelViewMatrix * vec4(pos, 1.0);
  float dist = -mvPos.z;
  float twinkle = 0.62 + 0.38 * sin(uTime * (0.42 + aRand * 0.34) + aPhase.z);
  vA = clamp(0.22 + (5.0 - dist) * 0.10, 0.055, 0.58) * twinkle;
  float sz = clamp(40.0 / max(0.5, dist), 1.3, 4.1);
  gl_PointSize = sz * uPixel;
  gl_Position = projectionMatrix * mvPos;
}
  `;
  var ffs = `
precision highp float;
uniform sampler2D uDotTex;
uniform float uFloatAlpha;
varying vec3 vC;
varying float vA;
void main(){
  vec4 tex = texture2D(uDotTex, gl_PointCoord);
  if (tex.a < 0.02) discard;
  gl_FragColor = vec4(vC, tex.a * vA * uFloatAlpha);
}
  `;
  var fmat = new THREE.ShaderMaterial({
    uniforms: {
      uTime: uniforms.uTime,
      uBass: uniforms.uBass,
      uPixel: uniforms.uPixel,
      uDotTex: uniforms.uDotTex,
      uFloatAlpha: uniforms.uFloatAlpha,
    },
    vertexShader: fvs, fragmentShader: ffs,
    transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
  });
  floatGroup = new THREE.Points(fgeo, fmat);
  floatGroup.frustumCulled = false;
  scene.add(floatGroup);
}

function destroyFloatLayer() {
  if (!floatGroup) return;
  scene.remove(floatGroup);
  floatGroup.geometry.dispose(); floatGroup.material.dispose();
  floatGroup = null;
}


// ============================================================
//  封面背面粒子层 (v7.2)
//   - 独立 Points, 放在 z=-1.5 (主封面平面背面)
//   - 颜色取自封面镜像 UV
//   - 慢呼吸 + 小幅 noise 漂移
//   - 跟主粒子同步旋转 (在主循环里赋值)
//   - 视角转到背面才能看到 — 不需要手动控制 visible
// ============================================================
var BACK_COVER_COUNT = 3000;

var backCoverGroup = null;

var backCoverColorArr = null;


function createBackCoverLayer() {
  if (backCoverGroup) return;
  var bg = new THREE.BufferGeometry();
  var bp = new Float32Array(BACK_COVER_COUNT * 3);
  var bc = new Float32Array(BACK_COVER_COUNT * 3);
  var br = new Float32Array(BACK_COVER_COUNT);
  var bu = new Float32Array(BACK_COVER_COUNT * 2);  // 镜像 UV 用于采样封面
  for (var i = 0; i < BACK_COVER_COUNT; i++) {
    var u = Math.random();
    var v = Math.random();
    // 在 PLANE_SIZE 范围内分布
    bp[i * 3] = (u - 0.5) * PLANE_SIZE;
    bp[i * 3 + 1] = (v - 0.5) * PLANE_SIZE;
    bp[i * 3 + 2] = -1.5 - Math.random() * 0.4;  // 在主平面后方
    bu[i * 2] = 1.0 - u;  // 镜像 X
    bu[i * 2 + 1] = v;
    br[i] = Math.random();
    bc[i * 3] = 0.7; bc[i * 3 + 1] = 0.6; bc[i * 3 + 2] = 0.8;  // 占位
  }
  bg.setAttribute('position', new THREE.BufferAttribute(bp, 3));
  bg.setAttribute('aColor', new THREE.BufferAttribute(bc, 3));
  bg.setAttribute('aRand', new THREE.BufferAttribute(br, 1));
  bg.setAttribute('aUv', new THREE.BufferAttribute(bu, 2));

  var vs = `
precision highp float;
uniform float uTime, uBass, uPixel, uAlpha;
attribute vec3 aColor;
attribute vec2 aUv;
attribute float aRand;
varying vec3 vC;
varying float vA;
void main(){
  vec3 pos = position;
  // 缓慢呼吸
  pos.x += sin(uTime * 0.20 + aRand * 8.0) * 0.20;
  pos.y += cos(uTime * 0.18 + aRand * 6.0) * 0.22;
  pos.z += sin(uTime * 0.12 + aRand * 5.0) * 0.18 + uBass * 0.12 * sin(aRand * 11.0);
  vC = aColor;
  vec4 mvPos = modelViewMatrix * vec4(pos, 1.0);
  float dist = -mvPos.z;
  vA = clamp(0.30 + 0.4 * sin(uTime * 0.6 + aRand * 5.0), 0.10, 0.65);
  float sz = clamp(46.0 / max(0.5, dist), 1.4, 4.5);
  gl_PointSize = sz * uPixel;
  gl_Position = projectionMatrix * mvPos;
}
  `;
  var fs = `
precision highp float;
uniform sampler2D uDotTex;
uniform float uAlpha;
varying vec3 vC;
varying float vA;
void main(){
  vec4 tex = texture2D(uDotTex, gl_PointCoord);
  if (tex.a < 0.02) discard;
  gl_FragColor = vec4(vC, tex.a * vA * uAlpha);
}
  `;
  var mat = new THREE.ShaderMaterial({
    uniforms: {
      uTime: uniforms.uTime,
      uBass: uniforms.uBass,
      uPixel: uniforms.uPixel,
      uDotTex: uniforms.uDotTex,
      uAlpha: uniforms.uAlpha,
    },
    vertexShader: vs, fragmentShader: fs,
    transparent: true, depthWrite: false, blending: THREE.NormalBlending,
  });
  backCoverGroup = new THREE.Points(bg, mat);
  backCoverGroup.frustumCulled = false;
  backCoverColorArr = bc;
  scene.add(backCoverGroup);
}


function destroyBackCoverLayer() {
  if (!backCoverGroup) return;
  scene.remove(backCoverGroup);
  backCoverGroup.geometry.dispose(); backCoverGroup.material.dispose();
  backCoverGroup = null; backCoverColorArr = null;
}


function refreshBackCoverColorsFromCanvas(coverCanvas) {
  if (!backCoverGroup || !coverCanvas || !backCoverColorArr) return;
  var ctx = coverCanvas.getContext('2d');
  var img = ctx.getImageData(0, 0, coverCanvas.width, coverCanvas.height).data;
  var w = coverCanvas.width, h = coverCanvas.height;
  var attr = backCoverGroup.geometry.attributes;
  var uvA = attr.aUv.array;
  for (var i = 0; i < BACK_COVER_COUNT; i++) {
    var u = uvA[i * 2], v = uvA[i * 2 + 1];
    var sx = Math.floor(u * w);
    var sy = Math.floor(v * h);
    var di = (sy * w + sx) * 4;
    backCoverColorArr[i * 3] = img[di] / 255 * 0.85;
    backCoverColorArr[i * 3 + 1] = img[di + 1] / 255 * 0.85;
    backCoverColorArr[i * 3 + 2] = img[di + 2] / 255 * 0.85;
  }
  attr.aColor.needsUpdate = true;
}

function updateFloatLayer(dt) {
  // 漂移已在 shader 中完成, JS 不需要每帧改 buffer
}

function refreshFloatColorsFromCover(coverCanvas) {
  if (!floatGroup || !coverCanvas) return;
  var ctx = coverCanvas.getContext('2d');
  var img = ctx.getImageData(0, 0, coverCanvas.width, coverCanvas.height).data;
  var w = coverCanvas.width, h = coverCanvas.height;
  for (var i = 0; i < FLOAT_COUNT; i++) {
    var sx = Math.floor(Math.random() * w);
    var sy = Math.floor(Math.random() * h);
    var di = (sy * w + sx) * 4;
    floatColorArr[i * 3] = img[di] / 255 * 0.95;
    floatColorArr[i * 3 + 1] = img[di + 1] / 255 * 0.95;
    floatColorArr[i * 3 + 2] = img[di + 2] / 255 * 0.95;
  }
  floatGroup.geometry.attributes.aColor.needsUpdate = true;
}

function resetFloatColorsToIdle() {
  if (!floatGroup || !floatColorArr) return;
  for (var i = 0; i < FLOAT_COUNT; i++) {
    var white = 0.88 + (i % 17) / 17 * 0.12;
    floatColorArr[i * 3] = white;
    floatColorArr[i * 3 + 1] = white;
    floatColorArr[i * 3 + 2] = white;
  }
  floatGroup.geometry.attributes.aColor.needsUpdate = true;
}