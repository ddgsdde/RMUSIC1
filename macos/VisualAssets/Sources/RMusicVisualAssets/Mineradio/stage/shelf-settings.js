// Mineradio d43de565acabfdc1a9c9820a27e81a98ccbebcef, GPL-3.0.
// R-Music port: selected visual declarations. Upstream DOM events, network and startup hooks excluded.

function normalizeShelfCameraMode(value) {
  return String(value || '') === 'static' ? 'static' : 'dynamic';
}

function shelfDefaultAngleForCameraMode(mode) {
  return normalizeShelfCameraMode(mode) === 'static' ? -15 : 0;
}

function normalizeShelfPresence(value) {
  return String(value || '') === 'always' ? 'always' : 'auto';
}

function normalizedShelfNumber(key, fallback, min, max) {
  var value = fx && fx[key] != null ? Number(fx[key]) : fallback;
  if (!isFinite(value)) value = fallback;
  return clampRange(value, min, max);
}

function shelfDetailSettings() {
  return {
    x: normalizedShelfNumber('shelfDetailOffsetX', fxDefaults.shelfDetailOffsetX, -4.8, 4.8),
    y: normalizedShelfNumber('shelfDetailOffsetY', fxDefaults.shelfDetailOffsetY, -3.6, 3.6),
    z: normalizedShelfNumber('shelfDetailOffsetZ', fxDefaults.shelfDetailOffsetZ, -3.6, 3.6),
    scale: normalizedShelfNumber('shelfDetailScale', fxDefaults.shelfDetailScale, 0.72, 1.35),
    rx: normalizedShelfNumber('shelfDetailAngleX', fxDefaults.shelfDetailAngleX, -24, 24) * Math.PI / 180,
    ry: normalizedShelfNumber('shelfDetailAngleY', fxDefaults.shelfDetailAngleY, -28, 28) * Math.PI / 180,
    rowGap: normalizedShelfNumber('shelfDetailRowGap', fxDefaults.shelfDetailRowGap, 0.72, 1.32),
    openDuration: normalizedShelfNumber('shelfDetailOpenDuration', fxDefaults.shelfDetailOpenDuration, 0.12, 1.2),
    closeDuration: normalizedShelfNumber('shelfDetailCloseDuration', fxDefaults.shelfDetailCloseDuration, 0.08, 0.8),
    rowDuration: normalizedShelfNumber('shelfDetailRowDuration', fxDefaults.shelfDetailRowDuration, 0.16, 1.6),
    intro: normalizedShelfNumber('shelfDetailIntroStrength', fxDefaults.shelfDetailIntroStrength, 0, 1.8),
    parallax: normalizedShelfNumber('shelfDetailParallax', fxDefaults.shelfDetailParallax, 0, 1.8)
  };
}

function shelfSummonSettings() {
  return {
    openDuration: normalizedShelfNumber('shelfSummonOpenDuration', fxDefaults.shelfSummonOpenDuration, 0.08, 2),
    closeDuration: normalizedShelfNumber('shelfSummonCloseDuration', fxDefaults.shelfSummonCloseDuration, 0.08, 1.6),
    slide: normalizedShelfNumber('shelfSummonSlide', fxDefaults.shelfSummonSlide, 0, 4),
    stagger: normalizedShelfNumber('shelfSummonStagger', fxDefaults.shelfSummonStagger, 0, 3),
    scale: normalizedShelfNumber('shelfSummonScale', fxDefaults.shelfSummonScale, 0, 3),
    parallax: normalizedShelfNumber('shelfSummonParallax', fxDefaults.shelfSummonParallax, 0, 2.5),
    cameraEnterSpeed: normalizedShelfNumber('shelfCameraEnterSpeed', fxDefaults.shelfCameraEnterSpeed, 0.2, 1.5),
    cameraExitSpeed: normalizedShelfNumber('shelfCameraExitSpeed', fxDefaults.shelfCameraExitSpeed, 0.2, 1.5)
  };
}

function durationEaseFactor(seconds, dt) {
  seconds = Math.max(0.016, Number(seconds) || 0.016);
  dt = Math.max(1 / 240, Number(dt) || 1 / 60);
  return clampRange(1 - Math.exp(-dt / seconds), 0.001, 1);
}

function shelfSettings() {
  var angleDeg = fx && fx.shelfAngleYManual === true
    ? normalizedShelfNumber('shelfAngleY', shelfDefaultAngleForCameraMode(fx.shelfCameraMode), -30, 30)
    : shelfDefaultAngleForCameraMode(fx && fx.shelfCameraMode);
  return {
    size: normalizedShelfNumber('shelfSize', fxDefaults.shelfSize, 0.65, 1.45),
    x: normalizedShelfNumber('shelfOffsetX', fxDefaults.shelfOffsetX, -1.2, 1.2),
    y: normalizedShelfNumber('shelfOffsetY', fxDefaults.shelfOffsetY, -0.9, 0.9),
    z: normalizedShelfNumber('shelfOffsetZ', fxDefaults.shelfOffsetZ, -0.9, 0.9),
    angle: angleDeg * Math.PI / 180,
    opacity: normalizedShelfNumber('shelfOpacity', fxDefaults.shelfOpacity, 0.25, 1),
    bgOpacity: normalizedShelfNumber('shelfBgOpacity', fxDefaults.shelfBgOpacity, 0.25, 0.98),
    accent: normalizeHexColor((fx && fx.shelfAccentColor) || fxDefaults.shelfAccentColor, fxDefaults.shelfAccentColor)
  };
}

function shelfAlwaysVisible() {
  return !!(fx && normalizeShelfPresence(fx.shelfPresence) === 'always');
}

function shouldUseShelfDynamicCamera(type) {
  if (!/^shelf-/.test(String(type || ''))) return true;
  return !(fx && normalizeShelfCameraMode(fx.shelfCameraMode) === 'static');
}

function shelfAccentHex() {
  return normalizeHexColor((fx && fx.shelfAccentColor) || fxDefaults.shelfAccentColor, fxDefaults.shelfAccentColor);
}

function shelfAccentRgba(alpha, fallback) {
  var rgb = hexToRgb(shelfAccentHex());
  if (!rgb) return fallback || 'rgba(244,210,138,' + alpha + ')';
  return 'rgba(' + rgb.r + ',' + rgb.g + ',' + rgb.b + ',' + alpha + ')';
}