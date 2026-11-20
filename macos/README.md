# macOS 客户端

支持 Apple Silicon 和 macOS 27+。从 [Releases](https://github.com/ddgsdde/RMUSIC1/releases) 下载 DMG，将 R-Music 拖到「应用程序」。安装 FFmpeg 和 FFprobe 后即可播放。

    brew install ffmpeg

预览 DMG 使用 ad-hoc 签名。首次打开时系统可能要求在「系统设置 → 隐私与安全性」确认。见 [Apple 公证和分发说明](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)。

在客户端选择自动发现的 Core 或手动输入地址，再登录。到「声音设备」启用想用的输出。USB DAC 的实际输出由设备、源格式和系统设置决定。

## 构建

需要支持 macOS 27 的 Swift 工具链与 Rust。开发包和发布包使用独立的 Bundle ID。

    cd macos
    ./script/build_and_run.sh --build-only RMusicNative
    RMUSIC_MAC_PUBLIC=1 ./script/build_and_run.sh --build-only RMusicNative

开发包生成于 dist/RMusicNative.app，发布包生成于 dist/R-Music.app。可通过 RMUSIC_MAC_DIST_DIR 指定独立输出目录。
