# TBuildTool · 外部打包工具（本地 Web 批量打包器）

TBuildTool 的**外部打包工具**部分：通过网页界面批量打包 Unity 项目
（Unity 6，仅 Win / Android），底层调用命令行完成构建：
每个 Profile 依次执行 `Unity.exe -batchmode -executeMethod`（或自定义 CLI 命令模板），
日志通过 SSE 实时推送到网页。

> 本工具调用工程内辅助插件（`TBuildTool/unity`）的打包入口
> `TBuildTool.Editor.BuildCommand.Build`；两者配套使用，详见 `TBuildTool/README.md`。

## 输出与产物归档（v2）

- 每个 Profile 一个工作目录：`<输出目录>/<Profile名>/`（如 `Builds/WallpaperAndroid/`），
  重复构建直接覆盖，目录里保留 Unity 产物与 `build.log`。
- 构建成功后，包体会被单独**挪到成功归档目录** `Builds/构建成功/<Profile名>-<yyyyMMdd_HHmmss>/`：
  - 安卓：`MiSideWallpaper_v<版本>_b<构建号>_<时间戳>.apk`
  - Windows：整个构建目录（exe + `_Data`）**自动压缩为 zip** `MiSideWallpaper_v<版本>_b<构建号>_<时间戳>.zip`
- 每个归档目录附带 `build-info.txt`（Profile / 目标 / 包版本 / 构建号 / 日期 / 输出 / 大小 / 耗时），
  Windows 的 zip 内也会打包一份。
- 右侧「构建产物」列表**扫描输出目录实时生成**（不再依赖本地缓存），可随时点「刷新」；
  点击条目打开所在文件夹（文件已被挪走时自动打开父目录）。
- 输出目录（<输出目录>）与成功归档目录（默认 `构建成功`，相对输出目录）可在 ③ 引擎设置中修改。

## 快速开始

1. 确认本机已安装 **Node.js 18+**（本机已有 v24）。
2. 双击 `start.bat`，或命令行执行 `node server.js`（在本目录下）。
3. 浏览器自动打开 http://127.0.0.1:8787 （也可手动访问）。
4. 在页面中：
   - ① 填写 Unity 项目路径（支持绝对路径；相对路径以**本工具目录 TBuildTool\web** 为基准，`..\..` 即本项目根目录），点击「扫描 Profile」；
   - ② 把扫描出的 Profile 逐个「添加」进队列，用 ↑↓ 排定打包顺序（仅显示 Win / Android 目标，其余目标自动忽略）；
   - ③ 选择打包引擎：
     - **Unity 批处理模式（默认）**：确认 Unity.exe 路径（可「自动探测」或「用 unity CLI 探测」）；
     - **自定义命令模板**：填写 CLI 路径与命令模板（占位符见页面）；
     - **安卓签名区（可选）**：keystore 文件路径（相对项目根目录，默认 `KeyStore/user.keystore`）、
       key 别名（默认 `key`）、keystore 密码、key 密码（留空=同 keystore 密码）。
       扫描 Profile 后会自动用安卓 Profile 里的签名配置预填文件路径与别名；
       密码每次构建手动输入、不写入配置文件。留空则沿用 Profile 原有签名配置。
     - 输出目录默认 `<项目>\Builds`，可改；
   - ④ 「预览命令」确认 →「开始批量打包」→ 实时查看每个 Profile 状态与日志，「停止」可中断。

## 环境编译检测（独立 Tab）

新增「**环境编译检测**」页：对用户指定的多条**环境线**（Win x64 / Win x86 / Android / iOS / macOS，
Build Target 5 / 19 / 13 / 9 / 4）逐条执行一次真实编译检测。

- 流程：勾选环境线（可多选）→ 配置项目路径 / Unity.exe / 单环境线超时 → 「开始检测」。
  每条环境线以独立的 `Unity.exe -batchmode` 进程执行
  `TBuildTool.Editor.CompileCheck.Run`：**校验平台支持性 → 切换目标平台 → 触发脚本重编译 → 检查编译错误**。
- 结果：支持性不通过的环境线标记「不支持」（如 Windows 主机上检测 iOS/macOS，属正常判定，不计失败）；
  出现编译错误的环境线标记「失败」并摘录错误行；另有「超时」「异常」「已取消」状态。
