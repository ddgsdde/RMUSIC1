# AirPlay 宿主适配器

基于固定版本 `pyatv==0.18.0`，在 Windows／Linux／macOS 宿主运行，负责局域网发现、RAOP／AirPlay 配对、播放、音量与状态。核心仍只通过同一 HTTPS／WSS 端口提供控制和媒体，局域网 AirPlay 通信由宿主负责。

```powershell
python -m venv plugins/airplay/.venv
plugins/airplay/.venv/Scripts/python.exe -m pip install -r plugins/airplay/requirements.txt
plugins/airplay/.venv/Scripts/python.exe plugins/airplay/adapter.py
```

通过 `RMUSIC_AIRPLAY_DATA_DIR` 指定持久凭据目录；默认 `plugins/airplay/data`（已排除 Git）。凭据以 0600 创建；Windows 宿主应对目录设置当前用户 ACL。`RMUSIC_FFMPEG` 可指定 FFmpeg 可执行文件。

stdin／stdout 使用 UTF-8 JSON Lines；每次 `{id,method,params}` 返回 `{id,result}` 或 `{id,error:{code,message}}`。日志仅输出 stderr，避免打印配对凭据和核心媒体票据。

| 方法 | 参数与结果 |
| --- | --- |
| `health` | 版本、FFmpeg 可用性及未做实机验收状态 |
| `discover` | timeout=1..30 秒；可选 hosts IPv4 数组用于定点发现。返回 items，包括 id、name、address、protocols、paired、pairing_required、password_required、capabilities |
| `pair_start` | device_id、protocol=raop 或 airplay；返回 session_id、pin_required、device_provides_pin |
| `pair_finish` | session_id、pin；验证配对完成后保存凭据，180 秒后会话失效 |
| `password` | device_id、password；设置需要密码的 RAOP 接收器，停止当前流并重建连接 |
| `play` | device_id、url、position?、title?、artist?、album?、zone_id?、duration?、transitions?；立即返回 buffering，后台发送音频 |
| `resume` | device_id；使用上次地址和暂停位置重开 |
| `seek` | device_id、position（秒）；从新位置重开，需要 FFmpeg |
| `pause`／`stop` | device_id；终止流，pause 保留位置，stop 归零 |
| `volume` | device_id、volume=0..1；仅在实际设备能力允许时设置 |
| `state` | device_id；返回 status、position、volume、ended、error?、zone_id? |

`state.status` 是 `buffering`／`playing`／`paused`／`stopped`／`error`。开始播放后异步失败会进入 error，不假报播放成功。自然结束返回 `stopped, ended=true`，主动停止／暂停 `ended=false`；网桥只有收到 ended 才通知核心下一首。音频被实际送入设备后，状态和位置来自 pyatv metadata。恢复或定位有重新缓冲延迟。

pyatv 自动协商 AirPlay 1／2 的接收器协议及 PCM 格式；输出可能重采样、改变位深，故 `bit_perfect=false`、`strict_sync=false`。不宣称精确多室同步。RAOP 原生 pause 在当前 pyatv 版本实现为 stop，所以恢复／seek 使用 FFmpeg 从保存位置生成无损 FLAC 管道再建立流，仍需实机检查定位和尾段播放。WAV 管道未通过当前版本的解码回归，改用已验证的 FLAC 管道，不增加有损编码。此适配器尚未接入共享 Rust DSP，enabled DSP 请求明确拒绝，`capabilities.dsp=false`。

宿主网桥提供经过核心证书验证的本地 HTTP 媒体代理，可避免把核心的不受信任证书设置传播到 pyatv 或接收器。适配器自身不全局关闭 TLS 校验。

```powershell
plugins/airplay/.venv/Scripts/python.exe -m unittest discover -s plugins/airplay -p test_adapter.py -v
```

测试包含 JSON 参数、能力边界、自然结束／主动暂停区别、异步错误，以及实际 FFmpeg 无损 FLAC 管道与 pyatv 解码兼容性。发现测试可只读执行；不自动向发现的设备播音。没有完成用户指定接收器的配对、音量、音频及长期稳定性验收，manifest 始终 `verified=false`。

参考：[pyatv 源码](https://github.com/postlund/pyatv)、[流播放接口与限制](https://pyatv.dev/development/stream/)。

淡入淡出由 FFmpeg afade 实现，需要 FFmpeg；无时长不推测曲尾。停止先取消本地流，再限时等待设备，接收器失联不会无限阻止本地取消。实机包络与定位仍需测试。
