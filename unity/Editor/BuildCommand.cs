using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using UnityEditor;
using UnityEditor.Build.Profile;
using UnityEditor.Build.Reporting;
using UnityEngine;

namespace TBuildTool.Editor
{
    /// <summary>
    /// 命令行打包入口（供 TBuildTool 网页打包工具通过 Unity.exe -batchmode -executeMethod 调用）。
    /// 本脚本属于 TBuildTool 工程内辅助插件（TBuildTool/unity），安装于 Assets/TBuildTool/Editor/。
    ///
    /// 用法：
    ///   Unity.exe -batchmode -nographics -quit \
    ///     -projectPath <项目路径> \
    ///     -executeMethod TBuildTool.Editor.BuildCommand.Build \
    ///     -profilePath Assets/Settings/Build Profiles/WallpaperAndroid.asset \
    ///     -outputPath <输出.apk/.exe/Xcode目录> \
    ///     [-wallpaperTarget 13] \                     （可选，13=Android, 19=Win64, 5=Win86, 9=iOS）
    ///     [-dev true] \                               （可选，开启 Development Build）
    ///     [-buildAddressables true/方法名] \          （可选，构建前先打包 Addressables）
    ///     [-keystoreName KeyStore/user.keystore] \   （可选，安卓签名文件覆盖）
    ///     [-keystoreAlias key] \                      （可选，别名覆盖）
    ///     [-keystorePass <keystore密码>] \            （可选，安卓必填否则签名失败）
    ///     [-keyaliasPass <key密码>] \                （可选，默认同 keystorePass）
    ///     [-versionCode <数字>] \                     （可选，安卓 bundleVersionCode）
    ///     -logFile <日志路径>
    /// </summary>
    public static class BuildCommand
    {
        public static void Build()
        {
            string[] args = Environment.GetCommandLineArgs();
            string profilePath   = GetArg(args, "-profilePath");
            string outputPath    = GetArg(args, "-outputPath");
            string keystoreName  = GetArg(args, "-keystoreName");
            string keystoreAlias = GetArg(args, "-keystoreAlias");
            string keystorePass  = GetArg(args, "-keystorePass");
            string keyaliasPass  = GetArg(args, "-keyaliasPass");
            string devArg        = GetArg(args, "-dev") ?? GetArg(args, "-development");
            string addrArg       = GetArg(args, "-buildAddressables") ?? GetArg(args, "-addressables");

            if (string.IsNullOrEmpty(profilePath) || string.IsNullOrEmpty(outputPath))
            {
                Debug.LogError("[TBuildTool] 缺少 -profilePath / -outputPath 参数，无法构建。");
                EditorApplication.Exit(1);
                return;
            }

            var profile = AssetDatabase.LoadAssetAtPath<BuildProfile>(profilePath);
            if (profile == null)
            {
                Debug.LogError("[TBuildTool] 找不到 Build Profile: " + profilePath);
                EditorApplication.Exit(1);
                return;
            }

            // 先激活 Profile，避免批处理/CI 下出现 "Build profile is invalid" 问题
            try { BuildProfile.SetActiveBuildProfile(profile); }
            catch (Exception e) { Debug.LogWarning("[TBuildTool] 激活 Profile 失败（可忽略）: " + e.Message); }

            // 目标平台：优先用显式传入的 -wallpaperTarget（数字或名称），否则取当前激活目标。
            BuildTarget target = EditorUserBuildSettings.activeBuildTarget;
            string btArg = GetArg(args, "-wallpaperTarget");
            if (!string.IsNullOrEmpty(btArg))
            {
                if (int.TryParse(btArg, out int btInt))
                    target = (BuildTarget)btInt;
                else if (Enum.TryParse(btArg, true, out BuildTarget btEnum))
                    target = btEnum;
            }

            // 安卓签名注入（仅 Android 目标）
            ApplyAndroidSigning(target, keystoreName, keystoreAlias, keystorePass, keyaliasPass);

            // 安卓构建号（bundleVersionCode）：网页工具自动递增后通过 -versionCode 传入
            string versionCodeArg = GetArg(args, "-versionCode");
            if (target == BuildTarget.Android && !string.IsNullOrEmpty(versionCodeArg)
                && int.TryParse(versionCodeArg, out int versionCode) && versionCode > 0)
            {
                PlayerSettings.Android.bundleVersionCode = versionCode;
                Debug.Log($"[TBuildTool] 安卓构建号(bundleVersionCode) → {versionCode}");
            }

            // ── 需求 3：构建 Addressables 资源包 ──
            bool needBuildAddressables = !string.IsNullOrEmpty(addrArg) && !string.Equals(addrArg, "false", StringComparison.OrdinalIgnoreCase);
            if (needBuildAddressables)
            {
                string customMethod = (string.Equals(addrArg, "true", StringComparison.OrdinalIgnoreCase) || string.Equals(addrArg, "1", StringComparison.OrdinalIgnoreCase)) ? null : addrArg;
                bool addrOk = ExecuteAddressablesBuild(customMethod);
                if (!addrOk)
                {
                    Debug.LogError("[TBuildTool] Addressables 构建失败，中止后续构建。");
                    EditorApplication.Exit(1);
                    return;
                }
            }

            // ── 需求 5：DEV (Development Build) 选项 ──
            BuildOptions buildOptions = BuildOptions.None;
            bool isDev = !string.IsNullOrEmpty(devArg) &&
                         (string.Equals(devArg, "true", StringComparison.OrdinalIgnoreCase) ||
                          string.Equals(devArg, "1", StringComparison.OrdinalIgnoreCase) ||
                          string.Equals(devArg, "dev", StringComparison.OrdinalIgnoreCase));
            if (isDev)
            {
                buildOptions |= BuildOptions.Development | BuildOptions.AllowDebugging;
                Debug.Log("[TBuildTool] 🐞 已开启 DEV (Development Build) 调试模式");
            }

            string dir = Path.GetDirectoryName(outputPath);
            if (!string.IsNullOrEmpty(dir))
                Directory.CreateDirectory(dir);

            // ── 构建进度钩子：扫描 Editor 下所有接入 IBuildProgress 的类，执行构建前阶段 ──
            var context = new BuildProgressContext
            {
                profilePath = profilePath,
                target      = target,
                outputPath  = outputPath,
            };
            var hooks = ScanBuildProgressHooks();
            Debug.Log("[TBuildTool] BuildProgress 钩子：" + hooks.Count + " 个"
                + (hooks.Count > 0 ? " → " + string.Join("、", hooks.ConvertAll(h => h.GetType().Name)) : ""));
            InvokeBuildProgress(hooks, context, h => h.OnBeginBuild(context), "OnBeginBuild");

            Debug.Log($"[TBuildTool] 开始构建 profile=\"{profile.name}\" target={target} (Options={buildOptions})");
            Debug.Log($"[TBuildTool] 输出目标路径: {outputPath}");

            BuildReport report = null;
            try
            {
                report = BuildPipeline.BuildPlayer(new BuildPlayerWithProfileOptions
                {
                    buildProfile = profile,
                    locationPathName = outputPath,
                    options = buildOptions,
                });
            }
            catch (Exception e)
            {
                Debug.LogError("[TBuildTool] 构建抛出异常: " + e);
                context.cancelled = true;
                InvokeBuildProgress(hooks, context, h => h.OnCancelled(context), "OnCancelled");
                EditorApplication.Exit(1);
                return;
            }

            context.report = report;

            // ── 构建结果分发到对应时期：取消 → OnCancelled；其余（成功/失败）→ OnFinishedBuild ──
            if (report.summary.result == BuildResult.Cancelled)
            {
                context.cancelled = true;
                InvokeBuildProgress(hooks, context, h => h.OnCancelled(context), "OnCancelled");
            }
            else
            {
                context.succeeded = report.summary.result == BuildResult.Succeeded;
                InvokeBuildProgress(hooks, context, h => h.OnFinishedBuild(context), "OnFinishedBuild");
            }

            if (context.succeeded)
            {
                Debug.Log($"[TBuildTool] 构建成功: {report.summary.outputPath} ({report.summary.totalSize} bytes)");
                EditorApplication.Exit(0);
            }
            else
            {
                Debug.LogError($"[TBuildTool] 构建失败: {report.summary.result}，错误数 {report.summary.totalErrors}");
                EditorApplication.Exit(1);
            }
        }

