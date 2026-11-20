# R-Music 图标：Track / Play

产品识别采用 R 字形，字腔用向右的播放三角形成负形，右下笔画保持同一方向。石墨黑底板与青柠色标志延续播放器配色；标志无纹理和立体挤出，轮廓独立于材质，在 Dock 小尺寸中仍可识别。底板保留轻微明暗和边缘，以适应浅色、深色桌面。

`RMusicIcon.svg` 是可编辑主稿；`RMusicIcon.png` 为 1024px 预览；`RMusic.icns` 包含标准 16／32／128／256／512px 及各自 2x 分辨率，外侧透明。使用 `bash macos/script/build-app-icon.sh` 从同一矢量稿生成，需 librsvg 的 `rsvg-convert` 与系统 `iconutil`。构建脚本将 ICNS 写入应用资源及 CFBundleIconFile。

运行 `python3 scripts/sync-brand-icons.py`（需 Pillow／librsvg）同步网页与其他桌面端导出；Android 的 adaptive／round／monochrome 图标按用户要求留到下一次安卓整改，本脚本不触碰其 res 目录。Mac 启动时同时设置运行中的 applicationIconImage，不能只凭打包资源判断 Dock 验收通过。

早期两版内置 imagegen 探索稿（立体音符 R、写实唱片）均因用户反馈弃用，没有进入应用。最终稿由代码绘制矢量轮廓，保留源文件，未使用这些生成位图作为应用图标。
