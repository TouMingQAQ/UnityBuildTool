//! Build queue, Unity command composition, artifact packaging and output discovery.
use chrono::Local;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};
use zip::{write::SimpleFileOptions, ZipWriter};

type Fault = (u16, String);
fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn string(v: &Value, k: &str) -> String {
    text(v, k).into()
}
fn number(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}
fn choose(values: &[Value]) -> Value {
    values
        .iter()
        .find(|v| !v.is_null() && !(v.is_string() && v.as_str() == Some("")))
        .cloned()
        .unwrap_or(Value::Null)
}
fn safe(x: &str) -> String {
    let y: String = x
        .chars()
        .map(|c| {
            if "\\/:*?\"<>|\r\n".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    if y.trim().is_empty() {
        "unknown".into()
    } else {
        y.trim().into()
    }
}
fn path_at(root: &Path, raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() {
        p
    } else {
        root.join(p)
    }
}
fn quote(x: &str) -> String {
    format!("\"{}\"", x.replace('"', "\\\""))
}
fn split_command(x: &str) -> Result<Vec<String>, String> {
    // Matches the Node template parser: double quotes group arguments; no shell execution.
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut present = false;
    for c in x.chars() {
        if c == '"' {
            quoted = !quoted;
            present = true;
        } else if c.is_whitespace() && !quoted {
            if present {
                out.push(std::mem::take(&mut cur));
                present = false;
            }
        } else {
            cur.push(c);
            present = true;
        }
    }
    if quoted {
        return Err("命令模板包含未闭合的双引号".into());
    }
    if present {
        out.push(cur)
    }
    if out.is_empty() || out[0].trim().is_empty() {
        return Err("命令模板没有可执行程序".into());
    }
    Ok(out)
}
struct Context {
    profile: Value,
    project: PathBuf,
    base: PathBuf,
    dir: PathBuf,
    output: PathBuf,
    log: PathBuf,
    no: i64,
    success: String,
}
impl Context {
    fn new(backend: &super::Backend, b: &Value, pr: &Value, no: i64) -> Self {
        let cfg = b
            .get("_configSnapshot")
            .cloned()
            .unwrap_or_else(|| backend.config());
        let project = backend.resolve(text(b, "projectPath"));
        let base = path_at(
            &project,
            if text(b, "outputBase").is_empty() {
                "Builds"
            } else {
                text(b, "outputBase")
            },
        );
        let mut p = pr.clone();
        let mut sign = b.get("sign").cloned().unwrap_or(json!({}));
        if !sign.is_object() {
            sign = json!({})
        }
        if let Some(m) = pr.get("sign").and_then(Value::as_object) {
            for (k, v) in m {
                sign[k] = v.clone();
            }
        }
        for key in [
            "customBuildDir",
            "androidBuildKind",
            "dev",
            "buildAddressables",
            "addressablesMethod",
            "autoZip",
        ] {
            p[key] = choose(&[
                pr[key].clone(),
                sign[key].clone(),
                b[key].clone(),
                match key {
                    "androidBuildKind" => json!("apk"),
                    "autoZip" => cfg.get("defaultAutoZip").cloned().unwrap_or(json!(true)),
                    "dev" | "buildAddressables" => json!(false),
                    _ => Value::Null,
                },
            ]);
        }
        p["nameTemplate"] = choose(&[
            pr["nameTemplate"].clone(),
            sign["nameTemplate"].clone(),
            b["artifactNameTemplate"].clone(),
            cfg["artifactNameTemplate"].clone(),
        ]);
        p["sign"] = sign;
        if text(&p, "targetName").is_empty() {
            p["targetName"] = json!(super::target_name(number(&p["target"]).unwrap_or(19)));
        }
        if text(&p, "absPath").is_empty() {
            p["absPath"] = json!(path_at(&project, text(&p, "assetPath")));
        }
        let kind = text(&p, "androidBuildKind").to_ascii_lowercase();
        p["androidBuildKind"] = json!(kind);
        let custom = text(&p, "customBuildDir").trim();
        let dir = if custom.is_empty() {
            base.join(safe(text(&p, "name")))
        } else if Path::new(custom).is_absolute() {
            PathBuf::from(custom)
        } else if custom
            .replace('\\', "/")
            .to_ascii_lowercase()
            .starts_with("builds/")
        {
            base.parent().unwrap_or(&project).join(custom)
        } else {
            base.join(custom)
        };
        // The backend supplies a directory; Unity owns the platform-specific filename.
        let output = dir.clone();
        let log = dir.join("build.log");
        let success = choose(&[
            b["successDir"].clone(),
            cfg["successDir"].clone(),
            json!("构建成功"),
        ])
        .as_str()
        .unwrap_or("构建成功")
        .to_string();
        Self {
            profile: p,
            project,
            base,
            dir,
            output,
            log,
            no,
            success,
        }
    }
    fn command(&self, engine: &Value) -> Result<(String, Vec<String>, String), String> {
        let p = &self.profile;
        let sign = &p["sign"];
        let target = number(&p["target"]).unwrap_or(19);
        let dev = p["dev"].as_bool().unwrap_or(false);
        let address = p["buildAddressables"].as_bool().unwrap_or(false);
        let method = if text(p, "addressablesMethod").is_empty() {
            "true"
        } else {
            text(p, "addressablesMethod")
        };
        let kind = text(p, "androidBuildKind");
        if text(engine, "mode") == "template" {
            let mut t = string(engine, "template");
            let pairs = [
                (
                    "{cli}",
                    if text(engine, "cliPath").is_empty() {
                        "unity".into()
                    } else {
                        string(engine, "cliPath")
                    },
                ),
                ("{unityExe}", string(engine, "unityExe")),
                ("{project}", self.project.to_string_lossy().into_owned()),
                ("{profilePath}", string(p, "assetPath")),
                ("{profileAbs}", string(p, "absPath")),
                ("{profileName}", string(p, "name")),
                ("{target}", string(p, "targetName")),
                ("{output}", self.output.to_string_lossy().into_owned()),
                ("{logFile}", self.log.to_string_lossy().into_owned()),
                (
                    "{dev}",
                    if dev {
                        "-dev true".into()
                    } else {
                        String::new()
                    },
                ),
                (
                    "{buildAddressables}",
                    if address {
                        format!("-buildAddressables {method}")
                    } else {
                        String::new()
                    },
                ),
                ("{keystoreName}", string(sign, "keystoreName")),
                ("{keyaliasName}", string(sign, "keyaliasName")),
                ("{keystorePass}", string(sign, "keystorePass")),
                ("{keyaliasPass}", string(sign, "keyaliasPass")),
                (
                    "{androidBuildKind}",
                    if target == 13 && kind != "apk" {
                        format!("-androidBuildKind {kind}")
                    } else {
                        String::new()
                    },
                ),
                (
                    "{versionCode}",
                    if target == 13 {
                        self.no.to_string()
                    } else {
                        String::new()
                    },
                ),
                ("{buildNumber}", self.no.to_string()),
            ];
            for (k, v) in pairs {
                t = t.replace(k, &v)
            }
            let mut parts = split_command(&t)?;
            let exe = parts.remove(0);
            return Ok((exe, parts, t));
        }
        let exe = string(engine, "unityExe");
        let mut args = vec!["-batchmode".into()];
        if engine["nographics"] != false {
            args.push("-nographics".into())
        }
        args.extend([
            "-quit".into(),
            "-projectPath".into(),
            self.project.to_string_lossy().into_owned(),
            "-executeMethod".into(),
            "TBuildTool.Editor.BuildCommand.Build".into(),
            "-profilePath".into(),
            string(p, "assetPath"),
            "-outputDirectory".into(),
            self.output.to_string_lossy().into_owned(),
            "-wallpaperTarget".into(),
            target.to_string(),
            "-logFile".into(),
            self.log.to_string_lossy().into_owned(),
        ]);
        if dev {
            args.extend(["-dev".into(), "true".into()])
        }
        if address {
            args.extend(["-buildAddressables".into(), method.into()])
        }
        if target == 13 {
            for (k, flag) in [
                ("keystoreName", "-keystoreName"),
                ("keyaliasName", "-keystoreAlias"),
                ("keystorePass", "-keystorePass"),
                ("keyaliasPass", "-keyaliasPass"),
            ] {
                if !text(sign, k).is_empty() {
                    args.extend([flag.into(), string(sign, k)])
                }
            }
            if self.no > 0 {
                args.extend(["-versionCode".into(), self.no.to_string()])
            }
            if kind != "apk" {
                args.extend(["-androidBuildKind".into(), kind.into()])
            }
        }
        let display = std::iter::once(quote(&exe))
            .chain(args.iter().map(|s| quote(s)))
            .collect::<Vec<_>>()
            .join(" ");
        Ok((exe, args, display))
    }
    fn artifact_name(&self, preview: bool) -> String {
        archive_name(&self.profile, self.no, preview)
    }
}
fn archive_name(p: &Value, no: i64, preview: bool) -> String {
    let now = Local::now();
    let time = if preview {
        "<时间戳>".into()
    } else {
        now.format("%Y%m%d_%H%M%S").to_string()
    };
    let date = if preview {
        "<日期>".into()
    } else {
        now.format("%Y%m%d").to_string()
    };
    let mut t = if text(p, "nameTemplate").trim().is_empty() {
        "{Product}_{Platform}_v{Version}_b{VersionCode}_{Time}{Dev}".into()
    } else {
        string(p, "nameTemplate")
    };
    for (k, v) in [
        ("{Platform}", string(p, "targetName")),
        ("{Target}", string(p, "targetName")),
        ("{Profile}", string(p, "name")),
        ("{ProfileName}", string(p, "name")),
        ("{Product}", string(p, "productName")),
        ("{ProductName}", string(p, "productName")),
        ("{Version}", string(p, "bundleVersion")),
        ("{VersionCode}", no.to_string()),
        ("{BuildNumber}", no.to_string()),
        ("{Time}", time.clone()),
        ("{DateTime}", time),
        ("{Date}", date),
        (
            "{Dev}",
            if p["dev"] == true {
                "_DEV".into()
            } else {
                String::new()
            },
        ),
        (
            "{DEV}",
            if p["dev"] == true {
                "DEV".into()
            } else {
                String::new()
            },
        ),
    ] {
        t = t.replace(k, &v)
    }
    while t.contains("__") {
        t = t.replace("__", "_")
    }
    while t.contains("--") {
        t = t.replace("--", "-")
    }
    safe(t.trim_matches(&['_', '-'][..]))
}

#[derive(Default)]
struct ProgressDecoder {
    pending: Vec<u8>,
}
impl ProgressDecoder {
    fn push(&mut self, bytes: &[u8]) -> Vec<(String, Option<u64>)> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            if let Some((_, payload)) = line.split_once("[TBuildToolProgress] ") {
                if let Ok(v) = serde_json::from_str::<Value>(payload.trim()) {
                    if v["version"].as_u64() != Some(1) { continue; }
                    if let Some(stage) = v["stage"].as_str().filter(|s| !s.trim().is_empty()) {
                        let pct = v["pct"].as_f64().filter(|p| p.is_finite() && *p >= 0.0)
                            .map(|p| p.clamp(0.0, 100.0).round() as u64);
                        events.push((stage.to_owned(), pct));
                    }
                }
            }
        }
        // A malformed/no-newline log must not grow memory indefinitely.
        if self.pending.len() > 1024 * 1024 { self.pending.clear(); }
        events
    }
}

