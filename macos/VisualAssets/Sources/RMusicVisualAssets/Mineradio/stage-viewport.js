/* R-Music: full-bleed background, with a separate safe viewport for the existing 3D lyrics. */
const rmusicLyricViewportFade = {value: new Float32Array(3)};
let rmusicFitLyricProjection;
function rmusicFadeLyricEdges(material) {
  if (!material || material.userData.rmusicViewportFade) return;
  material.userData.rmusicViewportFade = true;
  const compile = material.onBeforeCompile, cacheKey = material.customProgramCacheKey();
  material.onBeforeCompile = function(shader, renderer) {
    compile.call(this, shader, renderer);
    shader.uniforms.rmusicLyricViewportFade = rmusicLyricViewportFade;
    shader.fragmentShader = shader.fragmentShader
      .replace(/void\s+main\s*\(\s*\)\s*\{/, 'uniform vec3 rmusicLyricViewportFade;\n$&')
      .replace(/\}\s*$/, `
        gl_FragColor.a *= smoothstep(rmusicLyricViewportFade.x, rmusicLyricViewportFade.x + rmusicLyricViewportFade.z, gl_FragCoord.y)
          * (1.0 - smoothstep(rmusicLyricViewportFade.y - rmusicLyricViewportFade.z, rmusicLyricViewportFade.y, gl_FragCoord.y));
      }`);
  };
  material.customProgramCacheKey = () => cacheKey + ':rmusic-lyric-viewport-fade-v1';
  material.needsUpdate = true;
}
function rmusicStageViewport(width, height, top, bottom) {
  width = Math.max(1, Number(width) || 1); height = Math.max(1, Number(height) || 1);
  top = Math.max(0, Math.min(height * 0.4, Number(top) || 0));
  bottom = Math.max(0, Math.min(height * 0.4, Number(bottom) || 0));
  return {x: 0, y: bottom, width, height: height - top - bottom, fullHeight: height};
}
function rmusicRenderStage(renderer, scene, camera, lyrics, rect) {
  if (!lyrics || !lyrics.visible) { renderer.render(scene, camera); return; }
  const dpr = renderer.getPixelRatio ? renderer.getPixelRatio() : 1;
  rmusicLyricViewportFade.value.set([rect.y * dpr, (rect.y + rect.height) * dpr, Math.min(42, Math.max(18, rect.height * 0.08)) * dpr]);
  lyrics.traverse(node => {
    node.layers.set(1);
    for (const material of Array.isArray(node.material) ? node.material : [node.material]) rmusicFadeLyricEdges(material);
  });
  const mask = camera.layers.mask, aspect = camera.aspect, autoClear = renderer.autoClear;
  try {
    camera.layers.set(0);
    renderer.render(scene, camera);
    renderer.autoClear = false;
    renderer.clearDepth();
    camera.layers.set(1); camera.aspect = rect.width / rect.height; camera.updateProjectionMatrix();
    if (typeof rmusicLyricViewportFit === 'function') {
      rmusicFitLyricProjection ||= rmusicLyricViewportFit();
      rmusicFitLyricProjection(camera, lyrics, rect);
    }
    renderer.setViewport(rect.x, rect.y, rect.width, rect.height);
    renderer.setScissor(rect.x, rect.y, rect.width, rect.height); renderer.setScissorTest(true);
    renderer.render(scene, camera);
  } finally {
    camera.layers.mask = mask; camera.aspect = aspect; camera.updateProjectionMatrix();
    renderer.autoClear = autoClear; renderer.setScissorTest(false);
    renderer.setViewport(0, 0, rect.width, rect.fullHeight);
  }
}
