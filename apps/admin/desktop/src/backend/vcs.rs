//! Git/SVN probing and cancellable node updates. Relative paths use the runtime root.
use chrono::Utc;
use regex::Regex;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn text<'a>(node: &'a Value, key: &str) -> &'a str {
    node.get(key).and_then(Value::as_str).unwrap_or("")
}
fn name(node: &Value) -> Value {
    if text(node, "name").is_empty() {
        node["id"].clone()
    } else {
        node["name"].clone()
    }
}
fn detect(path: &Path, node: &Value) -> Option<&'static str> {
    match text(node, "type").to_ascii_lowercase().as_str() {
        "git" => return Some("git"),
        "svn" => return Some("svn"),
        _ => {}
    }
    let mut current = path;
    for _ in 0..12 {
        if current.join(".git").exists() {
            return Some("git");
        }
        if current.join(".svn").exists() {
            return Some("svn");
        }
        current = current.parent()?;
    }
    None
}
#[derive(Default)]
struct Output {
    code: i32,
    out: String,
    error: Option<String>,
}
impl Output {
    fn ok(&self) -> bool {
        self.code == 0 && self.error.is_none()
    }
    fn detail(&self) -> String {
        self.error.clone().unwrap_or_else(|| {
            format!(
                "退出码 {}：{}",
                self.code,
                self.out.trim().chars().take(200).collect::<String>()
            )
        })
    }
}
type Runner<'a> = dyn FnMut(&str, &[&str], Duration) -> Output + 'a;
fn seconds(n: u64) -> Duration {
    Duration::from_secs(n)
}

