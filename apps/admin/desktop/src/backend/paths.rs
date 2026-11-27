//! Common project directories, resolved with the same bases as their owning operations.
use super::*;

impl Backend {
    pub(super) fn application_paths(&self, body: &Value) -> Result<Value, Fault> {
        let cfg = self.config();
        if valstr(&cfg, "projectPath").trim().is_empty() {
            return Ok(json!({"paths":[],"profiles":[]}));
        }
        let resolved = self.resolve(&valstr(&cfg, "projectPath"));
        let project = fs::canonicalize(&resolved).unwrap_or(resolved);
        let scan = self.scan(&json!({"projectPath":project,"profileDir":cfg["profileDir"]}))?;
        let profiles = scan["profiles"].as_array().unwrap();
        let requested = valstr(body, "assetPath");
        let snapshot: Value = fs::read_to_string(project.join(".buildtool/application-paths.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let selected = profiles
            .iter()
            .find(|p| p["assetPath"] == requested)
            .or_else(|| {
                if requested.is_empty() {
                    profiles
                        .iter()
                        .find(|p| p["assetPath"] == snapshot["profilePath"])
                        .or(profiles.first())
                } else {
                    None
                }
            })
            .ok_or((404, "Profile 不存在，请重新扫描".into()))?;
        let actual = snapshot["profilePath"] == selected["assetPath"]
            && snapshot["companyName"] == selected["companyName"]
            && snapshot["productName"] == selected["productName"]
            && snapshot["dataPath"].as_str().is_some_and(|s| {
                fs::canonicalize(s).ok() == fs::canonicalize(project.join("Assets")).ok()
            });
        let mut paths = Vec::new();
        let mut add = |id: &str, path: Option<PathBuf>, source: &str, note: &str, file: bool| {
            let exists = path
                .as_ref()
                .is_some_and(|p| if file { p.is_file() } else { p.is_dir() });
            paths.push(json!({"id":id,"name":format!("Application.{id}"),"path":path,"exists":exists,"source":source,"note":note,"file":file}));
        };
        for (id, fallback) in [
            ("dataPath", Some(project.join("Assets"))),
            (
                "streamingAssetsPath",
                Some(project.join("Assets/StreamingAssets")),
            ),
            ("persistentDataPath", None),
            ("temporaryCachePath", None),
            ("consoleLogPath", None),
        ] {
            let sampled = if actual {
                snapshot[id]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
            } else {
                None
            };
            if sampled.is_some() {
                add(id, sampled, "Unity 编辑器实测", "", id == "consoleLogPath");
            } else if fallback.is_some() {
                add(id, fallback, "编辑器路径", "根据工程目录解析", false);
            } else if id == "persistentDataPath" && cfg!(windows) {
                let company = valstr(selected, "companyName");
                let product = valstr(selected, "productName");
                let valid = |s: &str| {
                    !s.is_empty()
                        && s != "."
                        && s != ".."
                        && !s
                            .chars()
                            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
                };
                let path = if valid(&company) && valid(&product) {
                    std::env::var_os("USERPROFILE").map(|p| {
                        PathBuf::from(p)
                            .join("AppData/LocalLow")
                            .join(company)
                            .join(product)
                    })
                } else {
                    None
                };
                add(
                    id,
                    path,
                    "Windows 本机预估",
                    "按 Company Name / Product Name 推算，Unity 实测值优先",
                    false,
                );
            } else {
                add(
                    id,
                    None,
                    "待 Unity 采集",
                    "请在本机 Unity 中打开对应 Profile；临时缓存目录不能仅凭项目名推算",
                    id == "consoleLogPath",
                );
            }
        }
        Ok(
            json!({"paths":paths,"profiles":profiles.iter().map(|p|json!({"assetPath":p["assetPath"],"name":p["name"]})).collect::<Vec<_>>(),"assetPath":selected["assetPath"],"capturedAt":if actual {snapshot["capturedAt"].clone()} else {Value::Null},"context":"本机 Unity Editor；Player 和移动设备的路径可能不同"}),
        )
    }

    pub(super) fn open_application_path(&self, body: &Value) -> Result<Value, Fault> {
        let data = self.application_paths(body)?;
        let id = valstr(body, "id");
        let entry = data["paths"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .ok_or((404, "Application 路径不存在".into()))?;
        if entry["exists"] != true {
            return Err((404, "目录尚未生成或尚未由 Unity 采集".into()));
        }
        let path = PathBuf::from(valstr(entry, "path"));
        let directory = if entry["file"] == true {
            path.parent()
                .ok_or((400, "日志目录无效".into()))?
                .to_path_buf()
        } else {
            path
        };
        open::that(&directory).map_err(|e| (500, e.to_string()))?;
        Ok(json!({"ok":true,"opened":directory}))
    }

    pub(super) fn project_paths(&self) -> Value {
        let cfg = self.config();
        let mut paths = Vec::new();
        let mut add = |id: String, name: String, path: PathBuf| {
            paths.push(json!({"id":id,"name":name,"path":path,"exists":path.is_dir()}));
        };
        let project_path = valstr(&cfg, "projectPath");
        if !project_path.trim().is_empty() {
            let project = self.resolve(&project_path);
            add("project".into(), "工程根目录".into(), project.clone());
            add("assets".into(), "Assets".into(), project.join("Assets"));
            add(
                "settings".into(),
                "ProjectSettings".into(),
                project.join("ProjectSettings"),
            );
            add(
                "packages".into(),
                "Packages".into(),
                project.join("Packages"),
            );
            let profiles = valstr(&cfg, "profileDir");
            add(
                "profiles".into(),
                "Profile 目录".into(),
                abs(
                    &project,
                    if profiles.is_empty() {
                        "Assets/Settings/Build Profiles"
                    } else {
                        &profiles
                    },
                ),
            );
            // Disabled projects still have local directories administrators can open.
            let mut local_cfg = cfg.clone();
            local_cfg["enabled"] = json!(true);
            let roots = crate::http::download::roots(&local_cfg, &self.runtime_root);
            if let Some(base) = roots.first() {
                add(
                    "builds".into(),
                    "构建输出目录".into(),
                    base.directory.clone(),
                );
                let success = valstr(&cfg, "successDir");
                add(
                    "success".into(),
                    "成功归档目录".into(),
                    abs(
                        &base.directory,
                        if success.is_empty() {
                            "构建成功"
                        } else {
                            &success
                        },
                    ),
                );
            }
            for root in roots.into_iter().skip(1) {
                add(root.id, root.name, root.directory);
            }
            let check_project = valstr(&cfg["check"], "projectPath");
            let check_project = if check_project.trim().is_empty() {
                project
            } else {
                self.resolve(&check_project)
            };
            add(
                "check".into(),
                "环境检测结果".into(),
                check_project.join(".buildtool/check-results"),
            );
        }
        for (i, node) in cfg["vcs"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
        {
            let path = valstr(node, "path");
            if !path.trim().is_empty() {
                let name = valstr(node, "name");
                add(
                    format!("repo-{i}"),
                    format!("仓库 · {}", if name.is_empty() { &path } else { &name }),
                    self.resolve(&path),
                );
            }
        }
        json!({"paths":paths})
    }

    pub(super) fn open_project_path(&self, id: &str) -> Result<Value, Fault> {
        let paths = self.project_paths();
        let entry = paths["paths"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .ok_or((404, "项目快捷路径不存在，请刷新后重试".into()))?;
        let path = PathBuf::from(valstr(entry, "path"));
        if !path.is_dir() {
            return Err((404, format!("目录尚未生成或已移除: {}", path.display())));
        }
        open::that(&path).map_err(|e| (500, e.to_string()))?;
        Ok(json!({"ok":true,"opened":path}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_paths_use_editor_snapshot_and_reject_stale_identity() {
        let t = tempfile::tempdir().unwrap();
        let project = t.path().join("game");
        fs::create_dir_all(project.join("Assets/StreamingAssets")).unwrap();
        fs::create_dir_all(project.join("ProjectSettings")).unwrap();
        fs::create_dir_all(project.join(".buildtool")).unwrap();
        let settings = project.join("ProjectSettings/ProjectSettings.asset");
        fs::write(
            &settings,
            "PlayerSettings:\n  companyName: Studio\n  productName: Game\n  bundleVersion: 1.0\n",
        )
        .unwrap();
        let persistent = t.path().join("save-data");
        fs::create_dir_all(&persistent).unwrap();
        fs::write(project.join(".buildtool/application-paths.json"), json!({
            "profilePath":PROJECT_DEFAULT_PROFILE,"companyName":"Studio","productName":"Game",
            "dataPath":project.join("Assets"),"streamingAssetsPath":project.join("Assets/StreamingAssets"),
            "persistentDataPath":persistent,"temporaryCachePath":t.path().join("UnityCache"),"capturedAt":"2026-11-13T00:00:00Z"
        }).to_string()).unwrap();
        let root = Backend::new(t.path().join("config.json"), t.path().into());
        root.handle_admin_project(
            "POST",
            "/api/config",
            json!({"projectPath":"game"}),
            "default",
        )
        .unwrap();
        let backend = root.project_runtime("default").unwrap();
        let data = backend.application_paths(&json!({})).unwrap();
        let entries = data["paths"].as_array().unwrap();
        let saved = entries
            .iter()
            .find(|p| p["id"] == "persistentDataPath")
            .unwrap();
        assert_eq!(PathBuf::from(valstr(saved, "path")), persistent);
        assert_eq!(saved["source"], "Unity 编辑器实测");
        assert_eq!(saved["exists"], true);
        fs::write(&settings, "PlayerSettings:\n  companyName: NewStudio\n  productName: NewGame\n  bundleVersion: 1.0\n").unwrap();
        let changed = backend.application_paths(&json!({})).unwrap();
        assert!(changed["capturedAt"].is_null());
        for entry in changed["paths"].as_array().unwrap() {
            assert_ne!(entry["source"], "Unity 编辑器实测");
            if entry["id"] == "temporaryCachePath" {
                assert!(entry["path"].is_null());
            }
        }
        assert!(backend
            .application_paths(&json!({"assetPath":"Assets/Missing.asset"}))
            .is_err());
        assert!(backend
            .open_application_path(&json!({"id":"temporaryCachePath"}))
            .is_err());
    }

    #[test]
    fn shortcuts_resolve_project_output_check_and_repository_bases() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("game/Assets")).unwrap();
        let backend = Backend::new(t.path().join("config.json"), t.path().into());
        let child = backend.project_runtime("default").unwrap();
        child.save(json!({"id":"default","slug":"default","projectPath":"game","outputBase":"Output","successDir":"Archives","profileDir":"Profiles","check":{"projectPath":"checks"},"vcs":{"nodes":[{"name":"Source","path":"repo"}]},"buildNodes":[{"name":"Win","customBuildDir":"Custom"}]})).unwrap();
        let data = child.project_paths();
        let entries = data["paths"].as_array().unwrap();
        let get = |id: &str| entries.iter().find(|p| p["id"] == id).unwrap();
        for (id, relative) in [
            ("project", "game"),
            ("assets", "game/Assets"),
            ("profiles", "game/Profiles"),
            ("builds", "game/Output"),
            ("success", "game/Output/Archives"),
            ("check", "checks/.buildtool/check-results"),
            ("repo-0", "repo"),
        ] {
            assert_eq!(
                PathBuf::from(valstr(get(id), "path")),
                t.path().join(relative)
            );
        }
        assert_eq!(get("assets")["exists"], true);
        assert_eq!(get("success")["exists"], false);
        assert!(entries
            .iter()
            .any(|p| PathBuf::from(valstr(p, "path")) == t.path().join("game/Output/Custom")));
        assert_eq!(child.open_project_path("success").unwrap_err().0, 404);
        assert_eq!(child.open_project_path("unknown").unwrap_err().0, 404);
    }

    #[test]
    fn shortcuts_are_scoped_to_the_requested_project() {
        let t = tempfile::tempdir().unwrap();
        let backend = Backend::new(t.path().join("config.json"), t.path().into());
        backend.save(json!({"projects":[{"id":"a","slug":"a","projectPath":"a"},{"id":"b","slug":"b","projectPath":"b"}]})).unwrap();
        for id in ["a", "b"] {
            let data = backend
                .handle_admin_project("GET", "/api/project-paths", json!({}), id)
                .unwrap();
            for entry in data["paths"].as_array().unwrap() {
                assert!(PathBuf::from(valstr(entry, "path")).starts_with(t.path().join(id)));
            }
        }
    }
}
