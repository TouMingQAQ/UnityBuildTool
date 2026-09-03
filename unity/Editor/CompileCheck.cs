using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Text;
using UnityEditor;
using UnityEditor.Compilation;
using UnityEngine;

namespace TBuildTool.Editor
{
    /// <summary>
    /// 环境编译检测入口（供 TBuildTool 网页工具「环境编译检测」页调用）。
    ///
    /// 对【单个】目标平台（BuildTarget，即一条“环境线”）尝试做一次真实的编译检测：
    ///   1. 校验当前主机/编辑器是否支持该环境线（平台模块缺失 → unsupported，不算编译失败）；
    ///   2. 切换活动构建目标并触发一次脚本重编译（平台宏变化会重新编译脚本程序集）；
    ///   3. 等待编译结束（自动处理“程序集重载导致静态状态丢失”——经 SessionState 续跑）；
    ///   4. 检查是否存在编译错误（EditorUtility.scriptCompilationFailed + 日志扫描兜底）；
    ///   5. 将结果写入 -resultFile（JSON），并按退出码退出。
    ///
    /// 用法（由网页工具为每条环境线启动一个 Unity 进程）：
    ///   Unity.exe -batchmode -nographics -projectPath <项目路径> \
    ///     -executeMethod TBuildTool.Editor.CompileCheck.Run \
    ///     -target 13 \
    ///     -resultFile <绝对路径/result.json> \
    ///     [-timeout 20] \           （单条环境线超时分钟数，默认 20）
    ///     -logFile <绝对路径/check.log>
    ///
    /// ⚠ 不要传 -quit：本方法需要在编译完成后调用 EditorApplication.Exit 结束进程。
    ///
    /// 退出码：0 = 通过 / 不支持(跳过)；1 = 编译失败 / 切换失败 / 参数错误；2 = 超时 / 内部异常。
    /// </summary>
    public static class CompileCheck
    {
        private const string StateKey = "TBuildTool.CompileCheck.State";

        /// <summary>会话状态（跨程序集重载持久化，经 SessionState 保存/恢复）。</summary>
        [Serializable]
        private class CheckState
        {
            public int target;
            public string targetName = "";
            public string groupName = "";
            public string resultFile = "";
            public string logFile = "";
            public int phase;              // 0=切换目标, 1=等待编译, 2=已结束
            public bool compileRequested;
            public bool observedCompiling; // 是否观察到过 isCompiling==true
            public long startedTicks;
            public long requestTicks;
            public long logMark;           // 请求编译前日志文件已读到的字节数
            public long lastTickTicks;
            public double timeoutSec = 1200;
            public string startedAtText = "";
            public string status;          // ok | fail | unsupported | error | timeout
            public string message = "";
            public long ms;
        }

        private static CheckState st;
        private static bool handlersBound;
        private static DateTime startedAt;

        public static void Run()
        {
            try
            {
                // 清理历史残留（本次进程为全新会话，双保险）
                SessionState.EraseString(StateKey);

                string[] args = Environment.GetCommandLineArgs();
                string targetArg = GetArg(args, "-target");
                string resultFile = GetArg(args, "-resultFile");
                string logFile = GetArg(args, "-logFile");
                double timeoutMin = 20;
                string timeoutArg = GetArg(args, "-timeout");
                if (!string.IsNullOrEmpty(timeoutArg) && double.TryParse(timeoutArg, out double tm) && tm > 0)
                    timeoutMin = tm;

                startedAt = DateTime.Now;

                if (string.IsNullOrEmpty(targetArg) || !int.TryParse(targetArg, out int target))
                {
                    Debug.LogError("[TBuildTool][CHECK] 缺少或无法解析 -target 参数（环境线 BuildTarget）");
                    st = new CheckState { status = "error", message = "缺少 -target 参数", resultFile = resultFile ?? "" };
                    WriteResult();
                    EditorApplication.Exit(2);
                    return;
                }

                st = new CheckState
                {
                    target = target,
                    targetName = TargetName(target),
                    groupName = ToGroupName(target),
                    resultFile = resultFile ?? "",
                    logFile = logFile ?? "",
                    startedTicks = startedAt.Ticks,
                    startedAtText = startedAt.ToString("yyyy-MM-dd HH:mm:ss"),
                    timeoutSec = timeoutMin * 60,
                };

                Debug.Log($"[TBuildTool][CHECK] begin target={target} name={st.targetName} resultFile=\"{st.resultFile}\" timeout={timeoutMin}min");

                BindReloadHandlers();
                EditorApplication.update += Tick;
                Tick();
            }
            catch (Exception e)
            {
                Debug.LogError("[TBuildTool][CHECK] Run 顶层异常: " + e);
                if (st != null)
                {
                    st.status = "error";
                    st.message = "Run 顶层异常: " + e.Message;
                    WriteResult();
                }
                EditorApplication.Exit(2);
            }
        }

