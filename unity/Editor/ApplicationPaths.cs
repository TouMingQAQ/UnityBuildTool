using System;
using System.IO;
using UnityEditor;
using UnityEngine;
#if UNITY_6000_0_OR_NEWER
using UnityEditor.Build.Profile;
#endif

namespace TBuildTool.Editor
{
    // Runs in old Editors too; Application supplies the host Editor's actual paths.
    [InitializeOnLoad]
    public static class ApplicationPaths
    {
        [Serializable]
        private sealed class Snapshot
        {
            public string profilePath;
            public string productName;
            public string companyName;
            public string dataPath;
            public string streamingAssetsPath;
            public string persistentDataPath;
            public string temporaryCachePath;
            public string consoleLogPath;
            public string platform;
            public string capturedAt;
        }

        private static string lastIdentity;
        private static double nextCheck;

        static ApplicationPaths()
        {
            EditorApplication.delayCall += Capture;
            EditorApplication.update += Check;
        }

        private static void Check()
        {
            if (EditorApplication.timeSinceStartup < nextCheck) return;
            nextCheck = EditorApplication.timeSinceStartup + 2;
            if (lastIdentity != Identity()) Capture();
        }

        private static string ProfilePath()
        {
#if UNITY_6000_0_OR_NEWER
            var profile = BuildProfile.GetActiveBuildProfile();
            if (profile != null) return AssetDatabase.GetAssetPath(profile);
#endif
            return "__project_default__";
        }

        private static string Identity()
        {
            return ProfilePath() + "\n" + PlayerSettings.companyName + "\n" + PlayerSettings.productName;
        }

        public static void Capture()
        {
            try
            {
                var snapshot = new Snapshot
                {
                    profilePath = ProfilePath(),
                    productName = PlayerSettings.productName,
                    companyName = PlayerSettings.companyName,
                    dataPath = Application.dataPath,
                    streamingAssetsPath = Application.streamingAssetsPath,
                    persistentDataPath = Application.persistentDataPath,
                    temporaryCachePath = Application.temporaryCachePath,
                    consoleLogPath = Application.consoleLogPath,
                    platform = Application.platform.ToString(),
                    capturedAt = DateTime.UtcNow.ToString("o"),
                };
                string folder = Path.Combine(Directory.GetParent(Application.dataPath).FullName, ".buildtool");
                Directory.CreateDirectory(folder);
                string file = Path.Combine(folder, "application-paths.json");
                // The reader retries naturally on refresh if it observes the short write window.
                File.WriteAllText(file, JsonUtility.ToJson(snapshot, true));
                lastIdentity = Identity();
            }
            catch (Exception e)
            {
                Debug.LogWarning("[TBuildTool] 采集 Application 路径失败: " + e.Message);
            }
        }
    }
}
