//! Common project directories, resolved with the same bases as their owning operations.
use super::*;

impl Backend {
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
