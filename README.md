# TBuildTool · 独立构建插件工具

TBuildTool 是独立于游戏工程的**打包构建插件工具**，把「工程内辅助插件」与「外部打包工具」
统一收编到一个目录，供任意 Unity（6.x）项目复用。本仓库（MiSide 壁纸改造工程）内置了本工具。

```
TBuildTool/
  README.md                 # 本文件：工具总览
  unity/                    # ① 工程内辅助插件（Unity Editor 插件，安装进 Assets/）
    package.json            #   UPM 包元数据（可选 UPM 安装）
    README.md               #   插件说明 + 安装方式
    Editor/
      BuildCommand.cs                  # 命令行打包入口（-executeMethod，驱动 IBuildProgress 钩子）
      CompileCheck.cs                  # 环境编译检测入口（-executeMethod：切换目标平台→触发脚本编译→检查错误）
      BuildProgress.cs                 # IBuildProgress 构建进度钩子接口 + 上下文
      WallpaperResourceStripper.cs     # 壁纸构建资源裁剪托管（IBuildProgress 实现）
  web/                      # ② 外部打包工具（本地 Web 批量打包器，Node 18+）
    server.js               #   零依赖 Node 后端（HTTP + SSE + 命令执行）
    public/                 #   前端（index.html / app.js / style.css）
    start.bat               #   一键启动
    config.json             #   运行后生成，保存界面配置（不含密码）
    README.md               #   外部打包工具使用说明
```

## 两个组成部分

| 部分 | 目录 | 职责 | 运行环境 |
|---|---|---|---|
| 工程内辅助插件 | `TBuildTool/unity/` | 命令行打包入口（`-executeMethod`）、`IBuildProgress` 构建进度钩子扫描/执行、构建资源裁剪托管 | Unity 编辑器 |
| 外部打包工具 | `TBuildTool/web/` | 网页界面：扫描 Build Profile → 排队 → 批量调用 Unity 命令行打包 → SSE 实时日志 / 产物归档 | Node.js 18+ |

两者的对接点：外部打包工具以
`Unity.exe -batchmode -executeMethod TBuildTool.Editor.BuildCommand.Build ...` 调用
工程内插件的打包入口；插件按项目相对路径加载 Build Profile 完成构建，成功退出码 0 / 失败 1。
「环境编译检测」则调用 `TBuildTool.Editor.CompileCheck.Run`（每条环境线一个进程，见上方版本记录）。

## 安装

1. **工程内辅助插件**：把 `TBuildTool/unity/` 链接（Windows 目录联接）或拷贝到项目
   `Assets/TBuildTool/`（详见 `TBuildTool/unity/README.md`）。
   本仓库已通过目录联接安装于 `Assets/TBuildTool`。
2. **外部打包工具**：安装 Node.js 18+，运行 `TBuildTool/web/start.bat`（或
   `node TBuildTool/web/server.js`），浏览器自动打开 http://127.0.0.1:8787 。

## 版本记录（相对原 BuildWeb 的变更）

- 目录 `BuildWeb/` 更名为 `TBuildTool/`（外部打包工具移至 `TBuildTool/web/`）；
- 工程内辅助插件自 `Assets/MisideWallpaper/Editor/` 独立为 `TBuildTool/unity/`
  （本工程经 `Assets/TBuildTool` 目录联接挂回）；
- 命名空间 `MisideWallpaper.Editor` → `TBuildTool.Editor`；
  `WallpaperBuildCommand` → `BuildCommand`、`WallpaperBuildResourceStripper` → `WallpaperResourceStripper`；
  `-executeMethod` 入口变为 `TBuildTool.Editor.BuildCommand.Build`；
- 菜单路径 `Tools/MisideWallpaper/...` → `Tools/TBuildTool/...`；
- 资源裁剪改为 `IBuildProgress` 钩子：`BuildCommand` 构建前扫描 Editor 下接入该接口的类，
  按时期执行 `OnBeginBuild` / `OnCancelled` / `OnFinishedBuild`（取代 Unity 全局构建回调，手动构建不再自动裁剪）；
- **新增「环境编译检测」**：网页工具新增独立 Tab「环境编译检测」，对用户指定的多条环境线
  （Win x64/x86 / Android / iOS / macOS）逐条以独立 Unity 批处理进程执行真实编译检测；
  后端新增 `/api/check/start`、`/api/check/stop`、`/api/check/preview` 与 SSE 事件
  （`check-start` / `check-target-start` / `check-line` / `check-result` / `check-end`）；
  工程内插件新增入口 `TBuildTool.Editor.CompileCheck.Run`（`-target` / `-resultFile` / `-timeout` / `-logFile`，
  注意**不要传 `-quit`**，由插件在编译结束后以退出码收尾：0=通过/不支持，1=失败，2=超时/异常）；
  检测结果按环境线输出到 `TBuildTool/web/check-results/check-<时间戳>/`（每条环境线一个 JSON + log + 汇总 summary.json）；
- 其余行为不变：仍由 Build Profile 驱动、不触碰全局 `EditorBuildSettings`、
  构建前后自动隐藏/还原游戏本体资源（壁纸约束不变）。