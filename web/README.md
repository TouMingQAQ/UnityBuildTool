# Legacy Node 用户 Web 参考服务

> 当前正式管理端是 [apps/admin/desktop](<../apps/admin/desktop/>) 中的 egui 原生程序。本目录的 Node 服务只用于用户 Web 页面和用户 API 的本地参考/回归测试，不参与安装包构建，也不由桌面程序启动。

## 运行

需要 Node.js 20+：

```powershell
npm run legacy:server
```

或：

```powershell
node web/server.js
```

默认地址为 `http://127.0.0.1:8787`。可通过环境变量指定端口和配置：

- `PORT`：监听端口。
- `NO_OPEN=1`：启动时不自动打开浏览器。
- `CONFIG_PATH`：legacy 单项目配置文件路径。

## 保留的路由

- `/`、`/workspace`
- `/user.js`、`/user.css`
- `/api/user/catalog`
- `/api/user/status`
- `/api/user/events`
- `/api/user/build`
- `/api/user/build/stop`
- `/api/user/files`
- `/api/user/download`
- `/api/user/download-folder`

用户页面直接复用 `apps/user/public`，SSE 只推送脱敏后的构建生命周期和进度，不暴露命令、签名信息、本机路径或原始 Unity 日志。

以下旧管理入口和静态资源均返回 404，不提供管理令牌、Cookie 或浏览器管理替代入口：

```text
/admin/*
/api/config
/api/status
/api/monitor
/api/build/*
/api/check/*
/api/vcs/*
/api/open-user-page
/api/open-folder
```

## 归档过滤

legacy Node 服务与正式 Rust 后端都不得把工具日志或诊断信息打进构建压缩包。至少应排除：

- 任意 `.log`、`.tmp`、`.temp`、`.bak`、崩溃转储
- `.buildtool`、`check-results`、`logs`
- `build-result.json`
- 重复的 APK/AAB/ZIP 产物
- Unity `DoNotShip` / `BackupThisFolder...` 调试目录

完整日志只保留在节点输出目录中，不随构建产物分发。

## 测试

```powershell
npm run check
npm run test:legacy
```

legacy 服务测试确认：用户主页、工作台、静态资源和用户 API 可用；所有管理页面与管理 API 返回 404；AI 密钥等私有配置不会出现在用户响应中。
