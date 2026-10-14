mod build;
mod check;
mod projects;
mod monitor;
#[cfg(test)]
mod tests;
mod vcs;
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

const TARGETS: &[(i64, &str)] = &[
    (5, "Windows (x86)"),
    (19, "Windows (x64)"),
    (13, "Android"),
    (9, "iOS"),
    (4, "macOS"),
];
fn target_name(t: i64) -> String {
    TARGETS
        .iter()
        .find(|x| x.0 == t)
        .map(|x| x.1.to_string())
        .unwrap_or_else(|| format!("BuildTarget {t}"))
}
fn valstr(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_owned()
}
fn abs(root: &Path, p: &str) -> PathBuf {
    let p = PathBuf::from(p);
    if p.is_absolute() {
        p
    } else {
        root.join(p)
    }
}
fn project_valid(p: &Path) -> bool {
    p.join("Assets").is_dir() && p.join("ProjectSettings").is_dir()
}
type Fault = (u16, String);
pub struct Backend {
    config_path: PathBuf,
    runtime_root: PathBuf,
    config: Mutex<Value>,
    tx: Mutex<Vec<Sender<Value>>>,
    busy: Mutex<bool>,
    stop: Mutex<bool>,
    job: Mutex<Value>,
    check: Mutex<Value>,
    vcs_job: Mutex<Value>,
    pub(super) project_id: Option<String>,
    pub(super) parent: Option<std::sync::Weak<Backend>>,
    pub(super) children: Mutex<std::collections::HashMap<String, Arc<Backend>>>,
    monitor: monitor::Monitor,
    scheduler: Arc<projects::Scheduler>,
}
impl Backend {
    pub fn new(config_path: PathBuf, runtime_root: PathBuf) -> Arc<Self> {
        let cfg = match fs::read_to_string(&config_path) {
            Ok(t) => serde_json::from_str(&t).unwrap_or_else(|e| {
                eprintln!("配置解析失败: {e}");
                json!({})
            }),
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("配置读取失败: {e}");
                }
                json!({})
            }
        };
        let migrated = projects::migrate(cfg.clone());
        if migrated != cfg && config_path.is_file() {
            let backup = config_path.with_extension("v1.backup.json");
            if !backup.exists() {
                let _ = fs::copy(&config_path, backup);
            }
        }
        let cfg = migrated;
        let backend = Arc::new(Self {
            config_path,
            runtime_root,
            config: Mutex::new(cfg),
            tx: Mutex::new(vec![]),
            busy: Mutex::new(false),
            stop: Mutex::new(false),
            job: Mutex::new(Value::Null),
            check: Mutex::new(Value::Null),
            vcs_job: Mutex::new(Value::Null),
            project_id: None,
            parent: None,
            children: Mutex::new(std::collections::HashMap::new()),
            monitor: monitor::Monitor::default(),
            scheduler: Arc::new(projects::Scheduler::default()),
        });
        if let Err(e) = backend.persist(&backend.config()) {
            eprintln!("配置迁移保存失败: {e}");
        }
        backend
    }
    pub fn config(&self) -> Value {
        let mut cfg = self.config.lock().unwrap().clone();
        if let Some(parent) = self.parent.as_ref().and_then(std::sync::Weak::upgrade) {
            cfg["ai"] = parent.config()["ai"].clone();
        }
        cfg
    }
    pub fn subscribe(&self) -> Receiver<Value> {
        let (tx, rx) = mpsc::channel();
        self.tx.lock().unwrap().push(tx);
        rx
    }
    pub fn busy(&self) -> bool {
        if self.project_id.is_none() {
            let state = self.scheduler.state.lock().unwrap();
            return state.0.is_some() || !state.1.is_empty();
        }
        *self.busy.lock().unwrap()
    }
    pub fn stop(&self) {
        *self.stop.lock().unwrap() = true;
        if self.project_id.is_none() {
            for child in self.children.lock().unwrap().values() {
                child.stop();
            }
        }
        self.scheduler.wake.notify_all();
    }
    pub fn wait_idle(&self, timeout: Duration) {
        let start = Instant::now();
        while self.busy() && start.elapsed() < timeout {
            thread::sleep(Duration::from_millis(40));
        }
    }
    pub(super) fn cancelled(&self) -> bool {
        *self.stop.lock().unwrap()
    }
    pub(super) fn runtime_root(&self) -> &Path {
        &self.runtime_root
    }
    pub(super) fn resolve(&self, p: &str) -> PathBuf {
        abs(&self.runtime_root, p)
    }
    pub(super) fn acquire(&self) -> Result<(), Fault> {
        let mut state = self.scheduler.state.lock().unwrap();
        let mut busy = self.busy.lock().unwrap();
        if *busy || state.0.is_some() || !state.1.is_empty() {
            return Err((409, "已有构建、检测或仓库更新任务运行中".into()));
        }
        *busy = true;
        *self.stop.lock().unwrap() = false;
        state.0 = Some(self.scheduler_key());
        Ok(())
    }
    pub(super) fn release(&self) {
        let mut state = self.scheduler.state.lock().unwrap();
        let key = self.scheduler_key();
        if state.0.as_ref() == Some(&key) {
            state.0 = None;
        }
        state.1.retain(|p| p != &key);
        *self.busy.lock().unwrap() = false;
        self.scheduler.wake.notify_all();
    }
    pub(crate) fn emit(&self, mut e: Value) {
        if let Some(id) = &self.project_id {
            e["projectId"] = json!(id);
        }
        self.monitor.record(&e);
        self.tx
            .lock()
            .unwrap()
            .retain(|s| s.send(e.clone()).is_ok());
        if let Some(parent) = self.parent.as_ref().and_then(std::sync::Weak::upgrade) {
            parent.emit(e);
        }
    }
    fn persist(&self, c: &Value) -> Result<(), String> {
        if let Some(p) = self.config_path.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let tmp = self.config_path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec_pretty(c).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(tmp, &self.config_path).map_err(|e| e.to_string())
    }
    pub(super) fn save(&self, c: Value) -> Result<(), String> {
        if !c.is_object() {
            return Err("配置必须是对象".into());
        }
        if self.project_id.is_some() {
            self.save_project(c)
        } else {
            self.save_root(c)
        }
    }
    pub(super) fn run_process(
        &self,
        exe: &str,
        args: &[String],
        cwd: &Path,
        log: &Path,
        event: &str,
        index: usize,
        timeout: Duration,
    ) -> Result<i32, String> {
        if self.cancelled() {
            return Err("cancelled".into());
        }
        if let Some(p) = log.parent() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let file = Arc::new(Mutex::new(
            fs::File::create(log).map_err(|e| e.to_string())?,
        ));
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "Never");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd.spawn().map_err(|e| format!("无法启动 {exe}: {e}"))?;
        let (tx, rx) = mpsc::channel::<String>();
        let mut readers = vec![];
        for mut pipe in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
            Box::new(child.stderr.take().unwrap()) as Box<dyn Read + Send>,
        ] {
            let tx = tx.clone();
            let file = file.clone();
            readers.push(thread::spawn(move || {
                let mut buf = [0; 8192];
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let _ = file.lock().unwrap().write_all(&buf[..n]);
                            let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
                        }
                    }
                }
            }));
        }
        drop(tx);
        self.monitor.pids.lock().unwrap().push(child.id());
        let started = Instant::now();
        let result = loop {
            for text in rx.try_iter() {
                self.emit(json!({"type":event,"index":index,"text":text}));
            }
            if self.cancelled() || started.elapsed() > timeout {
                #[cfg(windows)]
                {
                    let mut killer = Command::new("taskkill");
                    killer
                        .args(["/PID", &child.id().to_string(), "/T", "/F"])
                        .stdout(Stdio::null())
                        .stderr(Stdio::null());
                    use std::os::windows::process::CommandExt;
                    killer.creation_flags(0x08000000);
                    let _ = killer.status();
                }
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                break Err(if self.cancelled() {
                    "cancelled"
                } else {
                    "任务超时"
                }
                .into());
            }
            match child.try_wait() {
                Ok(Some(s)) => break Ok(s.code().unwrap_or(-1)),
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(e.to_string());
                }
                Ok(None) => thread::sleep(Duration::from_millis(40)),
            }
        };
        // A spawned descendant may retain stdout after its parent exits.
        self.monitor.pids.lock().unwrap().retain(|pid| *pid != child.id());
        // Never let an inherited pipe keep the scheduler busy indefinitely.
        let drain_started = Instant::now();
        while readers.iter().any(|r| !r.is_finished())
            && drain_started.elapsed() < Duration::from_secs(2)
        {
            for text in rx.try_iter() {
                self.emit(json!({"type":event,"index":index,"text":text}));
            }
            thread::sleep(Duration::from_millis(20));
        }
        let drained = readers.iter().all(|r| r.is_finished());
        for r in readers {
            if r.is_finished() {
                let _ = r.join();
            }
        }
        if !drained && result.is_ok() {
            return Err("子进程退出后输出管道仍被后台进程占用".into());
        }
        for text in rx.try_iter() {
            self.emit(json!({"type":event,"index":index,"text":text}));
        }
        result
    }
    fn nodes(&self) -> Vec<Value> {
        let c = self.config();
        let mut nodes = if let Some(a) = c["buildNodes"].as_array() {
            a.clone()
        } else {
            c["queue"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .map(|(i, mut n)| {
                    if let Some(o) =
                        c["profileConfigs"][n["assetPath"].as_str().unwrap_or("")].as_object()
                    {
                        for (k, v) in o {
                            n[k] = v.clone();
                        }
                    }
                    n["id"] = json!(format!("legacy-{i}"));
                    n
                })
                .collect()
        };
        nodes.retain(|n| {
            n["profileAssetPath"]
                .as_str()
                .or(n["assetPath"].as_str())
                .map(|p| !p.is_empty())
                .unwrap_or(false)
        });
        for n in &mut nodes {
            if let Some(p) = n["profileAssetPath"].as_str() {
                n["assetPath"] = json!(p);
            }
            if n["targetName"].is_null() {
                n["targetName"] = json!(target_name(n["target"].as_i64().unwrap_or(0)));
            }
        }
        nodes
    }
    fn public_status(&self) -> Value {
        let j = self.job.lock().unwrap();
        if j.is_null() {
            return json!({"job":null,"busy":self.busy()});
        }
        let queue: Vec<_> = j["queue"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|n| json!({"name":n["name"],"targetName":n["targetName"]}))
            .collect();
        let mut progress = json!({});
        if let Some(p) = j["progress"].as_object() {
            for (k, v) in p {
                progress[k] =
                    json!({"stage":v["stage"],"pct":v["pct"],"ok":v["ok"],"status":v["status"]});
            }
        }
        json!({"busy":self.busy(),"job":{"state":j["state"],"index":j["index"],"count":j["count"],"queue":queue,"progress":progress,"end":if j["end"].is_object(){json!({"ok":j["end"]["ok"],"reason":j["end"]["reason"]})}else{Value::Null}}})
    }
    fn catalog(&self) -> Value {
        let c = self.config();
        let nodes = self.nodes();
        let groups=c["buildGroups"].as_array().cloned().unwrap_or_else(||vec![json!({"id":"default","name":"默认构建组","nodeIds":nodes.iter().map(|n|n["id"].clone()).collect::<Vec<_>>()})]);
        json!({"nodes":nodes.iter().map(|n|json!({"id":n["id"],"name":n["name"],"targetName":n["targetName"],"dev":n.get("dev").unwrap_or(&c["defaultDevBuild"]),"bundleVersion":n["bundleVersion"]})).collect::<Vec<_>>(),"groups":groups.iter().map(|g|json!({"id":g["id"],"name":g["name"],"nodeIds":g["nodeIds"].as_array().into_iter().flatten().filter(|id|nodes.iter().any(|n|n["id"]==**id)).cloned().collect::<Vec<_>>()})).collect::<Vec<_>>()})
    }
    fn user_build(self: &Arc<Self>, b: Value) -> Result<Value, Fault> {
        if b.as_object()
            .map(|o| o.len() != 1 || !o.contains_key("nodeIds"))
            .unwrap_or(true)
        {
            return Err((400, "只能提交已配置的构建节点 ID".into()));
        }
        let ids = b["nodeIds"]
            .as_array()
            .filter(|a| !a.is_empty())
            .ok_or((400, "请选择构建节点".into()))?;
        let all = self.nodes();
        let mut selected = vec![];
        for id in ids {
            let n = all
                .iter()
                .find(|n| n["id"] == *id)
                .ok_or((400, "构建节点不存在，请刷新列表".into()))?;
            if !selected.iter().any(|v: &Value| v["id"] == *id) {
                selected.push(n.clone());
            }
        }
        let c = self.config();
        let mut request = json!({});
        for key in [
            "projectPath",
            "engine",
            "outputBase",
            "successDir",
            "artifactNameTemplate",
            "stopOnError",
            "buildNumber",
            "proxy",
        ] {
            request[key] = c[key].clone();
        }
        request["autoZip"] = json!(c["defaultAutoZip"] != false);
        request["dev"] = json!(c["defaultDevBuild"] == true);
        request["buildAddressables"] = json!(c["defaultBuildAddressables"] == true);
        request["androidBuildKind"] = json!(c["defaultAndroidBuildKind"].as_str().unwrap_or("apk"));
        for n in &mut selected {
            if !n["sign"].is_object() {
                n["sign"] = json!({});
            }
            for key in [
                "keystoreName",
                "keyaliasName",
                "keystorePass",
                "keyaliasPass",
                "customBuildDir",
                "beforeBuildVcsGroups",
            ] {
                if !n[key].is_null() {
                    n["sign"][key] = n[key].clone();
                }
            }
        }
        request["profiles"] = json!(selected);
        self.start_build(request).map_err(|(code, e)| {
            if code == 409 {
                (code, e)
            } else {
                (
                    code,
                    "构建配置不可用，请联系管理员检查项目路径和构建引擎".into(),
                )
            }
        })
    }
    fn scan(&self, b: &Value) -> Result<Value, Fault> {
        let p = self.resolve(&valstr(b, "projectPath"));
        if !project_valid(&p) {
            return Err((400, "项目路径无效，必须包含Assets和ProjectSettings".into()));
        }
        let mut profiles = vec![];
        let mut seen = std::collections::HashSet::new();
        fn walk(d: &Path, files: &mut Vec<PathBuf>) {
            if let Ok(es) = fs::read_dir(d) {
                for e in es.flatten() {
                    if let Ok(t) = e.file_type() {
                        if t.is_dir() {
                            walk(&e.path(), files)
                        } else if t.is_file()
                            && e.path().extension().and_then(|s| s.to_str()) == Some("asset")
                        {
                            files.push(e.path());
                        }
                    }
                }
            }
        }
        let mut files = vec![];
        let custom = valstr(b, "profileDir");
        walk(&p.join("Assets/Settings/Build Profiles"), &mut files);
        if !custom.is_empty() {
            walk(&abs(&p, &custom), &mut files);
        }
        for f in files {
            if !seen.insert(f.clone()) {
                continue;
            }
            let t = fs::read_to_string(&f).unwrap_or_default();
            if !t.contains("BuildProfile") {
                continue;
            }
            let field = |name: &str| {
                t.lines().find_map(|l| {
                    l.trim()
                        .strip_prefix(&format!("{name}:"))
                        .map(|v| v.trim().trim_matches('"').to_string())
                })
            };
            let target = field("m_BuildTarget")
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0);
            if !TARGETS.iter().any(|v| v.0 == target) {
                continue;
            }
            profiles.push(json!({"name":field("m_Name").unwrap_or_else(||f.file_stem().unwrap_or_default().to_string_lossy().into()),"target":target,"targetName":target_name(target),"assetPath":f.strip_prefix(&p).unwrap_or(&f).to_string_lossy().replace('\\',"/"),"absPath":f,"sceneCount":t.matches("m_path:").count(),"bundleVersion":field("bundleVersion"),"productName":field("productName"),"subtarget":field("m_Subtarget").and_then(|s|s.parse::<i64>().ok()).unwrap_or(0),"keystoreName":field("AndroidKeystoreName"),"keyaliasName":field("AndroidKeyaliasName"),"useCustomKeystore":field("androidUseCustomKeystore").as_deref()==Some("1")}));
        }
        let ver = fs::read_to_string(p.join("ProjectSettings/ProjectVersion.txt"))
            .unwrap_or_default()
            .lines()
            .find_map(|l| {
                l.strip_prefix("m_EditorVersion:")
                    .map(|s| s.trim().to_string())
            });
        Ok(json!({"ok":true,"projectAbs":p,"unityVersion":ver,"profiles":profiles,"ignored":[]}))
    }
    fn detect(&self) -> Value {
        let mut ex = vec![];
        for base in [
            std::env::var("ProgramFiles").ok(),
            std::env::var("ProgramFiles(x86)").ok(),
        ]
        .into_iter()
        .flatten()
        {
            if let Ok(es) = fs::read_dir(Path::new(&base).join("Unity/Hub/Editor")) {
                for e in es.flatten() {
                    let p = e.path().join("Editor/Unity.exe");
                    if p.is_file() {
                        ex.push(json!({"version":e.file_name().to_string_lossy(),"path":p}));
                    }
                }
            }
        }
        json!({"exes":ex,"cli":"unity"})
    }
    fn vcs_nodes(&self, b: &Value) -> Vec<Value> {
        if let Some(nodes) = b["nodes"].as_array() {
            return nodes.clone();
        }
        let c = self.config();
        let nodes = c["vcs"]["nodes"].as_array().cloned().unwrap_or_default();
        if let Some(ids) = b["ids"].as_array() {
            nodes
                .into_iter()
                .filter(|n| ids.contains(&n["id"]))
                .collect()
        } else {
            nodes
        }
    }
    fn start_vcs(self: &Arc<Self>, b: Value) -> Result<Value, Fault> {
        let nodes = self.vcs_nodes(&b);
        if nodes.is_empty() {
            return Err((400, "请选择版本管理节点".into()));
        }
        self.acquire()?;
        *self.vcs_job.lock().unwrap() =
            json!({"state":"running","index":0,"count":nodes.len(),"results":{}});
        let me = self.clone();
        thread::spawn(move || {
            struct Release<'a>(&'a Backend);
            impl Drop for Release<'_> {
                fn drop(&mut self) {
                    self.0.release();
                }
            }
            let _release = Release(&me);
            me.emit(json!({"type":"vcs-start","count":nodes.len()}));
            let mut all = true;
            for (i, n) in nodes.iter().enumerate() {
                if me.cancelled() {
                    all = false;
                    break;
                }
                me.vcs_job.lock().unwrap()["index"] = json!(i);
                let r = vcs::update(
                    &me,
                    n,
                    i,
                    Duration::from_secs(b["timeoutMinutes"].as_u64().unwrap_or(30) * 60),
                );
                if r["ok"] != true && r["status"] != "skipped" {
                    all = false
                }
                me.vcs_job.lock().unwrap()["results"][n["id"].as_str().unwrap_or("")] = r;
            }
            let end = json!({"ok":all&&!me.cancelled(),"reason":if me.cancelled(){"cancelled"}else if all{"done"}else{"error"}});
            {
                let mut j = me.vcs_job.lock().unwrap();
                j["state"] = json!("done");
                j["end"] = end.clone();
            }
            me.emit(json!({"type":"vcs-end","end":end}));
            me.release();
        });
        Ok(json!({"ok":true}))
    }
    pub fn handle(self: &Arc<Self>, method: &str, path: &str, body: Value) -> Result<Value, Fault> {
        if self.project_id.is_none() && path != "/api/config" && path != "/api/open-user-page" && path != "/api/monitor" {
            let id = self.config()["projects"]
                .as_array()
                .and_then(|p| p.first())
                .map(|p| valstr(p, "id"))
                .ok_or((404, "尚未配置项目".into()))?;
            return self.handle_admin_project(method, path, body, &id);
        }
        match (method, path) {
            ("GET", "/api/user/catalog") => Ok(self.catalog()),
            ("GET", "/api/user/status") => Ok(self.public_status()),
            ("POST", "/api/user/build") => self.user_build(body),
            ("POST", "/api/user/build/stop") => {
                if matches!(
                    self.job.lock().unwrap()["state"].as_str(),
                    Some("running" | "queued" | "starting")
                ) {
                    self.stop();
                }
                Ok(json!({"ok":true}))
            }
            ("GET", "/api/monitor") => self.monitor_snapshot(),
            ("GET", "/api/config") => Ok(json!({"ok":true,"config":self.config()})),
            ("GET", "/api/status") => Ok(
                json!({"config":self.config(),"unityExes":self.detect()["exes"],"cli":"unity","outputs":self.outputs()["outputs"],"job":self.job.lock().unwrap().clone(),"check":self.check.lock().unwrap().clone(),"vcs":self.vcs_job.lock().unwrap().clone()}),
            ),
            ("POST", "/api/config") => {
                if self.busy() {
                    return Err((409, "任务运行期间不能修改配置".into()));
                }
                if !body.is_object() {
                    return Err((400, "配置必须是对象".into()));
                }
                let mut c = self.config();
                for (k, v) in body.as_object().unwrap() {
                    c[k] = v.clone();
                }
                self.save(c).map_err(|e| (400, e))?;
                Ok(json!({"ok":true,"config":self.config()}))
            }
            ("GET", "/api/unity-detect") => Ok(self.detect()),
            ("POST", "/api/scan") => self.scan(&body),
            ("POST", "/api/build/preview") => self.preview(&body),
            ("POST", "/api/build/start") => self.start_build(body),
            ("POST", "/api/build/stop")
            | ("POST", "/api/check/stop")
            | ("POST", "/api/vcs/stop") => {
                self.stop();
                Ok(json!({"ok":true}))
            }
            ("GET", "/api/outputs") => Ok(self.outputs()),
            ("POST", "/api/check/preview") => self.check_preview(&body),
            ("POST", "/api/check/start") => self.start_check(body),
            ("POST", "/api/ai/analyze") => self.ai_analyze(&body),
            ("POST", "/api/vcs/save") => {
                if self.busy() {
                    return Err((409, "任务运行期间不能修改仓库配置".into()));
                }
                let mut c = self.config();
                c["vcs"] = body;
                self.save(c).map_err(|e| (400, e))?;
                Ok(json!({"ok":true}))
            }
            ("POST", "/api/vcs/probe") => {
                let nodes = self.vcs_nodes(&body);
                Ok(
                    json!({"ok":true,"results":nodes.iter().map(|n|vcs::probe(&self.runtime_root,n)).collect::<Vec<_>>()}),
                )
            }
            ("POST", "/api/vcs/update") => self.start_vcs(body),
            ("GET", "/api/vcs/state") => Ok(
                json!({"ok":true,"vcs":self.config()["vcs"],"nodes":self.config()["vcs"]["nodes"],"groups":self.config()["vcs"]["groups"],"job":self.vcs_job.lock().unwrap().clone()}),
            ),
            ("POST", "/api/open-user-page") => {
                let url = valstr(&body, "url");
                let parsed = url
                    .parse::<reqwest::Url>()
                    .map_err(|_| (400, "网页地址无效".into()))?;
                if parsed.scheme() != "http"
                    || parsed.host_str() != Some("127.0.0.1")
                        && parsed.host_str() != self.config()["server"]["host"].as_str()
                    || parsed.path() != "/" && !parsed.path().starts_with("/p/")
                    || parsed.query().is_some()
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                {
                    return Err((400, "只能打开本机用户主页".into()));
                }
                open::that(url).map_err(|e| (500, e.to_string()))?;
                Ok(json!({"ok":true}))
            }
            ("POST", "/api/open-folder") => {
                let p = self.resolve(&valstr(&body, "path"));
                let p = if p.is_file() {
                    p.parent().unwrap_or(&p).to_path_buf()
                } else {
                    p
                };
                if !p.is_dir() {
                    return Err((400, "目录不存在".into()));
                }
                open::that(p).map_err(|e| (500, e.to_string()))?;
                Ok(json!({"ok":true}))
            }
            ("POST", "/api/cli-probe") => {
                let exe = if cfg!(windows) { "unity.exe" } else { "unity" };
                let out = Command::new(exe)
                    .arg("--version")
                    .output()
                    .map_err(|e| (400, format!("Unity CLI不可用: {e}")))?;
                Ok(
                    json!({"ok":out.status.success(),"cli":exe,"version":String::from_utf8_lossy(&out.stdout),"exes":self.detect()["exes"]}),
                )
            }
            ("POST", "/api/cli-open") => {
                let p = self.resolve(&valstr(&body, "projectPath"));
                if !project_valid(&p) {
                    return Err((400, "项目路径无效".into()));
                }
                let exe = valstr(&self.config()["engine"], "unityExe");
                if exe.is_empty() {
                    return Err((400, "请先配置Unity路径".into()));
                }
                Command::new(exe)
                    .args(["-projectPath", &p.to_string_lossy()])
                    .spawn()
                    .map_err(|e| (500, e.to_string()))?;
                Ok(json!({"ok":true}))
            }
            _ => Err((404, "not found".into())),
        }
    }
}
