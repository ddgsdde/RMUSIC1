# Android Compose 原型

此目录是 Kotlin 和 Jetpack Compose 客户端原型。`core-client` 负责 Core 连接、认证和事件，`media` 负责 Android 播放与输出，`app` 提供界面。

使用 Java 21 和 Android SDK 36 从本目录运行 `./script/build.sh`。调试 APK 位于 `app/build/outputs/apk/debug/`。

当前发布的 Android 客户端由 Tauri 工程构建。见 [Android 安装与构建](../docs/android.md)。
