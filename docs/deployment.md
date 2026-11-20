# 部署 Core

需要 Docker Engine 和 Docker Compose v2。到 [Releases](https://github.com/ddgsdde/RMUSIC1/releases) 按宿主架构下载 `linux-amd64`（Intel / AMD）或 `linux-arm64` 镜像。两个归档内的默认标签均为 `rmusic-core:0.1.0-preview.1`。

## Docker Compose

```sh
git clone https://github.com/ddgsdde/RMUSIC1.git
cd RMUSIC1
./scripts/install-core.sh --image /path/to/rmusic-core-linux-amd64.tar.gz
```

浏览器访问 [127.0.0.1:7800](http://127.0.0.1:7800)。首次设置时在自己的终端读取初始化凭证，再在 Web 界面创建管理员账号。

```sh
docker compose exec -T core cat /data/bootstrap.token
```

默认将仓库的 `music/` 只读挂载到 `/music`。修改 `.env` 的 `RMUSIC_MUSIC_DIR` 可以指向现有音乐目录。在 Core 添加容器内路径 `/music` 并扫描。数据库、在线缓存和备份保存在独立 Docker 卷。需要自行构建时运行 `./scripts/install-core.sh --build`。

## Linux NAS

```sh
cp .env.nas.example .env.nas
# 编辑 .env.nas，写入 NAS 实际局域网地址及音乐目录
./scripts/install-core.sh --nas --image /path/to/rmusic-core-linux-amd64.tar.gz
docker compose --env-file .env.nas -f compose.nas.yaml exec -T core cat /data/bootstrap.token
```

NAS 配置使用 host 网络支持局域网发现。也可以在 NAS 容器管理器导入镜像归档，再以 `compose.nas.yaml` 创建项目。远程访问可使用 Core 的 HTTPS 或自有 HTTPS 反向代理。局域网客户端可手动输入 NAS 地址。

## 更新与回退

先在 Core 设置创建备份。导入新镜像后更新 `RMUSIC_IMAGE`，再执行安装脚本。Compose 项目与数据卷使用原名称。回退时选择已保留的旧镜像，数据库需要回退时使用对应版本的备份。