- 每条环境线产出独立结果文件与日志：`check-results/check-<时间戳>/target-<N>.json` / `target-<N>.log`，
  全部结束后写入 `summary.json`（可通过「打开结果目录」查看）。
- 后端 API：`POST /api/check/start`、`POST /api/check/stop`、`POST /api/check/preview`；
  SSE 事件：`check-start` / `check-target-start` / `check-line` / `check-result` / `check-end` / `check-hello`。
- 环境检测与批量构建互斥（任一进行中，另一项启动会被拒绝）。

## 版本管理（独立 Tab）

新增「**版本管理**」页：把本地 SVN / Git 工作副本做成「节点」，按「分组」管理，支持一键更新。

- **节点**：一个节点 = 一条本地路径 + 版本控制类型。点击「添加节点」填写名称与路径
  （**默认路径为空**）；类型可选「自动检测（推荐）/ Git / SVN」。填好路径后点「检测类型」，
  会自动检测当前指定路径是 Git 还是 SVN（向上查找 `.git` / `.svn`）、列出**分支下拉列表**
  （Git：本地 + 远端跟踪分支；SVN：`branches/` 下分支 + trunk）；每个节点还可填写
  **自定义还原指令**（留空 = 默认普通还原：Git `git reset --hard` / SVN `svn revert -R .`，
  可写组合命令如 `git reset --hard && git clean -fdx`）。探测同时显示当前分支 / 版本号、
  未提交更改数、指定分支是否一致、远端是否存在该分支，以及**远端服务器状态**
  （Git 无远端服务器也会明确提示）。
- **分组**：把节点卡片**拖动**到分组卡片上即可合并为组（拖回「未分组节点」区域可移出分组）；
  分组支持重命名、删除（节点保留、回到未分组）与「更新分组」一键更新。
  「更新全部节点」串行更新所有节点。
- **更新流程**：任何版本管理更新前，都会先把该路径下的**未提交内容还原**——
  优先执行节点配置的**自定义还原指令**（弹窗内提供一键预设）；未配置则用默认普通还原
  （Git：`git reset --hard`，失败自动回退 `git checkout -- .`、无提交记录时自动跳过；
  SVN：`svn revert -R .`；未跟踪 / 未版本化的新增文件默认不删除）。
  Git 三种未提交状态与还原手段：**未暂存**（工作区改动未 add）→ `git checkout -- .`；
  **已暂存**（已 add）→ `git reset --hard`（连未暂存一起还原）；**未跟踪**（新建文件）→ `git clean -fd`。
  需要"还原未暂存但不丢已暂存/未跟踪"可用预设「只还原未暂存」（`git checkout -- .`）；
  需要连新建文件一起清可用「还原并清理未跟踪」（`git reset --hard && git clean -fd`）。
  随后检测**远端服务器**：**不在线或没有远端服务器 → 只执行还原，不执行 pull / update**
  （节点标记为「仅还原」）；在线且有远端时才执行 `git pull` / `svn update`，
  并先按指定分支切换（`git checkout <分支>` / `svn switch <分支URL>`）。
  **路径为空或未检测到 Git / SVN 工作副本的节点直接跳过，不执行任何更新操作**。
- 更新过程经 SSE 实时推送日志与节点状态（成功 / 仅还原 / 失败 / 跳过 / 已取消），可随时「停止」；
  结束后自动刷新各节点最新状态。与批量构建、环境编译检测互斥运行。
- **打包前自动更新（构建钩子）**：在「引擎与高级配置」页可勾选「打包前版本管理自动更新」并多选更新分组；
  「开始批量打包」时会先按「还原未提交 → 远端在线才 pull / update」逐个更新选定分组的节点，
  **全部成功后才开始打包**；任一节点失败或取消将终止打包（预览命令里会显示该步骤与涉及节点）。
- 后端 API：`POST /api/vcs/save`、`POST /api/vcs/probe`、`POST /api/vcs/update`、`POST /api/vcs/stop`、
  `GET /api/vcs/state`；SSE 事件：`vcs-hello / vcs-start / vcs-node-start / vcs-line / vcs-node-end / vcs-end`
  （打包前更新时 `vcs-*` 事件带 `preBuild=true`）。
- 节点与分组配置保存在 `config.json` 的 `vcs` 字段（重启后自动恢复）。

## 两种打包引擎

