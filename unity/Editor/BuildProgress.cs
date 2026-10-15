using System;
using System.Text.RegularExpressions;
using UnityEditor;
using UnityEditor.Build.Reporting;
using UnityEngine;

namespace TBuildTool.Editor
{
    /// <summary>实现于 Editor 程序集的普通类会被 BuildCommand 自动扫描；单个钩子异常记录警告后继续。</summary>
    public interface IBuildProgress
    {
        void OnBeginBuild(BuildProgressContext context);
        void OnCancelled(BuildProgressContext context);
        void OnFinishedBuild(BuildProgressContext context);
    }

    /// <summary>仅反馈详细进度，不决定构建开始、完成或成功。百分比为当前阶段进度。</summary>
    public sealed class BuildProgressReporter : IDisposable
    {
        [Serializable]
        private sealed class Message
        {
            public int version = 1;
            public string stage;
            // -1 表示当前阶段没有可靠百分比。
            public float pct;
        }

        private static readonly Regex NativeStage = new Regex(@"DisplayProgressbar:\s*([^\r\n]+)");
        private static readonly Regex NativeSteps = new Regex(@"\[(\d+)/(\d+)\s+\d+s\]");
        private static readonly object Gate = new object();
        private static string lastMessage;
        private static bool active;
        [ThreadStatic] private static bool emitting;

        public BuildProgressReporter()
        {
            active = true;
            lastMessage = null;
            Progress.added += OnProgress;
            Progress.updated += OnProgress;
            // 同步 BuildPlayer 期间也能转发原生进度日志，不依赖 EditorApplication.update。
            Application.logMessageReceivedThreaded += OnLog;
            Report("准备构建…");
        }

        /// <summary>项目自定义构建步骤可调用；pct 为 0..100，省略时只更新阶段。</summary>
        public static void Report(string stage, float pct = -1f)
        {
            if (!active || emitting || string.IsNullOrWhiteSpace(stage)) return;
            if (float.IsNaN(pct) || float.IsInfinity(pct) || pct < 0f) pct = -1f;
            else pct = Mathf.Clamp(pct, 0f, 100f);
            lock (Gate)
            {
                string json = JsonUtility.ToJson(new Message { stage = stage, pct = pct });
                if (json == lastMessage) return;
                lastMessage = json;
                emitting = true;
                try { Debug.Log("[TBuildToolProgress] " + json); }
                finally { emitting = false; }
            }
        }

        private static void OnProgress(Progress.Item[] items)
        {
            if (items == null) return;
            foreach (var item in items)
            {
                if (item == null || !item.running) continue;
                string stage = string.IsNullOrEmpty(item.description) ? item.name : item.name + "：" + item.description;
                Report(stage, item.indefinite ? -1f : item.progress * 100f);
            }
        }

        private static void OnLog(string message, string stackTrace, LogType type)
        {
            if (emitting || string.IsNullOrEmpty(message) || message.Contains("[TBuildToolProgress]")) return;
            var stage = NativeStage.Match(message);
            if (stage.Success) Report(stage.Groups[1].Value.Trim());
            var steps = NativeSteps.Match(message);
            if (steps.Success && double.TryParse(steps.Groups[1].Value, out double done)
                && double.TryParse(steps.Groups[2].Value, out double total) && total > 0)
                Report("编译构建步骤 " + steps.Groups[1].Value + "/" + steps.Groups[2].Value, (float)(done / total * 100));
        }

        public void Dispose()
        {
            Progress.added -= OnProgress;
            Progress.updated -= OnProgress;
            Application.logMessageReceivedThreaded -= OnLog;
            active = false;
        }
    }

    /// <summary>IBuildProgress 阶段回调上下文。</summary>
    public struct BuildProgressContext
    {
        public string profilePath;
        public BuildTarget target;
        public string outputPath;
        public bool cancelled;
        public bool succeeded;
        public BuildReport report;
    }
}
