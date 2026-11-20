/* Fit actual glyphs after camera motion, including context and translation rows.
   Only the lyric render pass changes; no audio, textures or background projection. */
'use strict';
function rmusicLyricViewportFit() {
  const point = new THREE.Vector3(), transform = new THREE.Matrix4();
  let scale = 1, offsetX = 0, offsetY = 0, last = 0;
  return function(camera, lyrics, rect, now = performance.now()) {
    let left = Infinity, right = -Infinity, bottom = Infinity, top = -Infinity;
    lyrics.updateWorldMatrix(true, true);
    camera.updateMatrixWorld();
    lyrics.traverse(node => {
      const rows = node.userData?.lyric?.rowLayers;
      if (!rows || node.visible === false) return;
      for (const row of rows) {
        if (!row.mesh?.visible || !row.lineMask || row.targetAlpha < 0.01) continue;
        let visible = true;
        for (let parent = row.mesh.parent; parent; parent = parent.parent) if (!parent.visible) { visible = false; break; }
        if (!visible) continue;
        const opacity = row.mat?.uniforms?.uOpacity?.value ?? row.mat?.opacity ?? 1;
        if (opacity < 0.02) continue;
        const ink = row.lineMask.inkBounds || {left: row.lineMask.textMin ?? 0, right: row.lineMask.textMax ?? 1, top: 0, bottom: 1};
        const width = row.lineWorldW, height = row.lineWorldH;
        for (let ix = 0; ix < 2; ix++) for (let iy = 0; iy < 2; iy++) {
          const x = ix ? ink.right : ink.left, y = iy ? ink.bottom : ink.top;
          point.set((x - 0.5) * width, (0.5 - y) * height, 0).applyMatrix4(row.mesh.matrixWorld).project(camera);
          if (!Number.isFinite(point.x) || !Number.isFinite(point.y) || !Number.isFinite(point.z) || point.z < -1 || point.z > 1) continue;
          left = Math.min(left, point.x); right = Math.max(right, point.x);
          bottom = Math.min(bottom, point.y); top = Math.max(top, point.y);
        }
      }
    });
    if (!Number.isFinite(left)) { scale = 1; offsetX = offsetY = 0; last = 0; return; }
    const marginX = Math.min(48, rect.width * 0.04);
    const marginY = Math.min(rect.height * 0.2, Math.min(42, Math.max(18, rect.height * 0.08)) + 8);
    const limitX = 1 - 2 * marginX / rect.width, limitY = 1 - 2 * marginY / rect.height;
    const wanted = Math.min(1, 2 * limitX / Math.max(0.001, right - left), 2 * limitY / Math.max(0.001, top - bottom));
    const ease = last ? 1 - Math.exp(-Math.min(0.1, Math.max(0, now - last) / 1000) * 8) : 1;
    last = now;
    // Shrink before a glyph crosses an edge; recover gently as outgoing rows leave.
    scale = Math.min(wanted, scale + (wanted - scale) * ease);
    const clamp = (value, lo, hi) => Math.max(lo, Math.min(hi, value));
    offsetX = clamp(offsetX * (1 - ease), -limitX - scale * left, limitX - scale * right);
    offsetY = clamp(offsetY * (1 - ease), -limitY - scale * bottom, limitY - scale * top);
    transform.set(scale, 0, 0, offsetX, 0, scale, 0, offsetY, 0, 0, 1, 0, 0, 0, 0, 1);
    camera.projectionMatrix.premultiply(transform);
  };
}
