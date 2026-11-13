# UnityBuildTool · Unity 构建平台

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

安装包位于 `apps/admin/src-tauri/target/release/bundle/nsis/`。配置保存至系统应用数据目录 `com.unitybuildtool.admin/config.json`，开发模式读取 `web/config.json`。

### Android 工具目录

「引擎与高级配置」页提供项目级的 **Android 工具目录（SDK / NDK / JDK）**：点击「自动扫描」会依次读取系统环境变量（`ANDROID_SDK_ROOT` / `ANDROID_NDK_ROOT` / `JAVA_HOME` 等）、Unity 编辑器偏好（注册表中的 Unity EditorPrefs）以及 Unity.exe 安装目录下的 `PlaybackEngines/AndroidPlayer`，三个路径均可直接手动修改。保存后构建时会作为环境变量注入 Unity 子进程。

### 自动构建号

「安卓构建号」填 `-1` 时自动 **+1 Unity 工程里的构建号**：Android 目标会读取对应 Build Profile 资产（无则读取 `ProjectSettings/ProjectSettings.asset`）中的 `AndroidBundleVersionCode` 并加一，构建成功后写回该工程文件；其它平台继续使用工具内的 `lastVersionCode` 计数。构建失败不递增。该值同时通过 `-versionCode` 传给 Unity 插件设置 `bundleVersionCode`。

### 旧版 Unity（无 Build Profile）

若扫描不到任何 Build Profile（Unity 2023 及更早），会自动读取工程自身配置生成一个「项目默认（跟随工程）」节点：产品名/版本取自 `ProjectSettings.asset`，场景数取自 `EditorBuildSettings.asset`。构建时不再传 `-profilePath`，由 Unity 插件使用经典 `BuildPipeline.BuildPlayer` 按工程当前激活目标与 Build Settings 场景列表构建，并把实际目标回传给工具用于归档命名。

## 用户端

从托盘菜单选择“打开用户网页”，或在浏览器打开 Rust 服务显示的本机地址。网页首页保留星空、渐变、轨道与立方体动效，点击进入工作台。当前无用户登录流程。

1. 管理员先配置 Unity 工程、构建引擎、构建节点与分组。
2. 用户选择节点或分组，点击开始构建，按顺序执行管理员配置的任务。
3. 工作台显示任务进度与阶段，可停止当前构建；详细 Unity 和仓库日志在管理端查看。
4. 文件浏览器仅显示管理员指定的构建目录和节点自定义构建目录，可以下载文件或文件夹 ZIP，无上传功能。

Rust 服务默认监听 `127.0.0.1:50345`，使用固定端口保持项目链接稳定。可在设置中修改端口，重启后生效；端口被占用会报告错误。退出 Tauri 管理端后网页服务随之停止。

## 多项目

在管理端“设置”中创建项目，分别配置项目名称、链接标识、Unity 工程、仓库、构建节点和输出目录。主界面选择项目后展示该项目的状态和用户网页链接，例如：

```text
http://127.0.0.1:50345/p/client
http://127.0.0.1:50345/p/server
```

每个链接有独立首页与 `/workspace` 工作台。显示名称可以修改；链接标识修改后需重新分发链接。各项目独立保存构建号与任务状态，用户接口仅允许选择当前项目的节点，并只能下载当前项目配置的产物。不同项目提交的构建进入统一串行队列，等待中的任务可以取消。

旧的单项目配置会迁移为默认项目；迁移前保留配置备份。删除项目配置不删除工程与产物文件。默认链接仅供本机访问。

首页的项目详情和“设置 → 项目管理”提供快捷打开目录：工程根目录、Assets、ProjectSettings、Packages、Profile、构建输出、成功归档、环境检测结果，以及已配置的仓库与自定义构建目录。完整路径可选中复制；未生成的目录会标记并禁用打开按钮，生成后点击“刷新路径”即可。

## 验证

```powershell
npm run check
npm test
npm run check:rust
npm run test:legacy
```

实际 Unity 构建需要有效工程、对应 Unity 版本和平台模块。
