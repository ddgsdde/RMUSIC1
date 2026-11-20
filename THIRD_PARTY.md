# 第三方说明

- Mac 的 Mineradio 视觉组件保留[上游署名](macos/VisualAssets/Sources/RMusicVisualAssets/Mineradio/NOTICE.md)和 [GPL-3.0 许可证](macos/VisualAssets/Sources/RMusicVisualAssets/Mineradio/LICENSE-Mineradio.txt)。Three.js 另附 [MIT 许可证](macos/VisualAssets/Sources/RMusicVisualAssets/Mineradio/LICENSE-Three.txt)，Sonic Topography 的署名见对应源码。
- Web 主题保留 [Liser 署名](apps/web/public/third-party/liser-NOTICE.txt)。音频适配依赖的许可证见 [crates/audio/licenses](crates/audio/licenses)。
- [QQ 音乐插件](plugins/qqmusic)及其使用的 `qqmusic-api-python` 采用 GPL-3.0-or-later。
- Core 镜像使用 Debian 提供的 FFmpeg 和系统库。发行版版权文件保存在容器 `/usr/share/doc/`，源码包由 Debian 提供。Mac 客户端使用宿主安装的 FFmpeg 与 FFprobe。
