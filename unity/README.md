# TBuildTool · 工程内辅助插件（Unity Editor）

本目录是 TBuildTool 的**工程内辅助插件**：安装进 Unity 项目 `Assets/` 后在编辑器侧生效，
提供命令行打包入口与统一的构建进度钩子（`IBuildProgress`）机制。

## 内容

- `Editor/BuildCommand.cs` — 命令行打包入口
  `TBuildTool.Editor.BuildCommand.Build`（`Unity.exe -batchmode -executeMethod` 调用），
  按 Build Profile 构建，成功退出码 0 / 失败 1，不触碰全局 `EditorBuildSettings`；
  构建前自动扫描 Editor 下所有接入 `IBuildProgress` 的类并执行对应阶段函数。
- `Editor/CompileCheck.cs` — 环境编译检测入口
  `TBuildTool.Editor.CompileCheck.Run`：对单条“环境线”（BuildTarget）执行真实编译检测
  （校验平台支持性 → 切换活动构建目标 → 触发脚本重编译 → 检查编译错误），
  结果写入 `-resultFile` JSON；自动处理程序集重载续跑（SessionState），
  退出码 0=通过/不支持、1=失败、2=超时/异常。
- `Editor/BuildProgress.cs` — 构建进度钩子接口 `IBuildProgress`（+ 上下文 `BuildProgressContext`）：
  `OnBeginBuild`（构建前）/ `OnCancelled`（取消·异常）/ `OnFinishedBuild`（成功·失败）。

## 构建进度钩子（IBuildProgress）

`BuildCommand.Build` 启动时，会扫描所有 **Editor 程序集**（程序集名含 "Editor"，如
Assembly-CSharp-Editor、*.Editor）以及本插件程序集中实现了 `IBuildProgress` 的类，
实例化后按对应时期执行：

| 时期 | 接口方法 | 触发时机 |
|---|---|---|
| 构建前 | `OnBeginBuild(context)` | BuildPlayer 调用之前 |
| 取消 / 中断 | `OnCancelled(context)` | 构建抛异常或被取消（还原兜底） |
| 构建结束 | `OnFinishedBuild(context)` | BuildPlayer 返回后（成功或失败） |

新增钩子：在任意 Editor 文件夹下建一个**无参构造的普通类**实现 `IBuildProgress` 即可，无需注册。
单个钩子抛异常不影响其他钩子（记录 Warning 后继续）。

> ⚠ 行为变更：构建进度钩子不再通过 Unity 全局构建回调（IPreprocessBuildWithReport /
> IPostprocessBuildWithReport）自动生效，改为由 TBuildTool 打包时经 `IBuildProgress` 驱动——
> **编辑器内手动构建不会执行钩子，请一律通过 TBuildTool 打包**。

## 安装方式

任选其一（只装到某个 Unity 项目时推荐 1；随 TBuildTool 目录分发时推荐 2）：

### 1. 目录联接（Windows，推荐，单份源码）

把 `TBuildTool/unity` 以**目录联接**挂到项目 `Assets/TBuildTool`：

```bat
mklink /J "D:\UnityProject\Miside\Assets\TBuildTool" "D:\UnityProject\Miside\TBuildTool\unity"
```

或 PowerShell：

```powershell
New-Item -ItemType Junction -Path "D:\UnityProject\Miside\Assets\TBuildTool" -Target "D:\UnityProject\Miside\TBuildTool\unity"
```

> 注意：目录联接不会进 git。新克隆仓库后需重新执行一次上述命令。
> 就地联接时，把两个绝对路径都换成实际路径即可。

### 2. 拷贝安装（跨机器 / 无联接权限）

将 `Editor/` 文件夹（BuildCommand.cs / CompileCheck.cs / BuildProgress.cs）
拷贝到项目 `Assets/TBuildTool/Editor/`。

### 3. UPM 包引用（可选）

在项目 `Packages/manifest.json` 增加：
`"com.tbuildtool": "file:../../TBuildTool/unity"`。
（包内未携带 asmdef，自动生成程序集即可编译；若你项目有特殊程序集约束请自行调整。）

## 命令行用法（外部工具 / CI 调用）

构建（每条命令一个 Profile）：

```
Unity.exe -batchmode -nographics -quit \
  -projectPath <项目路径> \
  -executeMethod TBuildTool.Editor.BuildCommand.Build \
  -profilePath Assets/Settings/Build Profiles/WallpaperAndroid.asset \
  -outputPath <输出.apk/.aab/AS工程目录/.exe> \
  [-androidBuildKind apk|aab|gradleProject] \   # 安卓构建目标（默认 apk；gradleProject = 导出 Android Studio 工程）
  [-keystoreName KeyStore/user.keystore] [-keystoreAlias key] \
  [-keystorePass <密码>] [-keyaliasPass <密码>] \
  -logFile <日志路径>
```

环境编译检测（每条命令一条环境线，⚠ 不要传 `-quit`）：

```
Unity.exe -batchmode -nographics \
  -projectPath <项目路径> \
  -executeMethod TBuildTool.Editor.CompileCheck.Run \
  -target 13 \                            # 环境线 BuildTarget：13=Android, 19=Win64, 5=Win86, 9=iOS, 4=macOS
  -resultFile <绝对路径/result.json> \
  [-timeout 20] \                         # 单环境线超时（分钟），默认 20
  -logFile <绝对路径/check.log>
```

退出码：`0` = 编译通过 / 环境不支持（跳过）；`1` = 编译失败 / 切换失败 / 参数错误；`2` = 超时 / 内部异常。

## 迁移说明（原 MisideWallpaper.Editor）

| 原（BuildWeb 时代） | 现（TBuildTool） |
|---|---|
| `MisideWallpaper.Editor.WallpaperBuildCommand.Build` | `TBuildTool.Editor.BuildCommand.Build` |
| `Tools/MisideWallpaper/...` 菜单 | `Tools/TBuildTool/...` 菜单 |
| `Assets/MisideWallpaper/Editor/` | `TBuildTool/unity/Editor/`（经 `Assets/TBuildTool` 联接） |
| `IPreprocessBuildWithReport / IPostprocessBuildWithReport` 自动回调 | `IBuildProgress` 钩子（BuildCommand 驱动） |