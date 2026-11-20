# Android 客户端

发布 APK 适用于 ARM64、Android 8.0 及以上。安装后输入 Core 地址并登录。局域网可发现 Core 或手动填写，外出使用可达的 HTTPS 地址。

主客户端是 `apps/desktop/src-tauri` 中的 Tauri Android 工程。`android/` 保存原生 UI 实验项目。

构建需要 Rust ARM64 Android target、JDK 21、Android SDK 36、Build Tools 36.0.0 和 NDK。设置 `ANDROID_HOME`、`NDK_HOME` 和 `JAVA_HOME`，将 PKCS#12 密钥路径及密码通过 `RMUSIC_ANDROID_KEYSTORE` 和 `RMUSIC_ANDROID_KEY_PASSWORD` 传入。

```sh
rustup target add aarch64-linux-android
./scripts/build-android-release.sh 0.1.0-preview.1 10001
```

公开 APK 与开发包使用不同应用 ID。升级同一发布包时需使用同一签名密钥。