// Drain both pipes while waiting, so large status output cannot deadlock the probe.
fn quick(exe: &str, args: &[&str], cwd: &Path, timeout: Duration) -> Output {
    let mut command = Command::new(exe);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return Output {
                code: -1,
                error: Some(e.to_string()),
                ..Output::default()
            }
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut data = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut data);
            }
            data
        })
    };
    let out = read(stdout.map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = read(stderr.map(|p| Box::new(p) as Box<dyn Read + Send>));
    let start = Instant::now();
    let (code, error) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code().unwrap_or(-1), None),
            Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(20)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                let message = match result {
                    Err(e) => e.to_string(),
                    _ => format!("超时（{}ms）", timeout.as_millis()),
                };
                break (-1, Some(message));
            }
        }
    };
    // On timeout descendants may still own a pipe; don't wait on those reader threads.
    if error.is_some() {
        return Output {
            code,
            error,
            out: String::new(),
        };
    }
    let mut data = out.join().unwrap_or_default();
    data.extend(err.join().unwrap_or_default());
    Output {
        code,
        error,
        out: String::from_utf8_lossy(&data).into_owned(),
    }
}
#[derive(Default)]
struct Info {
    branch: String,
    root: String,
    revision: String,
    dirty_count: usize,
    has_remote: bool,
    remote: String,
    online: bool,
    error: Option<String>,
}
fn remote(run: &mut Runner<'_>) -> Info {
    let names = run("git", &["remote"], seconds(8));
    if !names.ok() {
        return Info {
            error: Some(format!("读取 remote 失败: {}", names.detail())),
            ..Info::default()
        };
    }
    let remote = names
        .out
        .lines()
        .find(|s| !s.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    if remote.is_empty() {
        return Info::default();
    }
    let online = run("git", &["ls-remote", "--heads", &remote], seconds(12)).ok();
    Info {
        has_remote: true,
        remote,
        online,
        ..Info::default()
    }
}
fn capture(pattern: &str, s: &str) -> String {
    Regex::new(pattern)
        .ok()
        .and_then(|re| {
            re.captures(s)
                .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        })
        .unwrap_or_default()
}
fn decode_xml_url(s: &str) -> String {
    let xml = s
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    let bytes = xml.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            if let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                decoded.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(decoded).unwrap_or(xml)
}
fn svn_info(run: &mut Runner<'_>) -> Info {
    let output = run("svn", &["info", "--xml"], seconds(20));
    if !output.ok() {
        return Info {
            has_remote: true,
            error: Some(output.detail()),
            ..Info::default()
        };
    }
    let url = capture(r"<url>([^<]+)</url>", &output.out);
    let root = capture(r"<root>([^<]+)</root>", &output.out);
    let online = !url.is_empty()
        && run(
            "svn",
            &["--non-interactive", "info", &decode_xml_url(&url)],
            seconds(15),
        )
        .ok();
    let status = run("svn", &["status"], seconds(20));
    Info {
        branch: decode_xml_url(&url),
        root: decode_xml_url(&root),
        revision: capture(r#"revision="(\d+)""#, &output.out),
        dirty_count: if status.ok() {
            status.out.lines().filter(|s| !s.trim().is_empty()).count()
        } else {
            0
        },
        has_remote: true,
        online,
        ..Info::default()
    }
}
fn svn_target(branch: &str, root: &str, current: &str) -> Option<String> {
    if Regex::new(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")
        .unwrap()
        .is_match(branch)
    {
        return Some(branch.to_string());
    }
    let fallback = Regex::new(r"/+(trunk|branches|tags)/.*$")
        .unwrap()
        .replace(current, "");
    let base = if root.is_empty() {
        fallback.as_ref()
    } else {
        root
    };
    if base.is_empty() {
        None
    } else {
        Some(format!(
            "{}/{}",
            base.trim_end_matches('/'),
            branch.trim_start_matches('/')
        ))
    }
}
fn normalize(url: &str) -> String {
    url.trim_end_matches('/').replace('\\', "/").to_lowercase()
}
fn branches(kind: &str, info: &Info, run: &mut Runner<'_>) -> Vec<String> {
    let mut values = Vec::new();
    if kind == "git" {
        for (refs, is_remote) in [("refs/heads", false), ("refs/remotes", true)] {
            let result = run(
                "git",
                &["for-each-ref", "--format=%(refname:short)", refs],
                seconds(10),
            );
            if result.ok() {
                for line in result.out.lines().map(str::trim) {
                    let line = if is_remote {
                        line.split_once('/').map(|(_, branch)| branch).unwrap_or("")
                    } else {
                        line
                    };
                    if !line.is_empty() && line != "HEAD" {
                        values.push(line.to_string());
                    }
                }
            }
        }
    } else {
        if info.online && !info.root.is_empty() {
            let url = format!("{}/branches", info.root.trim_end_matches('/'));
            let result = run("svn", &["--non-interactive", "ls", &url], seconds(12));
            values.push("trunk".to_string());
            if result.ok() {
                values.extend(
                    result
                        .out
                        .lines()
                        .map(|s| s.trim().trim_end_matches('/'))
                        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
                        .map(str::to_string),
                );
            }
        }
        let branch = capture(r"/(?:branches|tags)/([^/]+)/?$", &info.branch);
        if !branch.is_empty() {
            values.push(branch);
        }
        if info.branch.trim_end_matches('/').ends_with("/trunk") {
            values.push("trunk".to_string());
        }
    }
    let mut seen = HashSet::new();
    values.retain(|s| seen.insert(s.clone()));
    values
}

pub fn probe(root: &Path, node: &Value) -> Value {
    let path = text(node, "path");
    let abs = if Path::new(path).is_absolute() {
        Path::new(path).to_path_buf()
    } else {
        root.join(path)
    };
    let mut result = json!({"id":node["id"],"name":name(node),"path":path,"nodeBranch":text(node,"branch").trim(),"nodeRevert":text(node,"revert").trim(),
        "ok":true,"type":"none","root":"","branch":"","revision":"","dirty":false,"dirtyCount":0,"hasRemote":false,"remoteOnline":false,
        "branchMatched":null,"branchExists":null,"branches":[],"error":null});
    if path.trim().is_empty() || !abs.exists() {
        result["error"] = json!("路径为空或不存在");
        return result;
    }
    let kind = match detect(&abs, node) {
        Some(kind) => kind,
        None => {
            result["ok"] = json!(false);
            result["error"] = json!("未检测到 Git / SVN 工作副本（路径下无 .git 或 .svn）");
            return result;
        }
    };
    let mut run = |exe: &str, args: &[&str], timeout| quick(exe, args, &abs, timeout);
    let info = if kind == "git" {
        let branch = run("git", &["rev-parse", "--abbrev-ref", "HEAD"], seconds(10));
        let revision = run("git", &["rev-parse", "--short=8", "HEAD"], seconds(10));
        let status = run("git", &["status", "--porcelain"], seconds(20));
        let mut info = remote(&mut run);
        // An unborn HEAD is valid; a failed working-tree status is not.
        info.error = if !status.ok() {
            Some(status.detail())
        } else {
            None
        };
        info.branch = if branch.ok() {
            branch.out.trim().to_string()
        } else {
            "HEAD".to_string()
        };
        info.revision = if revision.ok() {
            revision.out.trim().to_string()
        } else {
            String::new()
        };
        info.dirty_count = if status.ok() {
            status.out.lines().filter(|s| !s.is_empty()).count()
        } else {
            0
        };
        info
    } else {
        svn_info(&mut run)
    };
    let branch = text(node, "branch").trim();
    if !branch.is_empty() {
        if kind == "git" {
            result["branchMatched"] = json!(info.branch.trim() == branch);
            if info.online && !info.remote.is_empty() {
                let output = run(
                    "git",
                    &["ls-remote", "--heads", &info.remote, branch],
                    seconds(10),
                );
                result["branchExists"] = json!(output.ok() && output.out.contains("refs/heads/"));
            }
        } else {
            let target = svn_target(branch, &info.root, &info.branch);
            result["branchMatched"] = json!(target
                .as_ref()
                .map(|url| normalize(url) == normalize(&info.branch))
                .unwrap_or(false));
            if info.online {
                if let Some(url) = target {
                    result["branchExists"] =
                        json!(run("svn", &["--non-interactive", "ls", &url], seconds(10)).ok());
                }
            }
        }
    }
    result["ok"] = json!(info.error.is_none());
    result["type"] = json!(kind);
    result["root"] = json!(info.root);
    result["branch"] = json!(info.branch);
    result["revision"] = json!(info.revision);
    result["dirty"] = json!(info.dirty_count > 0);
    result["dirtyCount"] = json!(info.dirty_count);
    result["hasRemote"] = json!(info.has_remote);
    result["remoteOnline"] = json!(info.online);
    result["branches"] = json!(branches(kind, &info, &mut run));
    result["error"] = json!(info.error.map(|s| format!("读取状态失败: {s}")));
    result
}

fn line(backend: &super::Backend, index: usize, message: &str) {
    backend.emit(json!({"type":"vcs-line","index":index,"text":message}));
}
fn finish(
    backend: &super::Backend,
    node: &Value,
    index: usize,
    kind: &str,
    status: &str,
    ok: bool,
    message: &str,
) -> Value {
    let result = json!({"id":node["id"],"name":name(node),"type":kind,"status":status,"ok":ok,"message":message,"finishedAt":Utc::now().to_rfc3339()});
    backend.emit(json!({"type":"vcs-node-end","index":index,"nodeId":node["id"],"name":name(node),"vcsType":kind,"status":status,"ok":ok,"message":message}));
    result
}

pub fn update(backend: &super::Backend, node: &Value, index: usize, timeout: Duration) -> Value {
    backend.emit(json!({"type":"vcs-node-start","index":index,"nodeId":node["id"],"name":name(node),"path":text(node,"path")}));
    let path = text(node, "path");
    let abs = backend.resolve(path);
    let kind = if !path.trim().is_empty() && abs.exists() {
        detect(&abs, node).unwrap_or("none")
    } else {
        "none"
    };
    let done = |status, ok, message: &str| finish(backend, node, index, kind, status, ok, message);
    if backend.cancelled() {
        return done("cancelled", false, "已取消");
    }
    if path.trim().is_empty() || !abs.exists() {
        line(
            backend,
            index,
            "（路径为空或不存在，跳过节点，未执行任何更新操作）\n",
        );
        return done("skipped", false, "路径为空或不存在，跳过");
    }
    if kind == "none" {
        line(
            backend,
            index,
            "（未检测到 Git / SVN 工作副本，跳过节点）\n",
        );
        return done("skipped", false, "未检测到 Git / SVN 工作副本");
    }
    let log_dir = backend.runtime_root().join("vcs-results");
    if let Err(e) = fs::create_dir_all(&log_dir) {
        return done("fail", false, &format!("无法创建版本管理日志目录：{e}"));
    }
    let stamp = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let mut step = 0usize;
    let mut run = |exe: &str, args: &[&str], limit: Duration| {
        if backend.cancelled() {
            return Output {
                code: -1,
                error: Some("cancelled".to_string()),
                ..Output::default()
            };
        }
        step += 1;
        let log = log_dir.join(format!("node-{index}-{stamp}-{step}.log"));
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        line(
            backend,
            index,
            &format!(
                "\n── {exe} {}\n",
                args.iter()
                    .map(|s| format!("{s:?}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
        let status = backend.run_process(
            exe,
            &args,
            &abs,
            &log,
            "vcs-line",
            index,
            timeout.min(limit),
        );
        let out = fs::read(&log)
            .map(|s| String::from_utf8_lossy(&s).into_owned())
            .unwrap_or_default();
        match status {
            Ok(code) => Output {
                code,
                out,
                error: None,
            },
            Err(error) => Output {
                code: -1,
                out,
                error: Some(error),
            },
        }
    };
    let info = if kind == "git" {
        remote(&mut run)
    } else {
        svn_info(&mut run)
    };
    if backend.cancelled() {
        return done("cancelled", false, "已取消");
    }
    line(
        backend,
        index,
        if info.online {
            "（远端在线，还原后更新）\n"
        } else {
            "（远端不可达或无远端服务器，仅还原未提交更改）\n"
        },
    );
    let custom = text(node, "revert").trim();
    let reverted = if !custom.is_empty() {
        #[cfg(windows)]
        let output = run("cmd.exe", &["/d", "/s", "/c", custom], timeout);
        #[cfg(not(windows))]
        let output = run("/bin/sh", &["-c", custom], timeout);
        output
    } else if kind == "svn" {
        run("svn", &["--non-interactive", "revert", "-R", "."], timeout)
    } else {
        let head = run("git", &["rev-parse", "--verify", "HEAD"], seconds(8));
        if !head.ok() {
            if head.error.is_some() {
                head
            } else {
                line(backend, index, "（仓库尚无提交记录，无未提交内容可还原）\n");
                Output::default()
            }
        } else {
            let first = run("git", &["reset", "--hard"], timeout);
            if first.ok() || backend.cancelled() || first.error.is_some() {
                first
            } else {
                run("git", &["checkout", "--", "."], timeout)
            }
        }
    };
    if backend.cancelled() {
        return done("cancelled", false, "已取消");
    }
    if !reverted.ok() {
        return done(
            "fail",
            false,
            &format!(
                "{}失败：{}",
                if custom.is_empty() {
                    "还原未提交更改"
                } else {
                    "自定义还原指令执行"
                },
                reverted.detail()
            ),
        );
    }
    if !info.online {
        return done(
            "partial",
            true,
            if kind == "svn" {
                "SVN 服务器不可达，仅还原未提交更改（未执行 update）"
            } else if info.has_remote {
                "远端不可达，仅还原未提交更改（未执行 pull）"
            } else {
                "无远端服务器，仅还原未提交更改（未执行 pull）"
            },
        );
    }
    let branch = text(node, "branch").trim();
    if !branch.is_empty() {
        if kind == "git" {
            let current = run("git", &["rev-parse", "--abbrev-ref", "HEAD"], seconds(10));
            if backend.cancelled() {
                return done("cancelled", false, "已取消");
            }
            if current.out.trim() != branch {
                let mut checkout = run("git", &["checkout", branch], timeout);
                if backend.cancelled() {
                    return done("cancelled", false, "已取消");
                }
                if !checkout.ok() && checkout.error.is_none() && !info.remote.is_empty() {
                    let fetch = run("git", &["fetch", &info.remote, branch], timeout);
                    if backend.cancelled() {
                        return done("cancelled", false, "已取消");
                    }
                    if fetch.ok() {
                        checkout = run(
                            "git",
                            &[
                                "checkout",
                                "-b",
                                branch,
                                "--track",
                                &format!("{}/{branch}", info.remote),
                            ],
                            timeout,
                        );
                    } else {
                        checkout = fetch;
                    }
                }
                if backend.cancelled() {
                    return done("cancelled", false, "已取消");
                }
                if !checkout.ok() {
                    return done(
                        "fail",
                        false,
                        &format!("切换分支失败：{}", checkout.detail()),
                    );
                }
            }
        } else if let Some(url) = svn_target(branch, &info.root, &info.branch) {
            if normalize(&url) != normalize(&info.branch) {
                let switch = run("svn", &["--non-interactive", "switch", &url], timeout);
                if backend.cancelled() {
                    return done("cancelled", false, "已取消");
                }
                if !switch.ok() {
                    return done("fail", false, &format!("切换分支失败：{}", switch.detail()));
                }
            }
        } else {
            line(
                backend,
                index,
                "（无法解析 SVN 分支 URL，按当前分支更新）\n",
            );
        }
    }
    let mut output = if kind == "git" {
        run("git", &["pull"], timeout)
    } else {
        run("svn", &["--non-interactive", "update"], timeout)
    };
    if backend.cancelled() {
        return done("cancelled", false, "已取消");
    }
    if kind == "git"
        && !output.ok()
        && output.error.is_none()
        && !branch.is_empty()
        && !info.remote.is_empty()
    {
        output = run("git", &["pull", &info.remote, branch], timeout);
    }
    if backend.cancelled() {
        return done("cancelled", false, "已取消");
    }
    if !output.ok() {
        return done("fail", false, &format!("更新失败：{}", output.detail()));
    }
    done(
        "ok",
        true,
        &if branch.is_empty() {
            "更新成功".to_string()
        } else {
            format!("更新成功（分支 {branch}）")
        },
    )
}