        // ─────────────────────────── 状态机 ───────────────────────────

        private static void Tick()
        {
            if (st == null) return;
            st.lastTickTicks = DateTime.Now.Ticks;

            // 看门狗：单条环境线超时
            double elapsedSec = TimeSpan.FromTicks(DateTime.Now.Ticks - st.startedTicks).TotalSeconds;
            if (elapsedSec > st.timeoutSec)
            {
                st.status = "timeout";
                st.message = "检测超时（单环境线超过 " + Math.Round(st.timeoutSec / 60) + " 分钟）";
                Debug.LogError("[TBuildTool][CHECK] 检测超时");
                Finish();
                return;
            }

            try
            {
                if (st.phase == 0) DoSwitchTarget();
                else if (st.phase == 1) PollCompile();
            }
            catch (Exception e)
            {
                Debug.LogError("[TBuildTool][CHECK] 状态机异常: " + e);
                st.status = "error";
                st.message = "检测过程异常: " + e.Message;
                Finish();
            }
        }

        private static void DoSwitchTarget()
        {
            var group = ToGroup(st.target);

            // 1. 支持性检查（模块是否安装、平台是否限主机）
            if (!BuildPipeline.IsBuildTargetSupported(group, (BuildTarget)st.target))
            {
                st.status = "unsupported";
                st.message = "当前主机不支持该环境线（缺少 " + st.groupName + " 平台构建模块，或该平台仅限 macOS 主机）";
                Debug.Log($"[TBuildTool][CHECK] unsupported target={st.target} name={st.targetName}");
                Finish();
                return;
            }

            // 2. 切换活动构建目标（平台宏变化会触发脚本重编译）
            Debug.Log($"[TBuildTool][CHECK] switching target={st.target} name={st.targetName} group={st.groupName}");
            if (!EditorUserBuildSettings.SwitchActiveBuildTarget(group, (BuildTarget)st.target))
            {
                st.status = "fail";
                st.message = "切换构建目标失败（平台模块或项目设置异常，详见日志）";
                Debug.LogError($"[TBuildTool][CHECK] switch-fail target={st.target}");
                Finish();
                return;
            }

            // 3. 记录日志水位，请求脚本重编译
            st.logMark = LogFileLength();
            st.phase = 1;
            st.requestTicks = DateTime.Now.Ticks;
            st.compileRequested = true;
            if (!EditorApplication.isCompiling)
            {
                Debug.Log($"[TBuildTool][CHECK] compiling target={st.target} name={st.targetName}");
                CompilationPipeline.RequestScriptCompilation();
            }
            else
            {
                Debug.Log($"[TBuildTool][CHECK] compiling target={st.target} name={st.targetName}（切换目标已触发编译，等待完成）");
            }
            SaveState();
        }

        /// <summary>
        /// 等待编译结束。编译可能伴随程序集重载（重载后由 afterAssemblyReload 直接收尾），
        /// 也可能不重载直接结束（此时 update 在编译期间暂停，恢复后 isCompiling==false）。
        /// </summary>
        private static void PollCompile()
        {
            if (EditorApplication.isCompiling)
            {
                st.observedCompiling = true;
                SaveState();
                return;
            }

            // 尚未观察到编译开始：留 8 秒宽限，避免“刚请求还没来得及开始”被误判为“已结束”
            if (!st.observedCompiling && TimeSpan.FromTicks(DateTime.Now.Ticks - st.requestTicks).TotalSeconds < 8.0)
            {
                SaveState();
                return;
            }

            EvaluateCompileResult();
        }

