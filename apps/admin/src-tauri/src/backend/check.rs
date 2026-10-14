use chrono::{Local, Utc};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
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
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
fn target_name(target: i64) -> &'static str {
    TARGETS
        .iter()
        .find(|x| x.0 == target)
        .map(|x| x.1)
        .unwrap_or("未知平台")
}
fn targets(body: &Value) -> Vec<i64> {
    let mut out = Vec::new();
    if let Some(items) = body["targets"].as_array() {
        for item in items {
            let target = item
                .as_i64()
                .or_else(|| item.as_str().and_then(|s| s.parse().ok()));
            if let Some(t) = target {
                if TARGETS.iter().any(|x| x.0 == t) && !out.contains(&t) {
                    out.push(t);
                }
            }
        }
    }
    out
}
fn timeout_minutes(body: &Value) -> f64 {
    let minutes = body["timeoutMinutes"]
        .as_f64()
        .or_else(|| body["timeoutMinutes"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(20.0);
    if minutes.is_finite() && minutes != 0.0 {
        minutes.clamp(1.0, 525600.0)
    } else {
        20.0
    }
}
fn command(
    engine: &Value,
    project: &Path,
    target: i64,
    result: &Path,
    log: &Path,
    minutes: f64,
) -> (Vec<String>, String) {
    let mut args = vec!["-batchmode".to_owned()];
    if engine["nographics"] != false {
        args.push("-nographics".into());
    }
    args.extend([
        "-projectPath".into(),
        project.to_string_lossy().into_owned(),
        "-executeMethod".into(),
        "TBuildTool.Editor.CompileCheck.Run".into(),
        "-target".into(),
        target.to_string(),
        "-resultFile".into(),
        result.to_string_lossy().into_owned(),
        "-timeout".into(),
        minutes.to_string(),
        "-logFile".into(),
        log.to_string_lossy().into_owned(),
    ]);
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\\\""));
    let display = std::iter::once(quote(text(engine, "unityExe")))
        .chain(args.iter().map(|s| quote(s)))
        .collect::<Vec<_>>()
        .join(" ");
    (args, display)
}
fn summary(results: &Value) -> Value {
    let mut out = json!({"total":0,"ok":0,"fail":0,"unsupported":0,"timeout":0,"error":0,"cancelled":0,"running":0});
    if let Some(map) = results.as_object() {
        for result in map.values() {
            out["total"] = json!(out["total"].as_u64().unwrap_or(0) + 1);
            let status = text(result, "status");
            if let Some(count) = out.get(status).and_then(Value::as_u64) {
                out[status] = json!(count + 1);
            }
        }
    }
    out
}
struct Release<'a>(&'a super::Backend);
impl Drop for Release<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

impl super::Backend {
    pub(super) fn check_preview(&self, b: &Value) -> Result<Value, (u16, String)> {
        let selected = targets(b);
        let minutes = timeout_minutes(b);
        let project = if text(b, "projectPath").trim().is_empty() {
            PathBuf::from("<项目路径>")
        } else {
            self.resolve(text(b, "projectPath"))
        };
        let dir = project.join(".buildtool/check-results/check-<时间戳>");
        let engine = &b["engine"];
        let items: Vec<Value> = selected.iter().map(|&target| {
            let log = dir.join(format!("target-{target}.log"));
            let result = dir.join(format!("target-{target}.json"));
            let (_, display) = command(engine, &project, target, &result, &log, minutes);
            json!({"target":target,"name":target_name(target),"command":display,"logFile":log,"resultFile":result})
        }).collect();
        Ok(json!({"ok":true,"items":items,"targets":selected,"timeoutMinutes":minutes}))
    }

