//! Project configuration migration, isolated runtimes and a shared FIFO scheduler.
use super::*;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Condvar, Weak},
};

#[derive(Default)]
pub(super) struct Scheduler {
    pub(super) state: Mutex<(Option<String>, VecDeque<String>)>,
    pub(super) wake: Condvar,
}

pub(super) fn migrate(mut cfg: Value) -> Value {
    if cfg["schemaVersion"] == 2 && cfg["projects"].is_array() {
        return cfg;
    }
    let ai = cfg.get("ai").cloned();
    let port = cfg.get("serverPort").cloned().unwrap_or(json!(50345));
    if !cfg.is_object() {
        cfg = json!({});
    }
    cfg["id"] = json!("default");
    cfg["slug"] = json!("default");
    cfg["name"] = json!("默认项目");
    cfg["enabled"] = json!(true);
    cfg.as_object_mut().unwrap().remove("ai");
    let mut root =
        json!({"schemaVersion":2,"server":{"port":port,"host":"127.0.0.1"},"projects":[cfg]});
    if let Some(ai) = ai {
        root["ai"] = ai;
    }
    root
}

fn validate(c: &Value) -> Result<(), String> {
    let projects = c["projects"].as_array().ok_or("projects 必须是数组")?;
    let mut ids = HashSet::new();
    let mut slugs = HashSet::new();
    for p in projects {
        let id = valstr(p, "id");
        let slug = valstr(p, "slug");
        if id.is_empty() || !ids.insert(id) {
            return Err("项目 ID 为空或重复".into());
        }
        if slug.is_empty()
            || slug.len() > 64
            || !slug
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !slugs.insert(slug)
        {
            return Err("项目链接标识无效或重复，仅允许字母、数字、下划线、短横线".into());
        }
        if !p.is_object() {
            return Err("项目必须是对象".into());
        }
    }
    Ok(())
}