        private static void EvaluateCompileResult()
        {
            bool failed = false;

            // 主判据：EditorUtility.scriptCompilationFailed（反射读取，兼容不同版本）
            bool? propFailed = ScriptCompilationFailed();
            if (propFailed == true) failed = true;

            // 兜底判据：扫描日志中新增的编译错误行（error CS / Scripts have compiler errors）
            var errors = ScanLogCompileErrors();
            if (errors.Count > 0) failed = true;

            st.status = failed ? "fail" : "ok";
            st.message = failed
                ? (errors.Count > 0 ? string.Join("  |  ", errors) : "脚本编译失败（详见日志）")
                : "编译通过（无脚本错误）";

            Debug.Log($"[TBuildTool][CHECK] result target={st.target} status={st.status} message=\"{st.message}\"");
            if (failed)
                Debug.LogError($"[TBuildTool][CHECK] 环境线 {st.targetName} 编译失败");

            Finish();
        }

        private static void Finish()
        {
            st.phase = 2;
            // 用 state 里持久化的开始时间计算耗时（跨程序集重载后静态 startedAt 已重置）
            st.ms = Math.Max(0, (long)TimeSpan.FromTicks(DateTime.Now.Ticks - st.startedTicks).TotalMilliseconds);
            WriteResult();

            int exitCode = st.status == "ok" || st.status == "unsupported" ? 0
                         : st.status == "timeout" ? 2 : 1;
            Debug.Log($"[TBuildTool][CHECK] finish target={st.target} status={st.status} ms={st.ms} exit={exitCode}");

            EditorApplication.update -= Tick;
            EditorApplication.Exit(exitCode);
        }

        // ─────────────────────────── 程序集重载续跑 ───────────────────────────

        private static void BindReloadHandlers()
        {
            if (handlersBound) return;
            handlersBound = true;
            AssemblyReloadEvents.beforeAssemblyReload += OnBeforeReload;
            AssemblyReloadEvents.afterAssemblyReload += OnAfterReload;
        }

        private static void OnBeforeReload()
        {
            if (st != null && st.phase == 1) SaveState();
        }

        private static void OnAfterReload()
        {
            // 重新载入静态状态并续跑
            LoadState();
            if (st == null) return;
            handlersBound = false;
            BindReloadHandlers();
            Debug.Log("[TBuildTool][CHECK] 程序集重载完成，继续环境检测…");
            // 发生了重载 ⇒ 编译必然已结束；把收尾推迟到下一个 update（避免在重载事件里直接 Exit）
            st.observedCompiling = true;
            EditorApplication.update += Tick;
        }

        private static void SaveState()
        {
            try { SessionState.SetString(StateKey, JsonUtility.ToJson(st)); }
            catch (Exception e) { Debug.LogWarning("[TBuildTool][CHECK] 保存状态失败: " + e.Message); }
        }

        private static void LoadState()
        {
            try
            {
                string json = SessionState.GetString(StateKey, null);
                st = string.IsNullOrEmpty(json) ? null : JsonUtility.FromJson<CheckState>(json);
            }
            catch (Exception e)
            {
                Debug.LogWarning("[TBuildTool][CHECK] 恢复状态失败: " + e.Message);
                st = null;
            }
        }

        // ─────────────────────────── 结果判定辅助 ───────────────────────────

        private static bool? ScriptCompilationFailed()
        {
            try
            {
                var prop = typeof(EditorUtility).GetProperty("scriptCompilationFailed", BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic);
                if (prop == null) return null;
                return prop.GetValue(null) is bool b ? b : (bool?)null;
            }
            catch { return null; }
        }