| 引擎 | 说明 |
|---|---|
| Unity 批处理模式 | `Unity.exe -batchmode -nographics -quit -projectPath ... -executeMethod TBuildTool.Editor.BuildCommand.Build -profilePath <Profile> -outputPath <输出> -logFile <日志>`。稳定、无头、可靠；安卓构建自动注入 keystore 密码。 |
| 自定义命令模板 | 模板内可用占位符：`{cli}` `{project}` `{profilePath}` `{profileAbs}` `{profileName}` `{target}` `{output}` `{logFile}` `{keystorePass}` `{keyaliasPass}` `{keystoreName}` `{keyaliasName}` `{unityExe}`。适合接入其他打包 CLI（如官方 Unity CLI 后续版本的 build 命令）。 |

## 安卓签名（keystore）

- 项目签名文件：`KeyStore/user.keystore`（已在仓库中），alias=`key`，包名 `com.miside.wallpaper`。
- 打包器「③ 引擎设置 → 安卓签名」可覆盖签名配置：
  - **keystore 文件**：相对项目根目录（如 `KeyStore/user.keystore`）或绝对路径；支持 Unity 的
    `{inproject}: KeyStore/user.keystore` 写法（自动剥离 `{inproject}:` 前缀）。
  - **key 别名**：默认 `key`。
  - **keystore 密码 / key 密码**：仅内存、每次构建输入；key 密码留空则等同 keystore 密码
    （Unity 要求二者一致，keytool 未单独指定 `-keypass` 时本就相同）。
- 文件路径与别名会随界面配置保存到 `config.json`（密码永不落盘）。
- 若签名区留空，构建时沿用 Build Profile 里的签名配置（WallpaperAndroid Profile 已内置
  `androidUseCustomKeystore=1` + 上述 keystore 配置），此时只需在构建前输入密码。

> 说明：当前官方 Unity CLI（v0.1.0-beta）命令列表中没有 `build`，无法直接打包玩家版本，
> 因此工具默认走 Unity.exe 批处理模式；`unity` CLI 用于编辑器探测（`editors list`）与「用 CLI 打开项目」（`open <path>`）。

## 依赖的工程内辅助插件

- `TBuildTool/unity/Editor/BuildCommand.cs`（安装于项目 `Assets/TBuildTool/Editor/`）
  批处理打包入口：按项目相对路径加载 Build Profile → `profile.BuildPlayer(...)`，
  成功退出码 0 / 失败 1；安卓构建时注入 keystore 密码。不会修改全局 `EditorBuildSettings`，
  场景列表完全由 Build Profile 提供（符合壁纸项目约束）。
- `TBuildTool/unity/Editor/CompileCheck.cs`
  环境编译检测入口：单环境线切换目标 + 触发脚本编译并检查错误，结果写入 JSON，
  退出码 0/1/2（详见 `TBuildTool/unity/README.md`；注意调用时**不要传 `-quit`**）。

## 目录结构

```
TBuildTool/web/
  server.js        # 零依赖 Node 后端（HTTP + SSE + 命令执行）
  config.json      # 运行后自动生成，保存界面配置（不含密码）
  public/          # 前端（index.html / app.js / style.css，无构建、无 CDN）
  start.bat        # 一键启动
```

## 环境变量

- `PORT`：服务端口（默认 8787）
- `NO_OPEN=1`：启动时不自动打开浏览器

## 常见问题

- **扫描不到 Profile**：确认项目路径正确，且 Profile 在 `Assets/Settings/Build Profiles/` 下
  （页面 ① 可自定义 Profile 目录）。
- **安卓打包失败**：多为 keystore 密码未填或填错（构建时手动输入，不保存）。
- **自动探测不到 Unity.exe**：确认 Unity Hub 的编辑器安装位置（含辅助安装路径，
  本项目在 `G:\Unity Editor`），或手动填写路径。
- **unity CLI 报 EPERM 锁文件错误**：CLI 需要写 `%APPDATA%\UnityHub`，确认该目录可写即可
  （正常桌面环境无此问题，仅部分受限环境会出现）。
- **找不到 `TBuildTool.Editor.BuildCommand.Build`**：确认工程内辅助插件已安装
  （`Assets/TBuildTool/Editor/` 下的 .cs 文件齐全，见 `TBuildTool/unity/README.md`）。