impl Backend {
    fn validate_roots(&self, c: &Value) -> Result<(), String> {
        validate(c)?;
        let mut all: Vec<(String, PathBuf)> = Vec::new();
        for p in c["projects"].as_array().unwrap() {
            let id = valstr(p, "id");
            let project = self.resolve(&valstr(p, "projectPath"));
            if valstr(p, "projectPath").is_empty() {
                if !valstr(p, "outputBase").is_empty() && valstr(p, "outputBase") != "Builds" {
                    return Err("请先配置工程路径，再设置输出目录".into());
                }
                continue;
            }
            let mut paths = vec![project.clone()];
            let output = valstr(p, "outputBase");
            let base = abs(&project, if output.is_empty() { "Builds" } else { &output });
            paths.push(base.clone());
            let custom_path = |raw: &str| {
                if Path::new(raw).is_absolute() {
                    PathBuf::from(raw)
                } else if raw
                    .replace('\\', "/")
                    .to_ascii_lowercase()
                    .starts_with("builds/")
                {
                    base.parent().unwrap_or(&project).join(raw)
                } else {
                    base.join(raw)
                }
            };
            for raw in [
                valstr(p, "customBuildDir"),
                valstr(&p["sign"], "customBuildDir"),
            ] {
                if !raw.is_empty() {
                    paths.push(custom_path(&raw));
                }
            }
            for n in p["buildNodes"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(p["queue"].as_array().into_iter().flatten())
                .chain(
                    p["profileConfigs"]
                        .as_object()
                        .into_iter()
                        .flat_map(|m| m.values()),
                )
            {
                for raw in [
                    valstr(n, "customBuildDir"),
                    valstr(&n["sign"], "customBuildDir"),
                ] {
                    if !raw.is_empty() {
                        paths.push(custom_path(&raw));
                    }
                }
            }
            for path in paths {
                let path = fs::canonicalize(&path).unwrap_or_else(|_| {
                    let mut clean = PathBuf::new();
                    for part in path.components() {
                        if part == std::path::Component::ParentDir {
                            clean.pop();
                        } else if part != std::path::Component::CurDir {
                            clean.push(part.as_os_str());
                        }
                    }
                    clean
                });
                #[cfg(windows)]
                let path = PathBuf::from(
                    path.to_string_lossy()
                        .to_ascii_lowercase()
                        .trim_start_matches(r"\\?\")
                        .to_string(),
                );
                if all.iter().any(|(owner, other)| {
                    owner != &id && (path.starts_with(other) || other.starts_with(&path))
                }) {
                    return Err("不同项目的工程或输出目录不能相同、互相包含或重叠".into());
                }
                all.push((id.clone(), path));
            }
        }
        Ok(())
    }
    pub fn project_by_slug(&self, slug: &str) -> Result<(String, Value), Fault> {
        let cfg = self.config();
        let p = cfg["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["slug"] == slug && p["enabled"] != false)
            .cloned()
            .ok_or((404, "项目不存在或已停用".into()))?;
        Ok((valstr(&p, "id"), p))
    }
    pub(super) fn project_runtime(self: &Arc<Self>, id: &str) -> Result<Arc<Self>, Fault> {
        let p = self.config()["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["id"] == id)
            .cloned()
            .ok_or((404, "项目不存在".into()))?;
        let mut children = self.children.lock().unwrap();
        if let Some(child) = children.get(id) {
            return Ok(child.clone());
        }
        let child = Arc::new(Self {
            config_path: self.config_path.clone(),
            runtime_root: self.runtime_root.clone(),
            config: Mutex::new(p),
            tx: Mutex::new(vec![]),
            busy: Mutex::new(false),
            stop: Mutex::new(false),
            job: Mutex::new(Value::Null),
            check: Mutex::new(Value::Null),
            vcs_job: Mutex::new(Value::Null),
            project_id: Some(id.to_owned()),
            parent: Some(Arc::downgrade(self)),
            children: Mutex::new(HashMap::new()),
            monitor: super::monitor::Monitor::default(),
            scheduler: self.scheduler.clone(),
        });
        children.insert(id.to_owned(), child.clone());
        Ok(child)
    }
    pub fn handle_admin_project(
        self: &Arc<Self>,
        method: &str,
        path: &str,
        body: Value,
        id: &str,
    ) -> Result<Value, Fault> {
        let child = self.project_runtime(id)?;
        if method == "POST" && path == "/api/config" {
            let mut body = body;
            if child.busy() {
                return Err((409, "任务运行或排队期间不能修改项目设置".into()));
            }
            if !body.is_object() {
                return Err((400, "配置必须是对象".into()));
            }
            let old = child.config();
            let mut merged = old.clone();
            for (k, v) in body.as_object().unwrap() {
                merged[k] = v.clone();
            }
            body = merged;
            for key in ["id", "slug", "name", "enabled", "lastVersionCode"] {
                body[key] = old[key].clone();
            }
            child.save(body).map_err(|e| (400, e))?;
            return Ok(json!({"ok":true}));
        }
        let mut result = child.handle(method, path, body)?;
        if method == "GET" && path == "/api/status" {
            result["projectId"] = json!(id);
            result["scheduler"] = self.scheduler_status();
        }
        Ok(result)
    }
    pub fn handle_user_project(
        self: &Arc<Self>,
        method: &str,
        path: &str,
        body: Value,
        slug: &str,
    ) -> Result<Value, Fault> {
        let (id, p) = self.project_by_slug(slug)?;
        let child = self.project_runtime(&id)?;
        let mut result = child.handle(method, path, body)?;
        if result.is_object() {
            result["scheduler"] = self.scheduler_status();
            result["queuePosition"] = json!(self
                .scheduler
                .state
                .lock()
                .unwrap()
                .1
                .iter()
                .position(|x| x == &id)
                .map(|x| x + 1));
            result["project"] = json!({"id":id,"slug":slug,"name":p["name"]});
        }
        Ok(result)
    }
    pub fn scheduler_status(&self) -> Value {
        let state = self.scheduler.state.lock().unwrap();
        json!({"activeProjectId":state.0,"pendingProjectIds":state.1})
    }
    pub(super) fn reserve_build(&self) -> Result<(), Fault> {
        let mut state = self.scheduler.state.lock().unwrap();
        let mut busy = self.busy.lock().unwrap();
        if *busy {
            return Err((409, "该项目已有运行中或等待中的任务".into()));
        }
        *busy = true;
        *self.stop.lock().unwrap() = false;
        state.1.push_back(self.scheduler_key());
        self.scheduler.wake.notify_all();
        Ok(())
    }
    pub(super) fn scheduler_key(&self) -> String {
        self.project_id.clone().unwrap_or_else(|| "__root__".into())
    }
    pub(super) fn wait_turn(&self) -> bool {
        let key = self.scheduler_key();
        let mut state = self.scheduler.state.lock().unwrap();
        loop {
            if self.cancelled() {
                state.1.retain(|s| s != &key);
                self.scheduler.wake.notify_all();
                return false;
            }
            if state.0.is_none() && state.1.front() == Some(&key) {
                state.1.pop_front();
                state.0 = Some(key);
                return true;
            }
            state = self
                .scheduler
                .wake
                .wait_timeout(state, Duration::from_millis(100))
                .unwrap()
                .0;
        }
    }
    pub(super) fn save_project(&self, mut c: Value) -> Result<(), String> {
        let parent = self
            .parent
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or("项目管理器已关闭")?;
        let id = self.project_id.as_ref().unwrap();
        let mut current = parent.config.lock().unwrap();
        let mut root = current.clone();
        if let Some(ai) = c.as_object_mut().unwrap().remove("ai") {
            root["ai"] = ai;
        }
        let projects = root["projects"].as_array_mut().ok_or("项目配置无效")?;
        let p = projects
            .iter_mut()
            .find(|p| p["id"] == *id)
            .ok_or("项目已删除")?;
        *p = c.clone();
        parent.validate_roots(&root)?;
        parent.persist(&root)?;
        *self.config.lock().unwrap() = c;
        *current = root;
        Ok(())
    }
    pub(super) fn save_root(&self, mut c: Value) -> Result<(), String> {
        self.validate_roots(&c)?;
        c["schemaVersion"] = json!(2);
        let mut children = self.children.lock().unwrap();
        let mut current = self.config.lock().unwrap();
        for child in children.values() {
            let id = child.project_id.as_ref().unwrap();
            let entry = c["projects"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|p| p["id"] == *id);
            if child.busy()
                && entry
                    .as_ref()
                    .map(|p| p["enabled"] == false)
                    .unwrap_or(true)
            {
                return Err("不能删除或停用有排队/运行任务的项目".into());
            }
            if let Some(p) = entry {
                if let Some(old) = current["projects"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|p| p["id"] == *id)
                {
                    p["lastVersionCode"] = old["lastVersionCode"].clone();
                    if child.busy() {
                        for (k, v) in old.as_object().unwrap() {
                            if !["name", "slug", "enabled"].contains(&k.as_str()) && p[k] != *v {
                                return Err("任务运行或排队期间不能修改项目设置".into());
                            }
                        }
                    }
                }
            }
        }
        self.persist(&c)?;
        for (id, child) in children.iter() {
            if let Some(p) = c["projects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == *id)
            {
                *child.config.lock().unwrap() = p.clone();
            }
        }
        children.retain(|id, _| {
            c["projects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["id"] == *id)
        });
        *current = c;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_preserves_legacy_nodes_and_counters() {
        let root = migrate(
            json!({"projectPath":"A","lastVersionCode":12,"buildNodes":[{"id":"n"}],"ai":{"enabled":true}}),
        );
        assert_eq!(root["schemaVersion"], 2);
        assert_eq!(root["projects"][0]["lastVersionCode"], 12);
        assert_eq!(root["projects"][0]["buildNodes"][0]["id"], "n");
        assert!(root["projects"][0]["ai"].is_null());
        assert_eq!(migrate(root.clone()), root);
    }
    #[test]
    fn fifo_cancel_counter_isolation_and_scoped_nodes() {
        let t = tempfile::tempdir().unwrap();
        let root = Backend::new(t.path().join("config.json"), t.path().into());
        root.save(json!({"schemaVersion":2,"projects":[{"id":"a","slug":"a","name":"A","projectPath":"A","lastVersionCode":4,"buildNodes":[{"id":"a-node","profileAssetPath":"Assets/a.asset"}]},{"id":"b","slug":"b","name":"B","projectPath":"B","lastVersionCode":9}]})).unwrap();
        let a = root.project_runtime("a").unwrap();
        let b = root.project_runtime("b").unwrap();
        a.reserve_build().unwrap();
        b.reserve_build().unwrap();
        assert_eq!(a.reserve_build().unwrap_err().0, 409);
        assert_eq!(
            root.scheduler_status()["pendingProjectIds"],
            json!(["a", "b"])
        );
        assert!(a.wait_turn());
        assert_eq!(root.scheduler_status()["activeProjectId"], "a");
        b.stop();
        assert!(!b.wait_turn());
        b.release();
        a.release();
        b.reserve_build().unwrap();
        assert!(b.wait_turn());
        b.release();
        let mut cfg = a.config();
        cfg["lastVersionCode"] = json!(5);
        a.save(cfg).unwrap();
        let mut stale = root.config();
        stale["projects"][0]["lastVersionCode"] = json!(1);
        root.save(stale).unwrap();
        assert_eq!(root.config()["projects"][0]["lastVersionCode"], 5);
        assert_eq!(b.config()["lastVersionCode"], 9);
        assert_eq!(
            root.handle_user_project(
                "POST",
                "/api/user/build",
                json!({"nodeIds":["a-node"]}),
                "b"
            )
            .unwrap_err()
            .0,
            400
        );
        let mut overlapping = root.config();
        overlapping["projects"][1]["outputBase"] = json!(t.path().join("A/Builds"));
        assert!(root.save(overlapping).is_err());
        let recv = root.subscribe();
        a.emit(json!({"type":"test"}));
        assert_eq!(recv.recv().unwrap()["projectId"], "a");
        let cfg = root.config();
        let loaded = Backend::new(t.path().join("config.json"), t.path().into());
        assert_eq!(loaded.config(), cfg);
    }
    #[test]
    fn migration_backups_legacy_file() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("config.json");
        fs::write(&path, r#"{"projectPath":"old","lastVersionCode":42}"#).unwrap();
        let b = Backend::new(path.clone(), t.path().into());
        assert_eq!(b.config()["projects"][0]["lastVersionCode"], 42);
        assert!(path.with_extension("v1.backup.json").is_file());
    }
    #[test]
    fn rejects_duplicate_or_unsafe_slugs() {
        assert!(validate(&json!({"projects":[{"id":"a","slug":"../a"}]})).is_err());
        assert!(
            validate(&json!({"projects":[{"id":"a","slug":"a"},{"id":"b","slug":"a"}]})).is_err()
        );
    }
}