        // ─────────────────────────── Addressables 构建 ───────────────────────────

        /// <summary>
        /// 执行 Addressables 构建：优先尝试用户指定的静态方法，否则反射调用官方 AddressableAssetSettings.BuildPlayerContent
        /// </summary>
        private static bool ExecuteAddressablesBuild(string customMethodName)
        {
            Debug.Log("[TBuildTool] ▶ 开始执行 Addressables 资源包构建…");
            
            // 1. 若指定了自定义构建函数（如 "MyNamespace.MyTool.BuildAddressables"）
            if (!string.IsNullOrEmpty(customMethodName))
            {
                Debug.Log($"[TBuildTool] 正在调用自定义 Addressables 构建函数: {customMethodName}");
                try
                {
                    int lastDot = customMethodName.LastIndexOf('.');
                    if (lastDot > 0)
                    {
                        string typeName = customMethodName.Substring(0, lastDot);
                        string methodName = customMethodName.Substring(lastDot + 1);
                        foreach (var asm in AppDomain.CurrentDomain.GetAssemblies())
                        {
                            var type = asm.GetType(typeName);
                            if (type != null)
                            {
                                var method = type.GetMethod(methodName, BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic);
                                if (method != null)
                                {
                                    object res = method.Invoke(null, null);
                                    if (res is bool b && !b) return false;
                                    Debug.Log("[TBuildTool] ✓ 自定义 Addressables 构建函数执行成功");
                                    return true;
                                }
                            }
                        }
                    }
                    Debug.LogWarning($"[TBuildTool] 未找到指定的静态函数 {customMethodName}，尝试使用默认 Addressables 构建…");
                }
                catch (Exception ex)
                {
                    Debug.LogError($"[TBuildTool] 调用自定义 Addressables 函数异常: {ex}");
                    return false;
                }
            }

            // 2. 默认反射调用 Unity 官方 Addressables API
            try
            {
                Type settingsType = Type.GetType("UnityEditor.AddressableAssets.Settings.AddressableAssetSettings, Unity.Addressables.Editor")
                                 ?? Type.GetType("UnityEditor.AddressableAssets.Settings.AddressableAssetSettings, Unity.Addressables");
                Type defaultObjectType = Type.GetType("UnityEditor.AddressableAssets.AddressableAssetSettingsDefaultObject, Unity.Addressables.Editor")
                                      ?? Type.GetType("UnityEditor.AddressableAssets.AddressableAssetSettingsDefaultObject, Unity.Addressables");

                if (settingsType == null || defaultObjectType == null)
                {
                    Debug.LogWarning("[TBuildTool] 项目中未找到 Addressables Editor 程序集（可能未安装 com.unity.addressables），跳过 Addressables 构建。");
                    return true;
                }

                var getSettingsMethod = defaultObjectType.GetProperty("Settings", BindingFlags.Static | BindingFlags.Public)?.GetGetMethod();
                object settingsInstance = getSettingsMethod?.Invoke(null, null);
                if (settingsInstance == null)
                {
                    Debug.LogWarning("[TBuildTool] 未找到 AddressableAssetSettings 配置资产（AddressableAssetSettingsDefaultObject.Settings 为 null），跳过。");
                    return true;
                }

                var buildPlayerContentMethod = settingsType.GetMethod("BuildPlayerContent", BindingFlags.Static | BindingFlags.Public, null, Type.EmptyTypes, null)
                                            ?? settingsType.GetMethod("BuildPlayerContent", BindingFlags.Instance | BindingFlags.Public, null, Type.EmptyTypes, null);

                if (buildPlayerContentMethod != null)
                {
                    object targetObj = buildPlayerContentMethod.IsStatic ? null : settingsInstance;
                    buildPlayerContentMethod.Invoke(targetObj, null);
                    Debug.Log("[TBuildTool] ✓ Addressables 资源包构建完成");
                    return true;
                }
                else
                {
                    Debug.LogWarning("[TBuildTool] 未能找到 AddressableAssetSettings.BuildPlayerContent 方法。");
                    return true;
                }
            }
            catch (Exception ex)
            {
                Debug.LogError("[TBuildTool] 执行 Addressables 构建时发生异常: " + ex);
                return false;
            }
        }

