//! Read and update Unity Player Settings without rewriting unrelated YAML.
use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;

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
fn field(line: &str) -> Option<(String, String, bool)> {
    let trimmed = line.trim();
    if let Some(raw) = trimmed.strip_prefix("- line:") {
        let inner = scalar(raw)?;
        let inner = inner.strip_prefix("| ")?;
        let (key, value) = inner.trim_start().split_once(':')?;
        if KEYS.contains(&key) {
            return Some((key.into(), scalar(value)?, true));
        }
    } else if let Some((key, value)) = trimmed.split_once(':') {
        if KEYS.contains(&key) {
            return Some((key.into(), scalar(value)?, false));
        }
    }
    None
}

fn values(text: &str) -> Vec<(String, String, bool)> {
    text.lines().filter_map(field).collect()
}

fn revision(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

// Creating an override requires the complete PlayerSettings document, not three loose keys.
fn initialize_override(text: &str, global: &str) -> Option<String> {
    let empty = regex::Regex::new(r"(?m)^( +)m_Settings: \[\](\r?)$").ok()?;
    if !text.contains("m_PlayerSettingsYaml:")
        || !global.contains("PlayerSettings:")
        || values(global).len() != 3
        || !values(text).is_empty()
        || empty.find_iter(text).count() != 1
    {
        return None;
    }
    let caps = empty.captures(text)?;
    let indent = caps.get(1)?.as_str();
    let newline = if caps.get(2)?.as_str() == "\r" {
        "\r\n"
    } else {
        "\n"
    };
    let mut replacement = format!("{indent}m_Settings:");
    for line in global
        .lines()
        .skip_while(|line| line.trim() != "PlayerSettings:")
    {
        replacement.push_str(newline);
        replacement.push_str(&format!(
            "{indent}- line: {}",
            serde_json::to_string(&format!("| {line}")).ok()?
        ));
    }
    if newline == "\r\n" {
        replacement.push('\r');
    }
    let found = caps.get(0)?;
    Some(format!(
        "{}{}{}",
        &text[..found.start()],
        replacement,
        &text[found.end()..]
    ))
}

pub(super) fn describe(text: &str, global: &str, fallback: bool) -> Value {
    let fields = values(text);
    let globals = values(global);
    let editable = KEYS
        .iter()
        .all(|key| fields.iter().filter(|(k, _, _)| k == key).count() == 1)
        || (!fallback && initialize_override(text, global).is_some());
    let mut result =
        json!({"editable": editable, "revision": revision(text), "fallback": fallback});
    if !fallback && initialize_override(text, global).is_some() {
        result["revision"] = json!(revision(&format!("{text}\0{global}")));
    }
    for key in KEYS {
        let own = fields.iter().find(|(k, _, _)| k == key);
        let inherited = globals.iter().find(|(k, _, _)| k == key);
        result[key] = json!(own.or(inherited).map(|(_, v, _)| v.as_str()));
    }
    result["inherited"] = json!(
        !fallback
            && KEYS
                .iter()
                .any(|key| !fields.iter().any(|(k, _, _)| k == key))
    );
    result
}

fn update(text: &str, input: &Value) -> Result<String, Fault> {
    let fields = values(text);
    for key in KEYS {
        if fields.iter().filter(|(k, _, _)| k == key).count() != 1 {
            return Err((400, "文件中缺少完整且唯一的 Player Settings 字段，无法安全修改；请先在 Unity 的 Build Profile 中启用 Player Settings 覆盖".into()));
        }
        let value = input[key]
            .as_str()
            .ok_or((400, format!("缺少字段 {key}")))?;
        if value.len() > 1024 || value.chars().any(char::is_control) {
            return Err((400, format!("{key} 不能包含控制字符且不能超过 1024 字节")));
        }
    }
    let mut output = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let ending = if line.ends_with("\r\n") {
            "\r\n"
        } else if line.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let body = line.trim_end_matches(['\r', '\n']);
        if let Some((key, _, nested)) = field(body) {
            let value = serde_json::to_string(input[&key].as_str().unwrap()).unwrap();
            if nested {
                let start = body.find("- line:").unwrap();
                let raw = body[start + 7..].trim();
                let inner = scalar(raw).unwrap();
                let colon = inner.find(&format!("{key}:")).unwrap() + key.len() + 1;
                let new_inner = format!("{} {value}", &inner[..colon]);
                output.push_str(&format!(
                    "{}- line: {}",
                    &body[..start],
                    serde_json::to_string(&new_inner).unwrap()
                ));
            } else {
                let colon = body.find(':').unwrap();
                output.push_str(&format!("{} {value}", &body[..=colon]));
            }
        } else {
            output.push_str(body);
        }
        output.push_str(ending);
    }
    Ok(output)
}