    pub(super) fn start_check(self: &Arc<Self>, b: Value) -> Result<Value, (u16, String)> {
        if self.busy() {
            return Err((
                400,
                "已有构建、环境检测或版本更新正在运行，请先停止任务".into(),
            ));
        }
        let project_input = text(&b, "projectPath");
        let project = self.resolve(project_input);
        if project_input.trim().is_empty()
            || !project.join("Assets").is_dir()
            || !project.join("ProjectSettings").is_dir()
        {
            return Err((400, format!("项目路径无效: {project_input}")));
        }
        let mut engine = b["engine"].clone();
        let exe = text(&engine, "unityExe").trim();
        if exe.is_empty() || !self.resolve(exe).is_file() {
            return Err((
                400,
                "未配置有效的 Unity.exe 路径（可在引擎与高级配置中设置或自动探测）".into(),
            ));
        }
        engine["unityExe"] = json!(self.resolve(exe).to_string_lossy());
        let selected = targets(&b);
        if selected.is_empty() {
            return Err((400, "未选择任何环境线（至少勾选一条目标平台）".into()));
        }
        let minutes = timeout_minutes(&b);
        self.acquire()?;
        let dir = project
            .join(".buildtool/check-results")
            .join(format!("check-{}", Local::now().format("%Y%m%d-%H%M%S-%f")));
        if let Err(e) = fs::create_dir_all(&dir) {
            self.release();
            return Err((500, format!("创建检测结果目录失败: {e}")));
        }
        let files: Vec<Value> = selected.iter().map(|t| json!({"target":t,"json":dir.join(format!("target-{t}.json")),"log":dir.join(format!("target-{t}.log"))})).collect();
        *self.check.lock().unwrap() = json!({"state":"starting","targets":selected,"count":selected.len(),"index":0,"results":{},"lines":[],"cancel":false,"end":null,"timeoutMinutes":minutes,"runDir":dir,"files":files,"projectAbs":project,"engine":engine,"startedAt":Utc::now().to_rfc3339()});
        self.emit(
            json!({"type":"check-start","count":selected.len(),"targets":selected,"runDir":dir}),
        );
        let backend = Arc::clone(self);
        let run_dir = dir.clone();
        let spawn = thread::Builder::new().name("unity-compile-check".into()).spawn(move || {
            let _release = Release(&backend);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                backend.check.lock().unwrap()["state"] = json!("running");
                for (index, &target) in selected.iter().enumerate() {
                    if backend.cancelled() { break; }
                    let log = run_dir.join(format!("target-{target}.log"));
                    let result_file = run_dir.join(format!("target-{target}.json"));
                    let (args, display) = command(&engine, &project, target, &result_file, &log, minutes);
                    {
                        let mut state = backend.check.lock().unwrap();
                        state["index"] = json!(index);
                        state["results"][target.to_string()] = json!({"target":target,"name":target_name(target),"status":"running","ok":false,"message":"检测中…（启动 Unity 编译）","ms":null});
                    }
                    backend.emit(json!({"type":"check-target-start","index":index,"target":target,"name":target_name(target),"logFile":log,"resultFile":result_file,"cmd":display}));
                    backend.check_note(format!("\n════════ 环境线 {}/{}：{} ════════\n命令: {}\n结果文件: {}\n",index+1,selected.len(),target_name(target),display,result_file.display()));
                    let started = Instant::now();
                    let deadline = Duration::from_secs_f64(minutes * 60.0 + 120.0);
                    let process_log = run_dir.join(format!("target-{target}.process.log"));
                    let tail_backend = Arc::clone(&backend);
                    let tail_log = log.clone();
                    let tail_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let tail_flag = Arc::clone(&tail_done);
                    let tail = thread::spawn(move || {
                        let mut offset = 0;
                        loop {
                            let done = tail_flag.load(std::sync::atomic::Ordering::Acquire);
                            if let Ok(mut file) = fs::File::open(&tail_log) {
                                if file.metadata().map(|m|m.len() < offset).unwrap_or(false) { offset = 0; }
                                if file.seek(SeekFrom::Start(offset)).is_ok() {
                                    let mut bytes = Vec::new();
                                    if file.read_to_end(&mut bytes).is_ok() && !bytes.is_empty() {
                                        offset += bytes.len() as u64;
                                        tail_backend.check_note(String::from_utf8_lossy(&bytes).into_owned());
                                    }
                                }
                            }
                            if done { break; }
                            thread::sleep(Duration::from_millis(200));
                        }
                    });
                    let process = backend.run_process(text(&engine, "unityExe"), &args, backend.runtime_root(), &process_log, "check-line", index, deadline);
                    tail_done.store(true, std::sync::atomic::Ordering::Release);
                    let _ = tail.join();
                    let mut result = json!({"target":target,"name":target_name(target),"status":"error","ok":false,"ms":started.elapsed().as_millis() as u64});
                    if backend.cancelled() {
                        result["status"] = json!("cancelled");
                        result["message"] = json!("环境检测已取消，Unity 进程已终止");
                    } else {
                        match process {
                            Err(error) => {
                                let lower = error.to_lowercase();
                                if lower.contains("timeout") || lower.contains("timed out") || error.contains("超时") || started.elapsed() >= deadline {
                                    result["status"] = json!("timeout");
                                }
                                result["message"] = json!(error);
                            }
                            Ok(code) => {
                                let parsed = fs::read_to_string(&result_file).ok().and_then(|s| serde_json::from_str::<Value>(s.trim_start_matches('\u{feff}')).ok());
                                if let Some(mut data) = parsed.filter(|v| v["target"].as_i64() == Some(target) && matches!(text(v,"status"), "ok"|"unsupported"|"fail"|"error"|"timeout")) {
                                    let success = matches!(text(&data,"status"), "ok"|"unsupported");
                                    if code != 0 && success {
                                        data["status"] = json!("error");
                                        data["message"] = json!(format!("Unity 进程异常退出（exit={code}），详见日志"));
                                    } else if text(&data,"status") == "ok" && data["ok"] != true {
                                        data["status"] = json!("error");
                                        data["message"] = json!("Unity 结果文件的通过状态与 ok 字段不一致");
                                    }
                                    data["ok"] = json!(text(&data,"status") == "ok");
                                    if data["name"].as_str().unwrap_or("").is_empty() { data["name"] = json!(target_name(target)); }
                                    result = data;
                                } else {
                                    result["message"] = json!(format!("Unity 已退出（exit={code}），未生成有效的检测结果文件。可能脚本编译失败或 CompileCheck.Run 入口缺失，详见日志。"));
                                }
                                result["exitCode"] = json!(code);
                            }
                        }
                    }
                    result["logFile"] = json!(log);
                    result["resultFile"] = json!(result_file);
                    backend.check.lock().unwrap()["results"][target.to_string()] = result.clone();
                    backend.emit(json!({"type":"check-result","target":target,"result":result}));
                    backend.check_note(format!("\n[环境线 {}] 状态: {} · {}\n",target_name(target),text(&result,"status"),text(&result,"message")));
                }
            }));
            let cancelled = backend.cancelled();
            let mut state = backend.check.lock().unwrap_or_else(|e| e.into_inner());
            if cancelled {
                for &target in &selected {
                    let key = target.to_string();
                    if state["results"].get(&key).is_none() {
                        state["results"][&key] = json!({"target":target,"name":target_name(target),"status":"cancelled","ok":false,"ms":null,"message":"检测已取消，未执行"});
                        backend.emit(json!({"type":"check-result","target":target,"result":state["results"][&key]}));
                    }
                }
            }
            let sums = summary(&state["results"]);
            let overall = !cancelled && outcome.is_ok() && state["results"].as_object().map(|r| r.len() == selected.len() && r.values().all(|v| matches!(text(v,"status"),"ok"|"unsupported"))).unwrap_or(false);
            let mut end = json!({"ok":overall,"summary":sums});
            if cancelled { end["reason"] = json!("cancelled"); }
            else if outcome.is_err() { end["reason"] = json!("error"); end["message"] = json!("环境检测线程异常"); }
            let report = json!({"ok":overall,"summary":sums,"startedAt":state["startedAt"],"finishedAt":Utc::now().to_rfc3339(),"targetOrder":selected,"results":selected.iter().filter_map(|t|state["results"].get(t.to_string()).cloned()).collect::<Vec<_>>()});
            state["state"] = json!("done");
            state["cancel"] = json!(cancelled);
            state["end"] = end.clone();
            drop(state);
            if let Err(error) = serde_json::to_vec_pretty(&report).map_err(|e|e.to_string()).and_then(|bytes|fs::write(run_dir.join("summary.json"),bytes).map_err(|e|e.to_string())) {
                backend.check_note(format!("\n[警告] 写入 summary.json 失败: {error}\n"));
            }
            backend.emit(json!({"type":"check-end","end":end,"ok":overall,"summary":sums}));
        });
        if let Err(e) = spawn {
            let end =
                json!({"ok":false,"reason":"error","message":format!("启动检测线程失败: {e}")});
            {
                let mut state = self.check.lock().unwrap();
                state["state"] = json!("done");
                state["end"] = end.clone();
            }
            self.release();
            self.emit(json!({"type":"check-end","end":end}));
            return Err((500, format!("启动检测线程失败: {e}")));
        }
        Ok(json!({"ok":true,"runDir":dir}))
    }

    fn check_note(&self, note: String) {
        {
            let mut state = self.check.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(lines) = state["lines"].as_array_mut() {
                lines.push(json!(note));
                if lines.len() > 3000 {
                    lines.drain(..lines.len() - 3000);
                }
            }
        }
        self.emit(json!({"type":"check-line","text":note}));
    }

    pub(super) fn ai_analyze(&self, b: &Value) -> Result<Value, (u16, String)> {
        let config = self.config();
        let ai = &config["ai"];
        if ai["enabled"] != true || text(ai, "apiKey").trim().is_empty() {
            return Err((
                400,
                "未启用 AI 或未填写 API Key（引擎设置 → AI 分析）".into(),
            ));
        }
        let job = self.job.lock().unwrap().clone();
        let target = if !text(b, "name").is_empty() {
            job["queue"]
                .as_array()
                .and_then(|q| q.iter().find(|v| text(v, "name") == text(b, "name")))
                .cloned()
        } else {
            job.get("lastFail").filter(|v| v.is_object()).cloned()
        };
        let target =
            target.ok_or_else(|| (400, "没有可分析的失败记录（请先运行一次失败的构建）".into()))?;
        let mut log = String::new();
        if !text(&target, "logFile").is_empty() {
            log = read_log_tail(&self.resolve(text(&target, "logFile"))).unwrap_or_default();
        }
        if log.is_empty() && !text(&target, "output").is_empty() {
            log = read_log_tail(&self.resolve(text(&target, "output")).join("build.log"))
                .unwrap_or_default();
        }
        if log.is_empty() {
            return Err((400, format!("找不到构建日志: {}", text(&target, "logFile"))));
        }
        let excerpt = error_excerpt(&log);
        let base = if text(ai, "baseUrl").trim().is_empty() {
            "https://api.deepseek.com"
        } else {
            text(ai, "baseUrl").trim()
        };
        let model = if text(ai, "model").trim().is_empty() {
            "deepseek-chat"
        } else {
            text(ai, "model").trim()
        };
        let system = "你是一名资深的 Unity 构建排障专家，擅长 Android 动态壁纸 APK、iOS Xcode、Gradle、IL2CPP、keystore 签名、资源裁剪与网络依赖问题。用户提供构建失败日志摘录，请用中文回答：1) 失败原因（一句话概括）；2) 具体修复建议（分步骤、可直接执行）；3) 若是网络/依赖/签名问题请明确指出。保持简洁专业，不要客套。";
        let user = format!(
            "构建目标: {}\nProfile: {}\n输出: {}\n\n--- 构建日志摘录 ---\n{}\n--- 结束 ---",
            text(&target, "targetName"),
            text(&target, "name"),
            text(&target, "output"),
            excerpt
        );
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(90))
            .build()
            .map_err(|e| (400, format!("调用 AI 失败: {e}")))?;
        let response = client.post(format!("{}/chat/completions",base.trim_end_matches('/'))).bearer_auth(text(ai,"apiKey").trim()).json(&json!({"model":model,"messages":[{"role":"system","content":system},{"role":"user","content":user}],"temperature":0.3,"max_tokens":2000})).send().map_err(|e|(400,format!("调用 AI 失败: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|e| (400, format!("读取 AI 响应失败: {e}")))?;
        if !status.is_success() {
            return Err((
                400,
                format!(
                    "AI 接口返回 {}: {}",
                    status.as_u16(),
                    body.chars().take(300).collect::<String>()
                ),
            ));
        }
        let data: Value =
            serde_json::from_str(&body).map_err(|e| (400, format!("AI 返回无效 JSON: {e}")))?;
        let analysis = data["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim();
        if analysis.is_empty() {
            return Err((400, "AI 返回内容为空".into()));
        }
        Ok(json!({"ok":true,"analysis":analysis}))
    }
}

fn read_log_tail(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let size = file.metadata()?.len();
    file.seek(SeekFrom::Start(size.saturating_sub(8 * 1024 * 1024)))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_uses_compile_check_without_quit() {
        let engine = json!({"unityExe":"Unity.exe","nographics":false});
        let (args, _) = command(
            &engine,
            Path::new("Project"),
            13,
            Path::new("result.json"),
            Path::new("check.log"),
            20.0,
        );
        assert!(args
            .windows(2)
            .any(|a| a == ["-executeMethod", "TBuildTool.Editor.CompileCheck.Run"]));
        assert!(args.windows(2).any(|a| a == ["-resultFile", "result.json"]));
        assert!(!args.iter().any(|a| a == "-quit" || a == "-nographics"));
        assert_eq!(
            targets(&json!({"targets":[13,"19",13,999,null]})),
            vec![13, 19]
        );
        assert_eq!(timeout_minutes(&json!({"timeoutMinutes":-1})), 1.0);
    }

    #[test]
    fn excerpts_keep_failure_context_and_limit_unicode_safely() {
        let log = "prefix\ncontext\nerror CS1001: broken script\ndetail\n";
        assert!(error_excerpt(log).contains("error CS1001: broken script"));
        assert!(error_excerpt(log).contains("detail"));
        assert_eq!(error_excerpt(&"错".repeat(7000)).chars().count(), 6000);
        let sums = summary(&json!({"13":{"status":"fail"},"19":{"status":"unsupported"}}));
        assert_eq!(sums["total"], 2);
        assert_eq!(sums["fail"], 1);
        assert_eq!(sums["unsupported"], 1);
    }
}

fn error_excerpt(log: &str) -> String {
    let lines: Vec<&str> = log.lines().collect();
    let markers = [
        "FAILURE:",
        "What went wrong",
        "Execution failed",
        "BUILD FAILED",
        "GradleInvokationException",
        "UnityException",
        "Exception:",
    ];
    let last = lines
        .iter()
        .rposition(|line| markers.iter().any(|m| line.contains(m)) || line.contains("error CS"));
    let excerpt = if let Some(index) = last {
        let start = index.saturating_sub(3);
        lines[start..(start + 80).min(lines.len())].join("\n")
    } else {
        lines[lines.len().saturating_sub(150)..].join("\n")
    };
    let tail: String = excerpt.chars().rev().take(6000).collect();
    tail.chars().rev().collect()
}
