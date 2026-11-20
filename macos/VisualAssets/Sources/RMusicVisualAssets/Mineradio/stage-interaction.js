/* R-Music: view-local mouse orbit; no timer, pointer lock or effect on audio. */
function rmusicStageInteraction(target, options) {
  let drag = null, suppressClick = false;
  const listeners = [];
  const on = (name, fn, config) => { target.addEventListener(name, fn, config); listeners.push([name, fn, config]); };
  const finish = () => { drag = null; options.end(); };
  on('pointerdown', event => {
    if (event.button !== 0 || !options.enabled()) return;
    drag = {id: event.pointerId, x: event.clientX, y: event.clientY, distance: 0}; suppressClick = false;
    event.target?.setPointerCapture?.(event.pointerId); options.begin();
  });
  on('pointermove', event => {
    if (!drag || drag.id !== event.pointerId || !options.enabled()) return;
    const dx = event.clientX - drag.x, dy = event.clientY - drag.y;
    drag.x = event.clientX; drag.y = event.clientY; drag.distance += Math.hypot(dx, dy);
    if (drag.distance > 4) { suppressClick = true; options.move(dx, dy); }
  });
  on('pointerup', finish); on('pointercancel', finish); on('blur', finish);
  on('click', event => { if (suppressClick) { event.stopImmediatePropagation(); suppressClick = false; } }, true);
  on('dblclick', () => { if (options.enabled()) options.reset(); });
  on('wheel', event => {
    if (!options.enabled() || !options.zoom) return;
    const units = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 400 : 1;
    const delta = Math.max(-400, Math.min(400, (Number(event.deltaY) || 0) * units));
    if (delta && options.zoom(delta, event) !== false) event.preventDefault();
  }, {passive: false});
  return () => { finish(); for (const [name, fn, config] of listeners) target.removeEventListener(name, fn, config); };
}
