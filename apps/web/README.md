# R-Music Web 界面

浏览器和 Tauri 客户端共用这套 React 界面。Core 在同一端口提供生产静态文件。开发服务器默认监听 `127.0.0.1:5173`，并将 `/api` 转发到本机 Core 的 `7800` 端口。

```sh
npm ci
npm run dev
npm run build
npm test
```

生产文件生成在 `dist/`。使用子路径反代时，访问地址末尾保留斜杠。离线缓存需要浏览器支持 Service Worker 和安全上下文。

[Core 部署方式](../../docs/deployment.md)