impl Backend {
    fn profile_settings_path(&self, body: &Value) -> Result<(PathBuf, PathBuf, bool), Fault> {
        let config = self.config();
        let project_raw = valstr(&config, "projectPath");
        if project_raw.trim().is_empty() {
            return Err((400, "请先保存 Unity 项目路径".into()));
        }
        let project = fs::canonicalize(self.resolve(&project_raw))
            .map_err(|_| (400, "Unity 项目路径不存在".into()))?;
        if !project_valid(&project) {
            return Err((400, "Unity 项目路径无效".into()));
        }
        let asset = valstr(body, "assetPath");
        let fallback = asset == PROJECT_DEFAULT_PROFILE;
        let target = if fallback {
            project.join("ProjectSettings/ProjectSettings.asset")
        } else {
            let relative = Path::new(&asset);
            if relative.is_absolute()
                || !relative
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                || relative.extension().and_then(|s| s.to_str()) != Some("asset")
            {
                return Err((400, "Profile 路径无效".into()));
            }
            project.join(relative)
        };
        let target =
            fs::canonicalize(target).map_err(|_| (404, "Profile 或项目设置文件不存在".into()))?;
        let allowed = if fallback {
            project.join("ProjectSettings")
        } else {
            project.join("Assets")
        };
        let allowed = fs::canonicalize(allowed).map_err(|_| (400, "项目目录不存在".into()))?;
        if !target.starts_with(allowed) {
            return Err((403, "Profile 路径超出项目目录".into()));
        }
        let text = fs::read_to_string(&target).map_err(|e| (500, e.to_string()))?;
        if !fallback && !text.contains("BuildProfile") {
            return Err((400, "所选文件不是 Build Profile".into()));
        }
        Ok((project, target, fallback))
    }
    pub(super) fn profile_settings_read(&self, body: &Value) -> Result<Value, Fault> {
        let (project, target, fallback) = self.profile_settings_path(body)?;
        let text = fs::read_to_string(&target).map_err(|e| (500, e.to_string()))?;
        let global = fs::read_to_string(project.join("ProjectSettings/ProjectSettings.asset"))
            .unwrap_or_default();
        let mut info = describe(&text, &global, fallback);
        info["path"] = json!(target);
        Ok(info)
    }
    pub(super) fn profile_settings_write(&self, body: &Value) -> Result<Value, Fault> {
        self.acquire()?;
        let result = (|| {
            let (project, target, fallback) = self.profile_settings_path(body)?;
            let text = fs::read_to_string(&target).map_err(|e| (500, e.to_string()))?;
            let global = fs::read_to_string(project.join("ProjectSettings/ProjectSettings.asset"))
                .unwrap_or_default();
            if body["revision"] != describe(&text, &global, fallback)["revision"] {
                return Err((409, "文件已被其他程序修改，请重新读取后再保存".into()));
            }
            let source = if fallback {
                text.clone()
            } else {
                initialize_override(&text, &global).unwrap_or_else(|| text.clone())
            };
            let updated = update(&source, body)?;
            let mut temp = tempfile::NamedTempFile::new_in(target.parent().unwrap())
                .map_err(|e| (500, e.to_string()))?;
            temp.write_all(updated.as_bytes())
                .map_err(|e| (500, e.to_string()))?;
            temp.as_file()
                .set_permissions(
                    fs::metadata(&target)
                        .map_err(|e| (500, e.to_string()))?
                        .permissions(),
                )
                .map_err(|e| (500, e.to_string()))?;
            if fs::read_to_string(&target).map_err(|e| (500, e.to_string()))? != text {
                return Err((409, "文件已被其他程序修改，请重新读取后再保存".into()));
            }
            temp.persist(&target).map_err(|e| (500, e.to_string()))?;
            self.profile_settings_read(body)
        })();
        self.release();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const GLOBAL: &str = "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!129 &1\nPlayerSettings:\n  companyName: Global Company\n  productName: Global Game\n  bundleVersion: 1.2.3\n  AndroidBundleVersionCode: 42\n";
    const PROFILE: &str = "--- !u!114 &11400000\nMonoBehaviour:\n  m_EditorClassIdentifier: UnityEditor.BuildProfileModule::UnityEditor.Build.Profile.BuildProfile\n  m_Name: Windows\n  m_BuildTarget: 19\n  m_PlayerSettingsYaml:\n    m_Settings:\n    - line: '| PlayerSettings:'\n    - line: '|   companyName: Original Company'\n    - line: '|   productName: Original Game'\n    - line: '|   bundleVersion: 2.0'\n    - line: '|   AndroidBundleVersionCode: 8'\n  m_Other: unchanged\n";

    fn input() -> Value {
        json!({"productName":"游戏: O'Brien \\ \"Deluxe\"","companyName":"公司 #1","bundleVersion":"3.4.5"})
    }

    fn setup() -> (tempfile::TempDir, Arc<Backend>) {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("game/Assets/Settings/Build Profiles")).unwrap();
        fs::create_dir_all(t.path().join("game/ProjectSettings")).unwrap();
        fs::write(
            t.path().join("game/ProjectSettings/ProjectSettings.asset"),
            GLOBAL,
        )
        .unwrap();
        let backend = Backend::new(t.path().join("config.json"), t.path().into());
        backend
            .handle_admin_project(
                "POST",
                "/api/config",
                json!({"projectPath":"game"}),
                "default",
            )
            .unwrap();
        let child = backend.project_runtime("default").unwrap();
        (t, child)
    }

