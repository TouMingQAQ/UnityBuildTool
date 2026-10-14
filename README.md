# TBuildTool · Unity 构建平台

项目由 **Tauri/Rust 本地后端**、**Tauri 管理端网页**和**用户网页前端**组成。用户网页保留原来的主页与工作台；配置、仓库、Unity、构建调度、状态和文件下载由 Tauri 内部的 Rust 服务处理。

| 部分 | 位置 | 职责 |
| --- | --- | --- |
| Rust 后端 | [apps/admin/src-tauri](<apps/admin/src-tauri/>) | 配置读写、仓库/Git/SVN、Unity 探测、构建调度、状态事件、目录浏览和下载 |
| 管理端 | [apps/admin](<apps/admin/>) | 管理仓库、Unity、Build Profile、签名、构建节点、检测和归档配置 |
| 用户端 | [apps/user/public](<apps/user/public/>) | 展示主页、选择构建、查看进度、浏览构建目录、下载文件和目录 ZIP |
| Unity 插件 | [unity](<unity/>) | 命令行构建、环境检测和构建进度钩子 |

## 启动管理端

需要 Rust、Windows C++ 构建工具及 WebView2。运行时不依赖 Node.js；Node 只用于前端静态文件检查和 Tauri CLI 开发脚本。

```powershell
npm --prefix apps/admin install
npm run app:dev
```

管理窗口默认 **800 × 600**。主界面显示构建控制台、当前进度和用户网页链接；仓库、构建节点、Unity、签名及高级配置统一在“设置”弹窗中管理。用户网页链接支持在浏览器打开和复制。关闭窗口后服务继续运行于托盘；单击托盘显示/隐藏窗口，右键菜单可以打开用户主页或退出。退出会终止 Rust 后端和构建子进程。

生成 Windows 安装包：

```powershell
npm run build:admin
```

安装包位于 `apps/admin/src-tauri/target/release/bundle/nsis/`。配置保存至系统应用数据目录 `com.tbuildtool.admin/config.json`，开发模式读取 `web/config.json`。

## 用户端

从托盘菜单选择“打开用户网页”，或在浏览器打开 Rust 服务显示的本机地址。网页首页保留星空、渐变、轨道与立方体动效，点击进入工作台。当前无用户登录流程。

1. 管理员先配置 Unity 工程、构建引擎、构建节点与分组。
2. 用户选择节点或分组，点击开始构建，按顺序执行管理员配置的任务。
3. 工作台显示任务进度与阶段，可停止当前构建；详细 Unity 和仓库日志在管理端查看。
4. 文件浏览器仅显示管理员指定的构建目录和节点自定义构建目录，可以下载文件或文件夹 ZIP，无上传功能。

Rust 服务默认仅监听 `127.0.0.1`，自动分配空闲端口。退出 Tauri 管理端后网页服务随之停止。

## 验证

```powershell
npm run check
npm test
npm run check:rust
npm run test:legacy
```

实际 Unity 构建需要有效工程、对应 Unity 版本和平台模块。
