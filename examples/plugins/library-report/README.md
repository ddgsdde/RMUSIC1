# 曲库统计扩展示例

非内建音源，不会自动安装。将本目录内文件打包到 ZIP 根目录，使用管理员插件安装接口；无需 pip 依赖。安装后：

`POST /api/v1/extensions/library-report/invoke/summarize`，正文 `{}`。

只接收 Core 提供的有界曲库数量摘要，不接收 Core token、数据库路径、其他插件配置或音频。验证扩展 RPC／升级回退合同使用，无自动任务、无自动修改。
