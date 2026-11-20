# Mineradio visual layer

Source: https://github.com/XxHuberrr/Mineradio-paused
Revision: d43de565acabfdc1a9c9820a27e81a98ccbebcef
License: GPL-3.0, full text in LICENSE-Mineradio.txt.

R-Music changes (2026-10-25): isolated GPU-only host, bounded native feature input, demand-driven lifecycle, quality controls and independent native interface. No upstream playback, platform APIs, account/session logic, microphone, gesture camera or service code is included.

Particle shader/geometry, skull point asset and Sonic Topography visual algorithms retain their upstream attribution. Sonic Topography: yin-yizhen/sonic-topography 1.1.1, commit 3ff303e (see source header). Three.js r128: Copyright three.js authors, MIT (license header in three.r128.min.js).

Sonic Workshop: the upstream bundled visual-only artifact and bridge are retained under the upstream source distribution. R-Music adds a bounded frame scheduler, reduces default instance density and validates parent message origin; no audio player or network access is enabled. See the pinned upstream public/vendor/sonic-workshop directory for corresponding distributed artifact.

The camera, bounded GPU lyric scene, word-progress rendering, cover palette/depth helpers, floating layers and eleven-card queue shelf also come from this pinned revision. `stage/` retains selected declarations and the required visual initialization; account, networking, input-capture and startup hooks are excluded. `scripts/import-mineradio.cjs` regenerates these files from the pinned checkout. `console.json` and `stage/console-schema.js` are extracted original control labels, defaults and bounds for the native console. The SwiftUI chrome and lifecycle adapter are R-Music code. These resources are App-only; WebUI and Docker builds do not include them.

Native v2 port: the pinned visual shader algorithms are converted at development time through glslang/SPIRV-Cross to MSL. The renderer uses Metal directly; the upstream JavaScript/Three.js/Workshop player runtime is not executed. SwiftUI owns the existing interface and CoreText prepares bounded glyph masks. See macos/script/port_mineradio_shaders.py for conversion and the native-v2 parity checklist for remaining work.