        // ─────────────────────────── IBuildProgress 钩子扫描 ───────────────────────────

        private static List<IBuildProgress> ScanBuildProgressHooks()
        {
            var hooks = new List<IBuildProgress>();
            var seen = new HashSet<string>();

            Assembly[] assemblies;
            try { assemblies = AppDomain.CurrentDomain.GetAssemblies(); }
            catch { return hooks; }

            var self = typeof(BuildCommand).Assembly;
            foreach (var asm in assemblies)
            {
                if (asm == null) continue;
                string asmName = asm.GetName().Name ?? string.Empty;

                if (asmName.StartsWith("Unity", StringComparison.Ordinal)
                    || asmName.StartsWith("System", StringComparison.Ordinal)
                    || asmName.StartsWith("Mono", StringComparison.Ordinal)
                    || asmName.StartsWith("netstandard", StringComparison.Ordinal)
                    || asmName == "mscorlib")
                    continue;

                if (asm != self && asmName.IndexOf("Editor", StringComparison.OrdinalIgnoreCase) < 0)
                    continue;

                Type[] types;
                try { types = asm.GetTypes(); }
                catch (ReflectionTypeLoadException e) { types = e.Types; }
                catch { continue; }
                if (types == null) continue;

                foreach (var t in types)
                {
                    if (t == null || !t.IsClass || t.IsAbstract || t.IsInterface || t.IsGenericTypeDefinition)
                        continue;
                    if (!typeof(IBuildProgress).IsAssignableFrom(t))
                        continue;
                    if (t.GetConstructor(Type.EmptyTypes) == null)
                        continue;

                    string key = t.FullName ?? t.AssemblyQualifiedName ?? t.Name;
                    if (!seen.Add(key)) continue;

                    try { hooks.Add((IBuildProgress)Activator.CreateInstance(t)); }
                    catch (Exception e) { Debug.LogWarning("[TBuildTool] 实例化 IBuildProgress 实现 " + key + " 失败: " + e.Message); }
                }
            }
            return hooks;
        }