    #[test]
    fn nested_yaml_round_trips_special_strings_and_preserves_other_lines() {
        let info = describe(PROFILE, GLOBAL, false);
        assert_eq!(info["productName"], "Original Game");
        assert_eq!(info["companyName"], "Original Company");
        assert_eq!(info["bundleVersion"], "2.0");
        let changed = update(&PROFILE.replace('\n', "\r\n"), &input()).unwrap();
        let info = describe(&changed, GLOBAL, false);
        for key in KEYS {
            assert_eq!(info[key], input()[key]);
        }
        assert!(changed
            .contains("    - line: '|   AndroidBundleVersionCode: 8'\r\n  m_Other: unchanged\r\n"));
        assert!(!changed.replace("\r\n", "").contains('\n'));
        // Both the outer asset and the reconstructed inner YAML remain valid.
        let outer = changed.lines().skip(1).collect::<Vec<_>>().join("\n");
        serde_yaml::from_str::<serde_yaml::Value>(&outer).unwrap();
        let inner = changed
            .lines()
            .filter_map(|l| scalar(l.trim().strip_prefix("- line:")?))
            .map(|l| l.strip_prefix("| ").unwrap().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        serde_yaml::from_str::<serde_yaml::Value>(&inner).unwrap();
    }

    #[test]
    fn empty_override_copies_complete_global_settings_into_profile() {
        let empty =
            "MonoBehaviour:\n  m_PlayerSettingsYaml:\n    m_Settings: []\n  m_Other: unchanged\n";
        assert_eq!(describe(empty, GLOBAL, false)["inherited"], true);
        assert_eq!(describe(empty, GLOBAL, false)["editable"], true);
        let initialized = initialize_override(empty, GLOBAL).unwrap();
        let changed = update(&initialized, &input()).unwrap();
        let info = describe(&changed, GLOBAL, false);
        for key in KEYS {
            assert_eq!(info[key], input()[key]);
        }
        assert!(changed.contains("AndroidBundleVersionCode: 42"));
        assert!(changed.ends_with("  m_Other: unchanged\n"));
        assert!(!changed.contains("%YAML"));
        assert!(!changed.contains("!u!129"));
        assert_eq!(
            describe("m_PlayerSettingsYaml: {}\n", GLOBAL, false)["editable"],
            false
        );
    }

    #[test]
    fn legacy_unity_reads_and_writes_project_settings_without_profiles() {
        let (t, backend) = setup();
        let mut body = input();
        body["assetPath"] = json!(PROJECT_DEFAULT_PROFILE);
        let read = backend.profile_settings_read(&body).unwrap();
        assert_eq!(read["fallback"], true);
        assert_eq!(read["companyName"], "Global Company");
        body["revision"] = read["revision"].clone();
        let written = backend.profile_settings_write(&body).unwrap();
        for key in KEYS {
            assert_eq!(written[key], body[key]);
        }
        let file = fs::read_to_string(t.path().join("game/ProjectSettings/ProjectSettings.asset"))
            .unwrap();
        assert!(file.contains("AndroidBundleVersionCode: 42"));
        let scan = backend.scan(&json!({"projectPath":"game"})).unwrap();
        for key in KEYS {
            assert_eq!(scan["profiles"][0][key], body[key]);
        }
        assert_eq!(backend.profile_settings_write(&body).unwrap_err().0, 409);
    }

    #[test]
    fn saving_inherited_profile_creates_override_without_changing_global_settings() {
        let (t, backend) = setup();
        let asset = "Assets/Settings/Build Profiles/Inherited.asset";
        let file = t.path().join("game").join(asset);
        let empty = "MonoBehaviour:\r\n  m_Name: BuildProfile\r\n  m_BuildTarget: 19\r\n  m_PlayerSettingsYaml:\r\n    m_Settings: []\r\n  m_Other: keep\r\n";
        fs::write(&file, empty).unwrap();
        let mut body = input();
        body["assetPath"] = json!(asset);
        let read = backend.profile_settings_read(&body).unwrap();
        assert_eq!(read["productName"], "Global Game");
        body["revision"] = read["revision"].clone();
        let result = backend.profile_settings_write(&body).unwrap();
        assert_eq!(result["inherited"], false);
        for key in KEYS {
            assert_eq!(result[key], body[key]);
        }
        assert_eq!(
            fs::read_to_string(t.path().join("game/ProjectSettings/ProjectSettings.asset"))
                .unwrap(),
            GLOBAL
        );
        let changed = fs::read_to_string(file).unwrap();
        assert!(!changed.replace("\r\n", "").contains('\n'));
        assert!(changed.contains("AndroidBundleVersionCode: 42"));
    }

    #[test]
    fn profile_edits_only_change_selected_asset_and_block_busy_or_invalid_requests() {
        let (t, backend) = setup();
        let asset = "Assets/Settings/Build Profiles/Windows.asset";
        let file = t.path().join("game").join(asset);
        fs::write(&file, PROFILE).unwrap();
        let mut body = input();
        body["assetPath"] = json!(asset);
        body["revision"] = backend.profile_settings_read(&body).unwrap()["revision"].clone();
        backend.acquire().unwrap();
        assert_eq!(backend.profile_settings_write(&body).unwrap_err().0, 409);
        backend.release();
        backend.profile_settings_write(&body).unwrap();
        assert_eq!(
            fs::read_to_string(t.path().join("game/ProjectSettings/ProjectSettings.asset"))
                .unwrap(),
            GLOBAL
        );
        let scan = backend.scan(&json!({"projectPath":"game"})).unwrap();
        for key in KEYS {
            assert_eq!(scan["profiles"][0][key], body[key]);
        }
        for asset in [
            "../outside.asset",
            "ProjectSettings/ProjectSettings.asset",
            "C:/outside.asset",
        ] {
            assert!(backend
                .profile_settings_read(&json!({"assetPath":asset}))
                .is_err());
        }
    }

    #[test]
    fn plain_subasset_and_rejected_fields_do_not_damage_yaml() {
        let plain = "MonoBehaviour:\n  m_Name: BuildProfile\n--- !u!129 &2\nPlayerSettings:\n  productName: Game\n  companyName: Studio\n  bundleVersion: 1.0\n  other: keep";
        let changed = update(plain, &input()).unwrap();
        for key in KEYS {
            assert_eq!(describe(&changed, "", false)[key], input()[key]);
        }
        assert!(changed.ends_with("  other: keep"));
        assert!(update(&format!("{plain}\n  productName: Duplicate"), &input()).is_err());
        assert!(update("productName: Game\n", &input()).is_err());
        let mut bad = input();
        bad["companyName"] = json!("bad\nline");
        assert!(update(plain, &bad).is_err());
    }
}
