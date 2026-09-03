# TBuildTool · 独立构建插件工具

TBuildTool 是独立于游戏工程的**打包构建插件工具**，把「工程内辅助插件」与「外部打包工具」
统一收编到一个目录，供任意 Unity（6.x）项目复用。

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
| 工程内辅助插件 | `TBuildTool/unity/` | 命令行打包入口（`-executeMethod`）、`IBuildProgress` 构建进度钩子扫描/执行 | Unity 编辑器 |
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