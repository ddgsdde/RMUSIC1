# QQ 音乐音源

Core 通过独立 Python 进程接入 QQ 音乐，使用 JSON Lines RPC。官方预览镜像内置此插件。登录由 Core 中的音源设置发起，支持 QQ 和微信扫码。

能力包括搜索、专辑与歌手目录、播放音源解析、同步歌单、逐行和逐字歌词。实际可播放的音质取决于平台授权。支持复用平台随歌曲提供的增益，相关设置由 Core 管理。

插件依赖版本见 `requirements.txt`，能力和当前版本见 `manifest.json`。凭据保存在各使用者自己的 Core 数据目录中。

本插件与上游 `qqmusic-api-python` 采用 GPL-3.0-or-later。见 [许可证](LICENSE)和 [上游项目](https://github.com/L-1124/QQMusicApi)。
