//! Read Unity Player Settings for profile scanning.
use super::*;

const KEYS: [&str; 3] = ["productName", "companyName", "bundleVersion"];

fn scalar(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(String::new());
    }
    match serde_yaml::from_str::<serde_yaml::Value>(raw).ok()? {
        serde_yaml::Value::String(s) => Some(s),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        serde_yaml::Value::Null => Some(String::new()),
        _ => None,
    }
}

// Legacy Unity 6 stores each line of the inner PlayerSettings YAML as a YAML string.
fn field(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if let Some(raw) = trimmed.strip_prefix("- line:") {
        let inner = scalar(raw)?;
        let inner = inner.strip_prefix("| ")?;
        let (key, value) = inner.trim_start().split_once(':')?;
        if KEYS.contains(&key) {
            return Some((key.into(), scalar(value)?));
        }
    } else if let Some((key, value)) = trimmed.split_once(':') {
        if KEYS.contains(&key) {
            return Some((key.into(), scalar(value)?));
        }
    }
    None
}

fn values(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(field).collect()
}

pub(super) fn read(text: &str, global: &str) -> Value {
    let fields = values(text);
    let globals = values(global);
    let mut result = json!({});
    for key in KEYS {
        let own = fields.iter().find(|(k, _)| k == key);
        let inherited = globals.iter().find(|(k, _)| k == key);
        result[key] = json!(own.or(inherited).map(|(_, v)| v.as_str()));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    const GLOBAL: &str = "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!129 &1\nPlayerSettings:\n  companyName: Global Company\n  productName: Global Game\n  bundleVersion: 1.2.3\n  AndroidBundleVersionCode: 42\n";
    const PROFILE: &str = "--- !u!114 &11400000\nMonoBehaviour:\n  m_EditorClassIdentifier: UnityEditor.BuildProfileModule::UnityEditor.Build.Profile.BuildProfile\n  m_Name: Windows\n  m_BuildTarget: 19\n  m_PlayerSettingsYaml:\n    m_Settings:\n    - line: '| PlayerSettings:'\n    - line: '|   companyName: Original Company'\n    - line: '|   productName: Original Game'\n    - line: '|   bundleVersion: 2.0'\n    - line: '|   AndroidBundleVersionCode: 8'\n  m_Other: unchanged\n";

    #[test]
    fn reads_profile_overrides() {
        let info = read(PROFILE, GLOBAL);
        assert_eq!(info["productName"], "Original Game");
        assert_eq!(info["companyName"], "Original Company");
        assert_eq!(info["bundleVersion"], "2.0");
    }

    #[test]
    fn inherits_missing_profile_fields_from_project_settings() {
        let info = read("MonoBehaviour:\n  m_PlayerSettingsYaml:\n    m_Settings: []\n", GLOBAL);
        assert_eq!(info, read(GLOBAL, GLOBAL));
        assert_eq!(info["productName"], "Global Game");
        assert_eq!(info["companyName"], "Global Company");
        assert_eq!(info["bundleVersion"], "1.2.3");
    }

    #[test]
    fn reads_plain_subasset_settings() {
        let plain = "MonoBehaviour:\n  m_Name: BuildProfile\n--- !u!129 &2\nPlayerSettings:\n  productName: Game\n  companyName: Studio\n  bundleVersion: 1.0\n  other: keep";
        let info = read(plain, GLOBAL);
        assert_eq!(info["productName"], "Game");
        assert_eq!(info["companyName"], "Studio");
        assert_eq!(info["bundleVersion"], "1.0");
    }
}