        private static List<string> ScanLogCompileErrors()
        {
            var errors = new List<string>();
            if (string.IsNullOrEmpty(st.logFile) || !File.Exists(st.logFile)) return errors;
            try
            {
                long total;
                byte[] buf;
                using (var fs = new FileStream(st.logFile, FileMode.Open, FileAccess.Read, FileShare.ReadWrite))
                {
                    total = fs.Length;
                    long start = Math.Min(st.logMark, total);
                    if (total <= start) return errors;
                    long count = total - start;
                    buf = new byte[count];
                    fs.Seek(start, SeekOrigin.Begin);
                    int read = fs.Read(buf, 0, (int)count);
                    if (read < count)
                    {
                        byte[] buf2 = new byte[read];
                        Buffer.BlockCopy(buf, 0, buf2, 0, read);
                        buf = buf2;
                    }
                }
                string text = Encoding.UTF8.GetString(buf).Replace("\r\n", "\n");
                foreach (string line in text.Split('\n'))
                {
                    string trimmed = line.Trim();
                    if (trimmed.Length == 0) continue;
                    if (trimmed.Contains("error CS") || trimmed.Contains("error BC") ||
                        trimmed.IndexOf("Scripts have compiler errors", StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        errors.Add(truncate(trimmed, 300));
                        if (errors.Count >= 3) break;
                    }
                }
            }
            catch { /* 日志不可读时忽略，主判据兜底 */ }
            return errors;
        }

        private static string truncate(string s, int max)
        {
            return s.Length <= max ? s : s.Substring(0, max) + "…";
        }

        private static long LogFileLength()
        {
            try
            {
                return string.IsNullOrEmpty(st.logFile) || !File.Exists(st.logFile) ? 0 : new FileInfo(st.logFile).Length;
            }
            catch { return 0; }
        }

        // ─────────────────────────── 结果文件 ───────────────────────────

        private static void WriteResult()
        {
            if (st == null || string.IsNullOrEmpty(st.resultFile)) return;
            try
            {
                string dir = Path.GetDirectoryName(st.resultFile);
                if (!string.IsNullOrEmpty(dir)) Directory.CreateDirectory(dir);

                bool ok = st.status == "ok" || st.status == "unsupported";
                string json =
                    "{\n" +
                    "  \"target\": " + st.target + ",\n" +
                    "  \"name\": \"" + Esc(st.targetName) + "\",\n" +
                    "  \"group\": \"" + Esc(st.groupName) + "\",\n" +
                    "  \"ok\": " + (ok ? "true" : "false") + ",\n" +
                    "  \"status\": \"" + Esc(st.status ?? "error") + "\",\n" +
                    "  \"ms\": " + st.ms + ",\n" +
                    "  \"message\": \"" + Esc(st.message ?? "") + "\",\n" +
                    "  \"unityVersion\": \"" + Esc(Application.unityVersion) + "\",\n" +
                    "  \"startedAt\": \"" + Esc(st.startedAtText) + "\",\n" +
                    "  \"finishedAt\": \"" + Esc(DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss")) + "\"\n" +
                    "}\n";
                File.WriteAllText(st.resultFile, json);
                Debug.Log("[TBuildTool][CHECK] 结果文件已写入: " + st.resultFile);
            }
            catch (Exception e)
            {
                Debug.LogWarning("[TBuildTool][CHECK] 写结果文件失败: " + e.Message);
            }
        }

        private static string Esc(string s)
        {
            if (s == null) return "";
            var sb = new StringBuilder(s.Length);
            foreach (char c in s)
            {
                switch (c)
                {
                    case '"': sb.Append("\\\""); break;
                    case '\\': sb.Append("\\\\"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\t': sb.Append("\\t"); break;
                    default:
                        if (c < 32) sb.Append("\\u").Append(((int)c).ToString("x4"));
                        else sb.Append(c);
                        break;
                }
            }
            return sb.ToString();
        }

        // ─────────────────────────── 平台映射 ───────────────────────────

        private static string TargetName(int target)
        {
            switch (target)
            {
                case 5: return "Windows (x86)";
                case 19: return "Windows (x64)";
                case 13: return "Android";
                case 9: return "iOS";
                case 4: return "macOS";
                default: return "BuildTarget " + target;
            }
        }

        private static BuildTargetGroup ToGroup(int target)
        {
            switch (target)
            {
                case 13: return BuildTargetGroup.Android;
                case 9: return BuildTargetGroup.iOS;
                default: return BuildTargetGroup.Standalone; // 5/19/4 及其他桌面平台
            }
        }

        private static string ToGroupName(int target)
        {
            return ToGroup(target).ToString();
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