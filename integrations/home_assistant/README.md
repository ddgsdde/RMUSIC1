# R-Music Home Assistant 集成

每个核心音区对应一个 `media_player`。支持状态推送、断线重连、播放／暂停／停止、上一首／下一首、定位、音量／静音、重复／随机、曲库分页浏览、队列追加／插入下一首／立即播放／替换／清空。不会宣称尚未验证的同步分组或广播能力。

## 本地安装

1. 在 R-Music「设置 → API 凭证」创建仅包含 **read、control** 的凭证。
2. 把 `custom_components/rmusic` 复制到 HA 配置目录的 `custom_components/rmusic`；重启 HA。
3. 「设置 → 设备与服务 → 添加集成 → R-Music」，填写核心 URL 和凭证。例：`http://192.168.1.20:7800`、`https://music.example.net/music`。
4. 证书验证默认开启；可仅对此核心关闭验证。凭证撤销后 HA 发起重新认证。

```powershell
python integrations/home_assistant/package.py --output artifacts
```

`rmusic-home-assistant.zip` 可解压到 HA 配置根目录。`rmusic-hacs-repository.zip` 包含独立仓库根目录结构（`hacs.json`、`custom_components`、README）；后续发布为用户自己的 GitHub 集成仓库后，才能在 HACS 添加自定义仓库。本地代码没有擅自发布或创建云端仓库。

## 服务示例

```yaml
action: media_player.play_media
target:
  entity_id: media_player.r_music_living_room
data:
  media_content_type: music
  media_content_id: "曲库中实际的 Track ID"
  enqueue: replace
```

`enqueue` 支持 `add`、`next`、`play` 和 `replace`。曲库浏览按 100 首分页，避免百万首曲库一次加载；播放队列浏览最多显示前 1000 首。更多查询通过核心公开 API 完成。HA 只控制音区，实际声音由该音区选定的客户端或网桥播放。

## 验证范围

`python -m unittest discover -s integrations/home_assistant/tests -v` 检查真实本地 HTTP／WebSocket 通信、路径前缀、Token 失效、队列参数及封面鉴权。源码通过 Python 编译检查。完整 HA UI 配置流／实体生命周期、实际 HA 主机和实际音区联调仍需验收；这些测试不能替代设备测试。

证书例外只作用于配置的核心；外部专辑封面始终使用正常 HTTPS 校验，API 凭证不会发往外部封面服务。停止／卸载集成会取消事件订阅；断线立即标记不可用，并以最大 60 秒退避重连，30 秒轮询作为状态修复。

接口依据：[HA media_player](https://developers.home-assistant.io/docs/core/entity/media-player/)、[配置流](https://developers.home-assistant.io/docs/core/integration/config_flow/)。最低目标 HA Core 2025.12，后续版本应执行完整集成回归。
