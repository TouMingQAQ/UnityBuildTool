using UnityEditor;
using UnityEditor.Build.Reporting;
using UnityEngine;

namespace TBuildTool.Editor
{
    /// <summary>
    /// TBuildTool 构建进度钩子接口：接入该接口的编辑器类，会被 BuildCommand 在对应构建阶段自动调用
    /// （取代 Unity 全局构建回调 / 手动逐个调用，打包流程统一由 TBuildTool 驱动）。
    ///
    /// 接入规则：
    ///   - 实现类须放在 Editor 程序集下（程序集名含 "Editor"，如 Assembly-CSharp-Editor、*.Editor），
    ///     或与本插件（BuildCommand）同程序集；
    ///   - 必须是普通类：非 abstract / interface / static，且有公开无参构造函数；无需注册；
    ///   - 单个钩子抛异常不影响其他钩子（记录 Warning 后继续）。
    ///
    /// 阶段（对应时期）：
    ///   - OnBeginBuild    ：构建开始前（BuildPlayer 调用之前）——预检查、资源隐藏等；
    ///   - OnCancelled     ：构建被取消 / 异常中断时——还原、清理兜底；
    ///   - OnFinishedBuild ：构建结束（成功或失败，BuildPlayer 返回后）——还原、归档等。
    /// </summary>
    public interface IBuildProgress
    {
        /// <summary>构建开始前（BuildPlayer 调用之前）执行。</summary>
        void OnBeginBuild(BuildProgressContext context);

        /// <summary>构建被取消 / 异常中断时执行。</summary>
        void OnCancelled(BuildProgressContext context);

        /// <summary>构建结束后（成功或失败）执行。</summary>
        void OnFinishedBuild(BuildProgressContext context);
    }

    /// <summary>IBuildProgress 阶段回调上下文。</summary>
    public struct BuildProgressContext
    {
        /// <summary>Build Profile 资产路径（项目相对路径，如 Assets/Settings/Build Profiles/WallpaperAndroid.asset）。</summary>
        public string profilePath;

        /// <summary>构建目标平台（BuildTarget）。</summary>
        public BuildTarget target;

        /// <summary>输出路径（.apk / .exe）。</summary>
        public string outputPath;

        /// <summary>本次构建是否被取消 / 异常中断（OnCancelled 阶段必为 true）。</summary>
        public bool cancelled;

        /// <summary>本次构建是否成功（仅 OnFinishedBuild 阶段有意义）。</summary>
        public bool succeeded;

        /// <summary>构建报告（OnBeginBuild 阶段为 null；异常中断时亦为 null）。</summary>
        public BuildReport report;
    }
}