fn read_build_result(dir: &Path) -> Result<Value, String> {
    let raw = fs::read_to_string(dir.join("build-result.json")).map_err(|e| {
        format!("未生成预期产物结果 build-result.json，请更新 Unity 插件或命令模板: {e}")
    })?;
    let result: Value = serde_json::from_str(&raw).map_err(|e| format!("构建结果无效: {e}"))?;
    let output = PathBuf::from(text(&result, "outputPath"));
    if !output.is_absolute() || output == dir {
        return Err("构建结果必须包含输出目录内的实际产物绝对路径".into());
    }
    let actual = output
        .canonicalize()
        .map_err(|e| format!("未生成预期产物: {e}"))?;
    let root = dir.canonicalize().map_err(|e| e.to_string())?;
    if !actual.starts_with(&root) || actual == root {
        return Err("构建产物路径超出输出目录".into());
    }
    Ok(result)
}
fn group_nodes(config: &Value, request: &Value, profile: &Value) -> (Vec<Value>, Vec<Value>) {
    let gids = choose(&[
        profile["vcsGroupIds"].clone(),
        profile["beforeBuildVcsGroups"].clone(),
        request["beforeBuildVcsGroups"].clone(),
    ]);
    let ids = gids.as_array().cloned().unwrap_or_default();
    let vc = &config["vcs"];
    let mut node_ids = HashSet::new();
    let mut names = Vec::new();
    for g in vc["groups"].as_array().cloned().unwrap_or_default() {
        if ids.iter().any(|id| id == &g["id"]) {
            names.push(choose(&[g["name"].clone(), g["id"].clone()]));
            for id in g["nodeIds"].as_array().cloned().unwrap_or_default() {
                node_ids.insert(id.to_string());
            }
        }
    }
    (
        vc["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|n| node_ids.contains(&n["id"].to_string()))
            .collect(),
        names,
    )
}
fn exclude(name: &str, dir: bool, target: i64) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        ".buildtool"
            | ".git"
            | ".svn"
            | ".hg"
            | ".vs"
            | ".idea"
            | ".vscode"
            | ".ds_store"
            | "thumbs.db"
            | "desktop.ini"
            | "ehthumbs.db"
            | "build-result.json"
    ) || n.contains("donotship")
        || n.contains("dontship")
        || n.contains("backupthisfolder")
        || n.starts_with('~')
        || [
            ".log",
            ".dmp",
            ".mdmp",
            ".tmp",
            ".temp",
            ".bak",
            ".swp",
            ".ilk",
            ".exp",
            ".tlog",
            ".idb",
            ".lastcodeanalysissucceeded",
        ]
        .iter()
        .any(|x| n.ends_with(x))
        || (!dir && [".apk", ".aab", ".zip"].iter().any(|x| n.ends_with(x)))
        || (target == 9
            && (n == "deriveddata"
                || n == "build"
                || n.ends_with(".intermediates")
                || n.ends_with(".dsym")))
}
fn collect(
    dir: &Path,
    base: &Path,
    target: i64,
    out: &mut Vec<(PathBuf, String)>,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for e in entries {
        let e = e.map_err(|e| e.to_string())?;
        let p = e.path();
        let ty = e.file_type().map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().into_owned();
        if exclude(&name, ty.is_dir(), target) {
            continue;
        }
        if ty.is_dir() {
            collect(&p, base, target, out)?
        } else if ty.is_file() {
            out.push((
                p.clone(),
                p.strip_prefix(base)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/"),
            ))
        } else if ty.is_symlink() {
            out.push((
                p.clone(),
                p.strip_prefix(base)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/"),
            ))
        }
    }
    Ok(())
}
fn write_zip(backend: &super::Backend, src: &Path, dst: &Path, target: i64) -> Result<u64, String> {
    let mut files = Vec::new();
    collect(src, src, target, &mut files)?;
    if files.is_empty() {
        return Err(format!("构建目录为空，无法生成 ZIP: {}", src.display()));
    }
    files.sort_by(|a, b| a.1.cmp(&b.1));
    let result = (|| {
        let mut zip = ZipWriter::new(File::create(dst).map_err(|e| e.to_string())?);
        for (p, rel) in files {
            if backend.cancelled() {
                return Err("cancelled".into());
            }
            let meta = fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
            let opt = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .large_file(meta.len() > u32::MAX as u64);
            if meta.file_type().is_symlink() {
                let link = fs::read_link(&p).map_err(|e| e.to_string())?;
                zip.add_symlink(rel, link.to_string_lossy().replace('\\', "/"), opt)
                    .map_err(|e| e.to_string())?;
            } else {
                #[cfg(unix)]
                let opt = {
                    use std::os::unix::fs::PermissionsExt;
                    opt.unix_permissions(meta.permissions().mode())
                };
                zip.start_file(rel, opt).map_err(|e| e.to_string())?;
                let mut input = File::open(&p).map_err(|e| e.to_string())?;
                let mut buf = [0u8; 1024 * 128];
                loop {
                    if backend.cancelled() {
                        return Err("cancelled".into());
                    }
                    let len = input.read(&mut buf).map_err(|e| e.to_string())?;
                    if len == 0 {
                        break;
                    }
                    zip.write_all(&buf[..len]).map_err(|e| e.to_string())?;
                }
            }
        }
        zip.finish().map_err(|e| e.to_string())?;
        fs::metadata(dst)
            .map(|m| m.len())
            .map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(dst);
    }
    result
}
fn read_info(path: &Path) -> Value {
    let mut info = json!({});
    if let Ok(t) = fs::read_to_string(path.join("build-info.txt")) {
        for l in t.lines() {
            if let Some((k, v)) = l.split_once(':') {
                info[k.trim().to_ascii_lowercase()] = json!(v.trim())
            }
        }
    }
    info
}
fn output_record(file: &Path, dir: &Path, info: &Value) -> Value {
    let meta = fs::metadata(file).ok();
    let ext = file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let time = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    json!({"name":choose(&[info["profile"].clone(),json!(dir.file_name().unwrap_or_default().to_string_lossy())]),"targetName":info["target"],"file":file,"dir":dir,"time":time,"version":info["version"],"versionCode":number(&info["buildnumber"]),"date":info["date"],"kind":if ext=="aab"||ext=="apk"{"apk"}else if ext=="exe"{"exe"}else if text(info,"target").to_lowercase().contains("ios"){"ios"}else{"zip"},"size":meta.map(|m|m.len()).unwrap_or(0),"dev":text(info,"dev")=="true"})
}
impl super::Backend {
    pub(super) fn preview(&self, body: &Value) -> Result<Value, Fault> {
        let engine = body.get("engine").cloned().unwrap_or(json!({}));
        let cfg = self.config();
        let no = number(&body["buildNumber"])
            .filter(|n| *n >= 0)
            .unwrap_or_else(|| number(&cfg["lastVersionCode"]).unwrap_or(0) + 1);
        let mut items = Vec::new();
        for pr in body["profiles"].as_array().cloned().unwrap_or_default() {
            if text(&pr, "assetPath").is_empty() {
                continue;
            }
            let c = Context::new(self, body, &pr, no);
            let (_, _, display) = c.command(&engine).map_err(|e| (400, e))?;
            let (nodes, groups) =
                group_nodes(body.get("_configSnapshot").unwrap_or(&cfg), body, &pr);
            let p = &c.profile;
            items.push(json!({"name":p["name"],"target":p["targetName"],"output":c.output,"logFile":c.log,"command":display,"versionCode":if number(&p["target"])==Some(13){json!(no)}else{Value::Null},"buildNo":no,"dev":p["dev"],"buildAddressables":p["buildAddressables"],"androidBuildKind":p["androidBuildKind"],"autoZip":p["autoZip"],"customBuildDir":p["customBuildDir"],"previewName":c.artifact_name(true),"archiveDir":c.base.join(&c.success).join(format!("{}-<时间戳>",safe(text(p,"name")))),"vcsBeforeBuild":{"enabled":!nodes.is_empty(),"groups":groups,"nodeCount":nodes.len(),"nodeNames":nodes.iter().map(|n|choose(&[n["name"].clone(),n["id"].clone()])).collect::<Vec<_>>()}}));
        }
        Ok(json!({"ok":true,"items":items,"engine":engine}))
    }
    pub(super) fn start_build(self: &Arc<Self>, mut body: Value) -> Result<Value, Fault> {
        body["_configSnapshot"] = self.config();
        let project = self.resolve(text(&body, "projectPath"));
        if !super::project_valid(&project) {
            return Err((400, "Unity 项目路径无效".into()));
        }
        let profiles = body["profiles"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| !text(p, "assetPath").is_empty())
            .collect::<Vec<_>>();
        if profiles.is_empty() {
            return Err((400, "未选择任何 Profile".into()));
        }
        let engine = &body["engine"];
        if text(engine, "mode") == "template" {
            if text(engine, "template").trim().is_empty() {
                return Err((400, "模板引擎下未填写命令模板".into()));
            }
        } else if !self.resolve(text(engine, "unityExe")).is_file()
            || text(engine, "unityExe").is_empty()
        {
            return Err((400, "未配置有效的 Unity 可执行文件路径".into()));
        }
        self.preview(&body)?;
        self.reserve_build()?;
        *self.job.lock().unwrap() = json!({"state":"queued","queue":profiles,"count":profiles.len(),"index":0,"lines":[],"progress":{},"outputs":[],"exitCode":null,"end":null,"lastFail":null,"stopOnError":body["stopOnError"]!=false,"cancel":false});
        let me = Arc::clone(self);
        let count = profiles.len();
        let spawn = thread::Builder::new()
            .name("unity-build".into())
            .spawn(move || {
                if !me.wait_turn() {
                    me.job.lock().unwrap()["state"] = json!("done");
                    me.job.lock().unwrap()["end"] = json!({"ok":false,"reason":"cancelled"});
                    me.emit(json!({"type":"job-end","ok":false,"reason":"cancelled"}));
                    me.release();
                    return;
                }
                me.job.lock().unwrap()["state"] = json!("running");
                me.emit(json!({"type":"job-start","count":count}));
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    me.build_queue(&body, &profiles)
                }));
                let end = match result {
                    Ok(Ok(())) => json!({"ok":true}),
                    Ok(Err((reason, message))) => {
                        json!({"ok":false,"reason":reason,"message":message})
                    }
                    Err(_) => json!({"ok":false,"reason":"error","message":"构建线程异常"}),
                };
                {
                    let mut j = me.job.lock().unwrap();
                    j["state"] = json!("done");
                    j["cancel"] = json!(me.cancelled());
                    j["end"] = end.clone();
                }
                let mut event = end.clone();
                event["type"] = json!("job-end");
                me.emit(event);
                me.emit(json!({"type":"done","end":end}));
                me.release();
            });
        if let Err(e) = spawn {
            self.job.lock().unwrap()["state"] = json!("done");
            self.release();
            return Err((500, e.to_string()));
        }
        Ok(
            json!({"ok":true,"state":"queued","projectId":self.project_id,"scheduler":self.scheduler_status()}),
        )
    }
    fn build_queue(
        self: &Arc<Self>,
        body: &Value,
        profiles: &[Value],
    ) -> Result<(), (String, String)> {
        let auto = number(&body["buildNumber"]).map(|n| n < 0).unwrap_or(true);
        let fixed = number(&body["buildNumber"]).unwrap_or(1);
        let stop = body["stopOnError"] != false;
        let mut any_failed = false;
        if profiles.iter().any(|p| number(&p["target"]) == Some(13)) {
            if let Err(e) = self.gradle_proxy(body) {
                self.emit(json!({"type":"notice","text":format!("[代理] Gradle 配置失败: {e}")}));
            }
        }
        for (index, pr) in profiles.iter().enumerate() {
            if self.cancelled() {
                return Err(("cancelled".into(), "构建已取消".into()));
            }
            self.job.lock().unwrap()["index"] = json!(index);
            if let Err(e) = self.before_build_vcs(body, pr, index) {
                any_failed = true;
                self.build_line(index, &e);
                self.emit(json!({"type":"profile-end","index":index,"ok":false,"skipped":true}));
                if self.cancelled() {
                    return Err(("cancelled".into(), e));
                }
                if stop {
                    return Err(("vcsPreFail".into(), e));
                }
                continue;
            }
            let no = if auto {
                number(&self.config()["lastVersionCode"]).unwrap_or(0) + 1
            } else {
                fixed
            };
            let c = Context::new(self, body, pr, no);
            let started = Instant::now();
            let run = self.run_build_profile(body, &c, index);
            let (exit, result) = match run {
                Ok(0) => (0, self.archive_build(&c, started.elapsed(), index)),
                Ok(code) => (code, Err(format!("Unity 构建退出码 {code}"))),
                Err(e) => (-1, Err(e)),
            };
            let ok = result.is_ok();
            {
                let mut j = self.job.lock().unwrap();
                j["exitCode"] = json!(exit);
                if !ok {
                    j["lastFail"] = json!({"name":pr["name"],"targetName":c.profile["targetName"],"logFile":c.log,"output":c.output,"exitCode":exit});
                }
            }
            self.emit(json!({"type":"profile-end","index":index,"ok":ok,"exitCode":exit,"versionCode":if number(&pr["target"])==Some(13){json!(no)}else{Value::Null},"buildNo":no}));
            match result {
                Ok(out) => {
                    let mut cfg = self.config();
                    cfg["lastVersionCode"] = json!(no);

                    if let Err(e) = self.save(cfg) {
                        self.build_line(index, &format!("[警告] 构建号保存失败: {e}"));
                    }
                    self.job.lock().unwrap()["outputs"]
                        .as_array_mut()
                        .unwrap()
                        .insert(0, out.clone());
                    let mut evt = out;
                    evt["type"] = json!("output");
                    self.emit(evt);
                }
                Err(e) => {
                    any_failed = true;
                    self.build_line(index, &format!("[错误] {e}"));
                    if self.cancelled() {
                        return Err(("cancelled".into(), e));
                    }
                    if stop {
                        return Err(("stopOnError".into(), e));
                    }
                }
            }
        }
        if self.cancelled() {
            Err(("cancelled".into(), "构建已取消".into()))
        } else if any_failed {
            Err(("buildFailed".into(), "部分构建配置失败".into()))
        } else {
            Ok(())
        }
    }
    fn build_line(&self, index: usize, text: &str) {
        {
            let mut j = self.job.lock().unwrap();
            if let Some(lines) = j["lines"].as_array_mut() {
                lines.push(json!({"i":index,"t":text}));
                if lines.len() > 2000 {
                    let excess = lines.len() - 2000;
                    lines.drain(..excess);
                }
            }
        }
        self.emit(json!({"type":"line","index":index,"text":text}));
    }
    fn run_build_profile(
        self: &Arc<Self>,
        body: &Value,
        c: &Context,
        index: usize,
    ) -> Result<i32, String> {
        fs::create_dir_all(&c.dir).map_err(|e| e.to_string())?;
        // Never guess or delete player filenames. Only invalidate our previous result.
        let result_path = c.dir.join("build-result.json");
        if result_path.exists() {
            fs::remove_file(&result_path).map_err(|e| e.to_string())?;
        }
        let _ = fs::remove_file(&c.log);
        let mut engine = body["engine"].clone();
        if text(&engine, "mode") != "template" {
            engine["unityExe"] = json!(self.resolve(text(&engine, "unityExe")));
        }
        let (exe, args, display) = c.command(&engine)?;
        self.emit(json!({"type":"profile-start","index":index,"name":c.profile["name"],"target":c.profile["targetName"],"cmd":display,"output":c.output,"versionCode":c.no,"dev":c.profile["dev"],"buildAddressables":c.profile["buildAddressables"],"androidBuildKind":c.profile["androidBuildKind"]}));
        self.progress(index, "启动…", None);
        let finished = Arc::new(AtomicBool::new(false));
        let end = finished.clone();
        let backend = Arc::clone(self);
        let log = c.log.clone();
        let tail = thread::spawn(move || {
            let mut decoder = ProgressDecoder::default();
            let mut offset = 0u64;
            loop {
                if let Ok(mut f) = File::open(&log) {
                    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                    if len < offset {
                        offset = 0;
                        decoder = ProgressDecoder::default();
                    }
                    if len > offset {
                        let _ = f.seek(SeekFrom::Start(offset));
                        let mut buf = Vec::new();
                        let _ = f.read_to_end(&mut buf);
                        offset += buf.len() as u64;
                        let txt = String::from_utf8_lossy(&buf);
                        backend.build_line(index, &txt);
                        for (stage, pct) in decoder.push(&buf) {
                            backend.progress(index, &stage, pct);
                        }
                    }
                }
                if end.load(Ordering::Acquire) {
                    break;
                }
                thread::sleep(Duration::from_millis(180));
            }
        });
        let timeout = Duration::from_secs(
            number(&body["timeoutMinutes"])
                .filter(|n| *n > 0)
                .unwrap_or(24 * 60) as u64
                * 60,
        );
        let result = self.run_process(
            &exe,
            &args,
            &c.project,
            &c.dir.join("process.log"),
            "line",
            index,
            timeout,
        );
        finished.store(true, Ordering::Release);
        let _ = tail.join();
        if result == Ok(0) {
            self.progress(index, "构建完成，归档中…", Some(100));
        }
        result
    }
    fn progress(&self, index: usize, stage: &str, pct: Option<u64>) {
        self.job.lock().unwrap()["progress"][index.to_string()] = json!({"stage":stage,"pct":pct});
        self.emit(json!({"type":"progress","index":index,"stage":stage,"pct":pct}));
    }
    fn before_build_vcs(
        &self,
        body: &Value,
        pr: &Value,
        profile_index: usize,
    ) -> Result<(), String> {
        let (nodes, groups) = group_nodes(
            body.get("_configSnapshot").unwrap_or(&self.config()),
            body,
            pr,
        );
        if nodes.is_empty() {
            return Ok(());
        }
        *self.vcs_job.lock().unwrap() = json!({"state":"running","nodes":nodes,"index":0,"lines":[],"results":{},"preBuild":true,"end":null});
        self.emit(json!({"type":"vcs-start","count":nodes.len(),"ids":nodes.iter().map(|n|n["id"].clone()).collect::<Vec<_>>(),"preBuild":true,"groupNames":groups,"label":pr["name"],"profileIndex":profile_index}));
        let mut failed = 0;
        for (i, node) in nodes.iter().enumerate() {
            if self.cancelled() {
                failed += 1;
                break;
            }
            self.vcs_job.lock().unwrap()["index"] = json!(i);
            self.emit(json!({"type":"vcs-node-start","index":i,"nodeId":node["id"],"name":node["name"],"path":node["path"],"preBuild":true}));
            let r = super::vcs::update(self, node, i, Duration::from_secs(30 * 60));
            let status = text(&r, "status");
            if matches!(status, "fail" | "failed" | "cancelled")
                || (r["ok"] == false && status != "skipped")
            {
                failed += 1
            }
            self.vcs_job.lock().unwrap()["results"][text(node, "id")] = r;
        }
        let ok = failed == 0 && !self.cancelled();
        self.emit(json!({"type":"vcs-end","preBuild":true,"ok":ok,"summary":{"fail":failed}}));
        *self.vcs_job.lock().unwrap() = Value::Null;
        if ok {
            Ok(())
        } else {
            Err("构建前仓库更新失败或已取消".into())
        }
    }
    fn gradle_proxy(&self, body: &Value) -> Result<(), String> {
        let project = self.resolve(text(body, "projectPath"));
        let proxy = &body["proxy"];
        let mut lines = vec![
            "org.gradle.jvmargs=-Xmx4096m -XX:MaxMetaspaceSize=1024m".to_string(),
            "org.gradle.parallel=true".into(),
            "unityStreamingAssets=**STREAMING_ASSETS**".into(),
            format!(
                "unityProjectPath={}",
                project.to_string_lossy().replace('\\', "/")
            ),
            format!(
                "unity.projectPath={}",
                project.to_string_lossy().replace('\\', "/")
            ),
            "android.useAndroidX=true".into(),
            "android.enableJetifier=true".into(),
        ];
        if proxy["enabled"] == true && !text(proxy, "host").is_empty() {
            for scheme in ["http", "https"] {
                lines.push(format!(
                    "systemProp.{scheme}.proxyHost={}",
                    text(proxy, "host")
                ));
                lines.push(format!(
                    "systemProp.{scheme}.proxyPort={}",
                    choose(&[proxy["port"].clone(), json!(10808)])
                        .to_string()
                        .trim_matches('"')
                ));
            }
        }
        let dir = project.join("Assets/Plugins/Android");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        fs::write(
            dir.join("gradleTemplate.properties"),
            lines.join("\n") + "\n",
        )
        .map_err(|e| e.to_string())
    }
    fn archive_build(
        &self,
        c: &Context,
        duration: Duration,
        index: usize,
    ) -> Result<Value, String> {
        let result = read_build_result(&c.dir)?;
        let output = PathBuf::from(text(&result, "outputPath"));
        // A zero process exit without a tangible output is a failed build.
        if !output.exists() {
            return Err(format!(
                "Unity 退出成功但未生成预期产物: {}",
                output.display()
            ));
        }
        if output.is_file() && fs::metadata(&output).map_err(|e| e.to_string())?.len() == 0 {
            return Err("Unity 生成的产物为空".into());
        }
        if output.is_dir() {
            let mut entries = fs::read_dir(&output).map_err(|e| e.to_string())?;
            if entries.next().is_none() {
                return Err("Unity 生成的工程/应用目录为空".into());
            }
        }
        let now = Local::now();
        let final_dir = c.base.join(&c.success).join(format!(
            "{}-{}",
            safe(text(&c.profile, "name")),
            now.format("%Y%m%d_%H%M%S_%3f")
        ));
        fs::create_dir_all(&final_dir).map_err(|e| e.to_string())?;
        let mut resolved = c.profile.clone();
        resolved["productName"] = result["productName"].clone();
        resolved["bundleVersion"] = result["version"].clone();
        let name = archive_name(&resolved, c.no, false);
        let target = number(&c.profile["target"]).unwrap_or(19);
        let kind = text(&c.profile, "androidBuildKind");
        let android_file = target == 13 && matches!(kind, "apk" | "aab");
        let mut info=format!("Profile: {}\nTarget: {}\nProduct: {}\nVersion: {}\nBuildNumber: {}\nDEV: {}\nDate: {}\nDuration: {}s\nSourceDir: {}\n",text(&c.profile,"name"),text(&c.profile,"targetName"),text(&result,"productName"),text(&result,"version"),c.no,c.profile["dev"]==true,now.format("%Y-%m-%d %H:%M:%S"),duration.as_secs(),c.dir.display());
        fs::write(c.dir.join("build-info.txt"), &info).map_err(|e| e.to_string())?;
        let file = if android_file {
            let dst = final_dir.join(output.file_name().ok_or("产物没有文件名")?);
            fs::copy(&output, &dst).map_err(|e| e.to_string())?;
            dst
        } else if c.profile["autoZip"] != false {
            self.build_line(index, "[归档] 正在生成 ZIP…\n");
            let dst = final_dir.join(format!("{name}.zip"));
            let src = if target == 9 || (target == 13 && !android_file) {
                fs::write(output.join("build-info.txt"), &info).map_err(|e| e.to_string())?;
                &output
            } else {
                &c.dir
            };
            write_zip(self, src, &dst, target)?;
            dst
        } else {
            output.clone()
        };
        let size = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        info.push_str(&format!("Output: {}\nSize: {}\n", file.display(), size));
        fs::write(final_dir.join("build-info.txt"), &info).map_err(|e| e.to_string())?;
        let mut out = output_record(&file, &final_dir, &read_info(&final_dir));
        out["time"] = json!(now.timestamp_millis());
        out["kind"] = json!(if android_file {
            "apk"
        } else if target == 9 {
            "ios"
        } else if c.profile["autoZip"] == false && target != 4 {
            "exe"
        } else {
            "zip"
        });
        Ok(out)
    }
    pub(super) fn outputs(&self) -> Value {
        let cfg = self.config();
        let project = self.resolve(text(&cfg, "projectPath"));
        let base = path_at(
            &project,
            if text(&cfg, "outputBase").is_empty() {
                "Builds"
            } else {
                text(&cfg, "outputBase")
            },
        );
        let success = base.join(if text(&cfg, "successDir").is_empty() {
            "构建成功"
        } else {
            text(&cfg, "successDir")
        });
        let mut outputs = Vec::new();
        let mut seen = HashSet::new();
        for root in [success, base] {
            if let Ok(entries) = fs::read_dir(&root) {
                for ent in entries.flatten() {
                    let dir = ent.path();
                    if !dir.is_dir() {
                        continue;
                    }
                    let info = read_info(&dir);
                    let named = text(&info, "output");
                    if !named.is_empty() {
                        let file = path_at(&dir, named);
                        if file.exists() && seen.insert(file.clone()) {
                            outputs.push(output_record(&file, &dir, &info))
                        }
                    }
                    if let Ok(files) = fs::read_dir(&dir) {
                        for ent in files.flatten() {
                            let file = ent.path();
                            if file.is_file()
                                && file
                                    .extension()
                                    .and_then(|s| s.to_str())
                                    .map(|s| {
                                        ["apk", "aab", "exe", "zip"]
                                            .contains(&s.to_ascii_lowercase().as_str())
                                    })
                                    .unwrap_or(false)
                                && seen.insert(file.clone())
                            {
                                outputs.push(output_record(&file, &dir, &info));
                            }
                        }
                    }
                }
            }
        }
        if let Some(items) = self.job.lock().unwrap()["outputs"].as_array() {
            for out in items {
                let file = PathBuf::from(text(out, "file"));
                if file.exists() && seen.insert(file) {
                    outputs.push(out.clone());
                }
            }
        }
        outputs.sort_by(|a, b| b["time"].as_u64().cmp(&a["time"].as_u64()));
        outputs.truncate(50);
        json!({"ok":true,"outputs":outputs})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn template_parser_preserves_quoted_arguments_and_rejects_unclosed_quotes() {
        let args = split_command(r#"unity -projectPath "C:\My Project" -dev true"#).unwrap();
        assert_eq!(
            args,
            vec!["unity", "-projectPath", "C:\\My Project", "-dev", "true"]
        );
        assert!(split_command(r#"unity -projectPath "C:\broken"#).is_err());
    }

    #[test]
    fn profile_values_override_request_and_config_defaults() {
        let root = tempdir().unwrap();
        let config = root.path().join("config.json");
        fs::write(
            &config,
            r#"{"defaultAutoZip":true,"artifactNameTemplate":"config-{Product}"}"#,
        )
        .unwrap();
        let backend = super::super::Backend::new(config, root.path().to_path_buf());
        let body = json!({
            "projectPath":".", "outputBase":"Builds", "autoZip":false,
            "artifactNameTemplate":"request-{Product}", "profiles":[{
                "name":"Profile", "assetPath":"Assets/Profile.asset", "target":19,
                "targetName":"Windows (x64)", "productName":"Game", "autoZip":true,
                "nameTemplate":"profile-{Product}"
            }]
        });
        let ctx = Context::new(&backend, &body, &body["profiles"][0], 7);
        assert!(ctx.profile["autoZip"].as_bool().unwrap());
        assert_eq!(ctx.profile["nameTemplate"], "profile-{Product}");
        assert!(ctx.artifact_name(true).starts_with("profile-Game"));
    }

    #[test]
    fn progress_protocol_handles_byte_boundaries_and_ignores_lifecycle() {
        let log = "DisplayProgressbar: ignored\n[TBuildToolProgress] {\"version\":1,\"stage\":\"编译中文\",\"pct\":42.4}\n[TBuildToolProgress] {\"version\":1,\"stage\":\"归档\",\"pct\":-1}\n[TBuildToolProgress] {\"version\":2,\"stage\":\"bad\",\"pct\":100}\n[TBuildToolProgress] broken\n";
        let mut decoder = ProgressDecoder::default();
        let mut events = Vec::new();
        for byte in log.as_bytes() { events.extend(decoder.push(&[*byte])); }
        assert_eq!(events, vec![("编译中文".into(), Some(42)), ("归档".into(), None)]);
    }

    #[test]
    fn command_only_supplies_output_directory() {
        let root = tempdir().unwrap();
        let backend =
            super::super::Backend::new(root.path().join("config.json"), root.path().into());
        for target in [19, 13, 9, 4] {
            let body = json!({"projectPath":root.path(), "profiles":[{"name":"NotTheProduct", "target":target}]});
            let ctx = Context::new(&backend, &body, &body["profiles"][0], 1);
            assert_eq!(ctx.output, ctx.dir);
            assert!(ctx.profile["productName"].is_null());
            let (_, args, _) = ctx.command(&json!({"unityExe":"Unity.exe"})).unwrap();
            let flag = args.iter().position(|a| a == "-outputDirectory").unwrap();
            assert_eq!(args[flag + 1], ctx.dir.to_string_lossy());
            assert!(!args.iter().any(|a| a == "-outputPath"));
        }
    }

    #[test]
    fn archive_preserves_unity_apk_name() {
        let root = tempdir().unwrap();
        let backend =
            super::super::Backend::new(root.path().join("config.json"), root.path().into());
        let body = json!({"projectPath":root.path(), "profiles":[{"name":"Profile", "target":13, "androidBuildKind":"apk"}]});
        let ctx = Context::new(&backend, &body, &body["profiles"][0], 1);
        fs::create_dir_all(&ctx.dir).unwrap();
        let player = ctx.dir.join("ActualProduct.apk");
        fs::write(&player, b"apk").unwrap();
        fs::write(
            ctx.dir.join("build-result.json"),
            json!({"outputPath":player,"productName":"ActualProduct","version":"2.0"}).to_string(),
        )
        .unwrap();
        let out = backend.archive_build(&ctx, Duration::ZERO, 0).unwrap();
        assert_eq!(
            Path::new(out["file"].as_str().unwrap())
                .file_name()
                .unwrap(),
            "ActualProduct.apk"
        );
        let info =
            fs::read_to_string(Path::new(out["dir"].as_str().unwrap()).join("build-info.txt"))
                .unwrap();
        assert!(info.contains("Product: ActualProduct\nVersion: 2.0"));
    }

    #[test]
    fn result_rejects_paths_outside_output_directory() {
        let root = tempdir().unwrap();
        let dir = root.path().join("out");
        fs::create_dir(&dir).unwrap();
        let external = root.path().join("external.apk");
        fs::write(&external, b"old").unwrap();
        fs::write(
            dir.join("build-result.json"),
            json!({"outputPath":external}).to_string(),
        )
        .unwrap();
        assert!(read_build_result(&dir)
            .unwrap_err()
            .contains("超出输出目录"));
    }

    #[test]
    fn archive_rejects_missing_unity_output() {
        let root = tempdir().unwrap();
        let backend =
            super::super::Backend::new(root.path().join("config.json"), root.path().to_path_buf());
        let ctx = Context {
            profile: json!({"name":"Missing","target":19,"targetName":"Windows (x64)","productName":"Game","autoZip":true}),
            project: root.path().to_path_buf(),
            base: root.path().join("Builds"),
            dir: root.path().join("out"),
            output: root.path().join("out"),
            log: root.path().join("out/build.log"),
            no: 1,
            success: "成功".into(),
        };
        let err = backend
            .archive_build(&ctx, Duration::from_secs(0), 0)
            .unwrap_err();
        assert!(err.contains("未生成预期产物"));
    }

    #[test]
    fn zip_filter_excludes_logs_and_intermediate_artifacts() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("Game.exe"), b"game").unwrap();
        fs::write(root.path().join("build.log"), b"secret log").unwrap();
        fs::write(root.path().join("old.apk"), b"nested artifact").unwrap();
        fs::create_dir(
            root.path()
                .join("BackupThisFolder_butDontShipItWithYourGame"),
        )
        .unwrap();
        fs::write(
            root.path()
                .join("BackupThisFolder_butDontShipItWithYourGame/debug.txt"),
            b"debug",
        )
        .unwrap();
        let zip_path = root.path().join("artifact.zip");
        let backend = super::super::Backend::new(
            root.path().join(".buildtool/config.json"),
            root.path().to_path_buf(),
        );
        write_zip(&backend, root.path(), &zip_path, 19).unwrap();
        let file = File::open(zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let names: Vec<_> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert_eq!(names, vec!["Game.exe"]);
    }
}