        private static void InvokeBuildProgress(List<IBuildProgress> hooks, BuildProgressContext context,
            Action<IBuildProgress> action, string phaseName)
        {
            if (hooks == null) return;
            foreach (var hook in hooks)
            {
                try { action(hook); }
                catch (Exception e)
                {
                    Debug.LogWarning("[TBuildTool] IBuildProgress." + phaseName + " 钩子异常（" + hook.GetType().FullName + "）: " + e.Message);
                }
            }
        }

        // ─────────────────────────── 签名 / 参数 ───────────────────────────

        private static void ApplyAndroidSigning(BuildTarget target, string keystoreName, string keystoreAlias,
            string keystorePass, string keyaliasPass)
        {
            if (target != BuildTarget.Android)
                return;

            if (!string.IsNullOrEmpty(keystoreName))
            {
                PlayerSettings.Android.useCustomKeystore = true;
                PlayerSettings.Android.keystoreName = StripInproject(keystoreName);
                Debug.Log($"[TBuildTool] keystore 文件 → {PlayerSettings.Android.keystoreName}");
            }

            if (!string.IsNullOrEmpty(keystoreAlias))
            {
                PlayerSettings.Android.keyaliasName = keystoreAlias;
                Debug.Log($"[TBuildTool] key alias → {keystoreAlias}");
            }

            if (string.IsNullOrEmpty(keyaliasPass))
                keyaliasPass = keystorePass;

            if (!string.IsNullOrEmpty(keystorePass))
            {
                PlayerSettings.Android.keystorePass = keystorePass;
                PlayerSettings.Android.keyaliasPass = keyaliasPass;
                Debug.Log("[TBuildTool] 已注入 keystore / key 密码（不打印明文）");
            }
        }

        private static string StripInproject(string p)
        {
            return p.StartsWith("{inproject}:") ? p.Substring("{inproject}:".Length).Trim() : p;
        }

        private static string GetArg(string[] args, string name)
        {
            for (int i = 0; i < args.Length - 1; i++)
                if (args[i] == name)
                    return args[i + 1];
            return null;
        }
    }
}
