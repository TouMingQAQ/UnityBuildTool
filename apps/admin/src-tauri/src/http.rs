//! Embedded loopback HTTP service. No runtime Node.js or web asset files required.
#[path = "download.rs"]
mod download;
use crate::backend::Backend;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    error::Error,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tiny_http::{Header, Request, Response, Server, StatusCode};

pub struct Service {
    backend: Arc<Backend>,
    server: Mutex<Option<Arc<Server>>>,
    stopping: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}
impl Service {
    pub fn start(
        config: PathBuf,
        root: PathBuf,
        token: String,
    ) -> Result<(Service, String), Box<dyn Error>> {
        if token.is_empty()
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("管理令牌必须为非空安全 ASCII 字符串".into());
        }
        let backend = Backend::new(config, root.clone());
        let settings = backend.config();
        let (host, port) = listen_address(&settings)?;
        let server = Arc::new(Server::http((host.as_str(), port)).map_err(|e| {
            format!("启动 HTTP 服务失败 ({host}:{port})，请检查监听地址或端口是否被占用: {e}")
        })?);
        let addr = server.server_addr().to_ip().ok_or("无效的监听地址")?;
        let base = format!("http://{addr}");
        let stopping = Arc::new(AtomicBool::new(false));
        let state = Arc::new(State {
            backend: backend.clone(),
            root,
            token,
            origin: base.clone(),
            stopping: stopping.clone(),
            active: AtomicUsize::new(0),
        });
        let listener = server.clone();
        let flag = stopping.clone();
        let handle = thread::Builder::new()
            .name("tbuild-http".into())
            .spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    let request = match listener.recv_timeout(Duration::from_millis(200)) {
                        Ok(Some(r)) => r,
                        Ok(None) => continue,
                        Err(_) => break,
                    };
                    if state.active.fetch_add(1, Ordering::AcqRel) >= 64 {
                        state.active.fetch_sub(1, Ordering::AcqRel);
                        respond_json(request, 503, json!({"error":"连接过多，请稍后重试"}));
                        continue;
                    }
                    let state = state.clone();
                    thread::spawn(move || {
                        let _active = Active(state.clone());
                        serve(request, &state);
                    });
                }
            })?;
        Ok((
            Self {
                backend,
                server: Mutex::new(Some(server)),
                stopping,
                thread: Mutex::new(Some(handle)),
            },
            base,
        ))
    }
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(server) = self.server.lock().unwrap_or_else(|e| e.into_inner()).take() {
            server.unblock();
        }
        self.backend.stop();
        self.backend.wait_idle(Duration::from_secs(5));
        if let Some(handle) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = handle.join();
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}
struct State {
    backend: Arc<Backend>,
    root: PathBuf,
    token: String,
    origin: String,
    stopping: Arc<AtomicBool>,
    active: AtomicUsize,
}
struct Active(Arc<State>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}
fn listen_address(config: &Value) -> Result<(String, u16), Box<dyn Error>> {
    let host = config["server"]["host"].as_str().unwrap_or("127.0.0.1");
    let ip: std::net::IpAddr = host.parse().map_err(|_| "监听地址必须为有效 IP 地址")?;
    if ip.is_unspecified() {
        return Err("请配置明确的本机监听 IP 地址，不能使用通配地址".into());
    }
    let port = match config["server"].get("port") {
        None => 50345,
        Some(value) => value
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .ok_or("监听端口必须为 1–65535 的整数")?,
    };
    if port == 0 && !cfg!(test) {
        return Err("监听端口必须为 1–65535 的整数".into());
    }
    Ok((ip.to_string(), port))
}
fn project_route(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/projects/")?;
    let (slug, suffix) = rest.split_once('/')?;
    if slug.is_empty() || suffix.is_empty() {
        return None;
    }
    Some((slug, suffix))
}
fn project_page(path: &str) -> Option<(&str, bool)> {
    let rest = path.strip_prefix("/p/")?;
    let (slug, suffix) = rest.split_once('/').unwrap_or((rest, ""));
    if slug.is_empty() {
        return None;
    }
    match suffix {
        "" => Some((slug, false)),
        "workspace" | "workspace/" => Some((slug, true)),
        _ => None,
    }
}
fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid HTTP header")
}
fn req_header<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str())
}
fn equal(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |diff, (x, y)| diff | (x ^ y))
        == 0
}
fn authorized(request: &Request, token: &str) -> bool {
    if let Some(value) = req_header(request, "x-admin-token") {
        return equal(value, token);
    }
    req_header(request, "cookie")
        .unwrap_or("")
        .split(';')
        .filter_map(|s| s.trim().split_once('='))
        .any(|(k, v)| k == "tbuild_admin" && equal(v, token))
}
fn decode(input: &str, plus: bool) -> Result<String, download::Fault> {
    let mut result = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return Err((400, "无效 URL 编码".into()));
                }
                let h = (bytes[i + 1] as char).to_digit(16);
                let l = (bytes[i + 2] as char).to_digit(16);
                match (h, l) {
                    (Some(h), Some(l)) => result.push((h * 16 + l) as u8),
                    _ => return Err((400, "无效 URL 编码".into())),
                }
                i += 3;
            }
            b'+' if plus => {
                result.push(b' ');
                i += 1;
            }
            b => {
                result.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(result).map_err(|_| (400, "URL 必须为 UTF-8".into()))
}
fn url(raw: &str) -> Result<(String, HashMap<String, String>), download::Fault> {
    let (path, query) = raw.split_once('?').unwrap_or((raw, ""));
    let path = decode(path, false)?;
    if !path.starts_with('/') || path.contains(['\\', '\0']) || path.split('/').any(|s| s == "..") {
        return Err((403, "禁止访问此路径".into()));
    }
    let mut params = HashMap::new();
    for part in query.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        let k = decode(k, true)?;
        if params.insert(k, decode(v, true)?).is_some() {
            return Err((400, "重复的查询参数".into()));
        }
    }
    Ok((path, params))
}
fn respond_json(request: Request, status: u16, value: Value) {
    let response = Response::from_string(value.to_string())
        .with_status_code(StatusCode(status))
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"));
    let _ = request.respond(response);
}
fn error(request: Request, fault: download::Fault) {
    respond_json(request, fault.0, json!({"error":fault.1}));
}
fn public_route(method: &str, path: &str) -> bool {
    matches!(
        (method, path),
        (
            "GET",
            "/api/user/catalog"
                | "/api/user/status"
                | "/api/user/events"
                | "/api/user/files"
                | "/api/user/download"
                | "/api/user/download-folder"
        ) | ("POST", "/api/user/build" | "/api/user/build/stop")
    )
}
fn serve(mut request: Request, state: &State) {
    let (path, query) = match url(request.url()) {
        Ok(v) => v,
        Err(e) => {
            error(request, e);
            return;
        }
    };
    let method = request.method().as_str().to_owned();
    // Reject DNS rebinding and cross-origin mutations, including public build submissions.
    let host = req_header(&request, "host").unwrap_or("");
    let expected = state.origin.trim_start_matches("http://");
    let localhost = expected.replacen("127.0.0.1", "localhost", 1);
    if host != expected && host != localhost {
        error(request, (403, "无效的本地服务 Host".into()));
        return;
    }
    if !matches!(method.as_str(), "GET" | "HEAD")
        && req_header(&request, "origin")
            .is_some_and(|o| o != state.origin && o != format!("http://{localhost}"))
    {
        error(request, (403, "不允许跨站提交操作".into()));
        return;
    }
    let admin = authorized(&request, &state.token);
    if path == "/admin" || path.starts_with("/admin/") {
        if method != "GET" {
            error(request, (405, "仅支持 GET".into()));
            return;
        }
        if matches!(path.as_str(), "/admin" | "/admin/")
            && query.get("token").is_some_and(|v| equal(v, &state.token))
        {
            let response = Response::empty(303)
                .with_header(header("Location", "/admin/"))
                .with_header(header(
                    "Set-Cookie",
                    &format!(
                        "tbuild_admin={}; HttpOnly; SameSite=Strict; Path=/",
                        state.token
                    ),
                ))
                .with_header(header("Referrer-Policy", "no-referrer"));
            let _ = request.respond(response);
            return;
        }
        if !admin {
            error(request, (401, "管理端需要授权".into()));
            return;
        }
        let asset = match path.as_str() {
            "/admin" | "/admin/" | "/admin/index.html" => Some((
                include_bytes!("../../public/index.html").as_slice(),
                "text/html; charset=utf-8",
            )),
            "/admin/app.js" => Some((
                include_bytes!("../../public/app.js").as_slice(),
                "text/javascript; charset=utf-8",
            )),
            "/admin/monitor.js" => Some((
                include_bytes!("../../public/monitor.js").as_slice(),
                "text/javascript; charset=utf-8",
            )),
            "/admin/style.css" => Some((
                include_bytes!("../../public/style.css").as_slice(),
                "text/css; charset=utf-8",
            )),
            _ => None,
        };
        static_response(request, asset);
        return;
    }
    if path.starts_with("/api/") {
        let route = project_route(&path);
        let public = route.is_some() || path.starts_with("/api/user/");
        let scope = if let Some((slug, _)) = route {
            match state.backend.project_by_slug(slug) {
                Ok((id, project)) => Some((id, project, slug.to_owned())),
                Err(e) => {
                    error(request, e);
                    return;
                }
            }
        } else {
            None
        };
        // Unscoped public routes cannot select or expose another project's state.
        if public && scope.is_none() {
            error(request, (404, "请通过项目链接访问".into()));
            return;
        }
        let path = route
            .map(|(_, suffix)| format!("/api/user/{suffix}"))
            .unwrap_or(path.clone());
        let admin_project = query.get("project").map(String::as_str);
        if public && !public_route(&method, &path) {
            error(request, (404, "用户端不提供此操作".into()));
            return;
        }
        if !public && !admin {
            error(request, (401, "管理接口需要管理端授权".into()));
            return;
        }
        if method == "GET" && (path == "/api/user/events" || path == "/api/build/events") {
            let event_id = scope
                .as_ref()
                .map(|(id, _, _)| id.as_str())
                .or(admin_project);
            events(
                request,
                state,
                public,
                event_id,
                scope.as_ref().map(|(_, _, slug)| slug.as_str()),
            );
            return;
        }
        if method == "GET"
            && matches!(
                path.as_str(),
                "/api/user/files" | "/api/user/download" | "/api/user/download-folder"
            )
        {
            let config = &scope.as_ref().expect("public project scope validated").1;
            let roots = download::roots(config, &state.root);
            let id = query.get("root").map(String::as_str).unwrap_or("");
            let relative = query.get("path").map(String::as_str).unwrap_or("");
            if path == "/api/user/files" {
                match download::listing(&roots, id, relative) {
                    Ok(v) => respond_json(request, 200, v),
                    Err(e) => error(request, e),
                }
                return;
            }
            let result = download::select(&roots, id).and_then(|root| {
                if path == "/api/user/download" {
                    download::file(root, relative)
                } else {
                    download::folder(root, relative)
                }
            });
            match result {
                Ok(file) => {
                    let size = usize::try_from(file.size).ok();
                    let name = encode(&file.name);
                    let mime = file.mime;
                    let response = Response::new(
                        StatusCode(200),
                        vec![
                            header("Content-Type", mime),
                            header(
                                "Content-Disposition",
                                &format!(
                                    "attachment; filename=\"download\"; filename*=UTF-8''{name}"
                                ),
                            ),
                            header("X-Content-Type-Options", "nosniff"),
                            header("Cache-Control", "no-store"),
                        ],
                        file,
                        size,
                        None,
                    );
                    let _ = request.respond(response);
                }
                Err(e) => error(request, e),
            }
            return;
        }
        if req_header(&request, "content-type")
            .is_some_and(|t| t.to_ascii_lowercase().starts_with("multipart/"))
        {
            error(request, (415, "不支持文件上传".into()));
            return;
        }
        if request.body_length().is_some_and(|n| n > 2 * 1024 * 1024) {
            error(request, (413, "请求体过大".into()));
            return;
        }
        let mut bytes = Vec::new();
        if request
            .as_reader()
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .is_err()
        {
            error(request, (400, "读取请求失败".into()));
            return;
        }
        if bytes.len() > 2 * 1024 * 1024 {
            error(request, (413, "请求体过大".into()));
            return;
        }
        let body = if bytes.is_empty() {
            json!({})
        } else {
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(v) => v,
                Err(_) => {
                    error(request, (400, "无效 JSON 请求".into()));
                    return;
                }
            }
        };
        let result = if let Some((_, _, slug)) = scope {
            state
                .backend
                .handle_user_project(&method, &path, body, &slug)
        } else if let Some(id) = admin_project {
            state.backend.handle_admin_project(&method, &path, body, id)
        } else {
            state.backend.handle(&method, &path, body)
        };
        match result {
            Ok(v) => respond_json(request, 200, v),
            Err(e) => error(request, e),
        }
        return;
    }
    if method != "GET" {
        error(request, (405, "仅支持 GET".into()));
        return;
    }
    if matches!(path.as_str(), "/" | "/index.html") {
        let config = state.backend.config();
        let mut html = String::from("<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>构建项目</title><link rel=\"stylesheet\" href=\"/user.css\"><body><main style=\"max-width:760px;margin:80px auto;padding:24px\"><h1>构建项目</h1><p>选择项目，进入对应的构建页面。</p><ul>");
        for project in config["projects"].as_array().into_iter().flatten() {
            if project["enabled"].as_bool() == Some(false) {
                continue;
            }
            let slug = project["slug"].as_str().unwrap_or("");
            let name = project["name"].as_str().unwrap_or(slug);
            html.push_str(&format!(
                "<li><a href=\"/p/{}\">{}</a></li>",
                encode(slug),
                escape_html(name)
            ));
        }
        html.push_str("</ul></main></body></html>");
        let _ = request.respond(
            Response::from_string(html)
                .with_header(header("Content-Type", "text/html; charset=utf-8"))
                .with_header(header("Cache-Control", "no-store"))
                .with_header(header("X-Content-Type-Options", "nosniff")),
        );
        return;
    }
    if let Some((slug, workspace)) = project_page(&path) {
        if let Err(e) = state.backend.project_by_slug(slug) {
            error(request, e);
            return;
        }
        let bytes = if workspace {
            include_bytes!("../../../user/public/workspace.html").as_slice()
        } else {
            include_bytes!("../../../user/public/index.html").as_slice()
        };
        static_response(request, Some((bytes, "text/html; charset=utf-8")));
        return;
    }
    let asset = match path.as_str() {
        "/" | "/index.html" => Some((
            include_bytes!("../../../user/public/index.html").as_slice(),
            "text/html; charset=utf-8",
        )),
        "/workspace" | "/workspace.html" => Some((
            include_bytes!("../../../user/public/workspace.html").as_slice(),
            "text/html; charset=utf-8",
        )),
        "/user.js" => Some((
            include_bytes!("../../../user/public/user.js").as_slice(),
            "text/javascript; charset=utf-8",
        )),
        "/user.css" => Some((
            include_bytes!("../../../user/public/user.css").as_slice(),
            "text/css; charset=utf-8",
        )),
        _ => None,
    };
    static_response(request, asset);
}
fn static_response(request: Request, asset: Option<(&'static [u8], &str)>) {
    match asset {
        Some((bytes, mime)) => {
            let response = Response::from_data(bytes)
                .with_header(header("Content-Type", mime))
                .with_header(header("Cache-Control", "no-store"))
                .with_header(header("X-Content-Type-Options", "nosniff"))
                .with_header(header("Referrer-Policy", "no-referrer"));
            let _ = request.respond(response);
        }
        None => error(request, (404, "not found".into())),
    }
}
fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn encode(name: &str) -> String {
    name.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn event_matches_project(event: &Value, project_id: Option<&str>) -> bool {
    project_id.is_none_or(|id| event["projectId"].as_str() == Some(id))
}
fn public_event(event: &Value) -> Option<Value> {
    let kind = event["type"].as_str()?;
    let fields: &[&str] = match kind {
        "job-start" => &["count"],
        "profile-start" => &["index", "name", "target"],
        "progress" => &["index", "stage", "pct"],
        "profile-end" => &["index", "ok", "skipped"],
        "job-end" => &["ok", "reason"],
        "done" => &[],
        _ => return None,
    };
    let mut safe = json!({"type":kind});
    for key in ["projectId", "projectName", "taskId"] {
        if let Some(value) = event.get(key).filter(|v| v.is_string()) {
            safe[key] = value.clone();
        }
    }
    for key in fields {
        if let Some(v) = event.get(*key) {
            safe[*key] = v.clone();
        }
    }
    if kind == "done" {
        safe["end"] = if event["end"].is_object() {
            json!({"ok":event["end"]["ok"],"reason":event["end"]["reason"]})
        } else {
            Value::Null
        };
    }
    if kind == "job-end" && event.get("message").is_some() {
        safe["message"] = json!("请在管理端查看详细错误日志");
    }
    Some(safe)
}
fn events(
    request: Request,
    state: &State,
    public: bool,
    project_id: Option<&str>,
    slug: Option<&str>,
) {
    let receiver = state.backend.subscribe();
    let result = if public {
        state
            .backend
            .handle_user_project("GET", "/api/user/status", json!({}), slug.unwrap_or(""))
    } else if let Some(id) = project_id {
        state
            .backend
            .handle_admin_project("GET", "/api/status", json!({}), id)
    } else {
        state.backend.handle("GET", "/api/status", json!({}))
    };
    let status = match result {
        Ok(v) => v,
        Err(e) => {
            error(request, e);
            return;
        }
    };
    let hello = if public {
        json!({"type":"hello","projectId":project_id,"job":status["job"],"queue":status["queue"]})
    } else {
        json!({"type":"hello","projectId":project_id,"job":status["job"],"check":status["check"],"vcs":status["vcs"]})
    };
    let mut writer = request.into_writer();
    if writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\nX-Accel-Buffering: no\r\n\r\nretry: 3000\n\n").is_err(){return;}
    if writeln!(writer, "data: {hello}\n").is_err() || writer.flush().is_err() {
        return;
    }
    while !state.stopping.load(Ordering::Acquire) {
        let frame = match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(event) => {
                if !event_matches_project(&event, project_id) {
                    continue;
                }
                let event = if public {
                    match public_event(&event) {
                        Some(e) => e,
                        None => continue,
                    }
                } else {
                    event
                };
                format!("data: {event}\n\n")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => ": ping\n\n".into(),
            Err(_) => break,
        };
        if writer.write_all(frame.as_bytes()).is_err() || writer.flush().is_err() {
            break;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_boundary() {
        assert_eq!(
            url("/api/user/download?path=%2E%2E%2Fsecret").unwrap().1["path"],
            "../secret"
        );
        assert!(url("/admin/../config.json").is_err());
        assert!(url("/api/user/files?root=a&root=b").is_err());
        assert!(url("/%zz").is_err());
    }
    #[test]
    fn loopback_routes_and_authorization() {
        use std::net::TcpStream;
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("config.json"), json!({"schemaVersion":2,"server":{"port":0},"projects":[{"id":"alpha","slug":"alpha","name":"Alpha","enabled":true}]}).to_string()).unwrap();
        let (service, base) = Service::start(
            directory.path().join("config.json"),
            directory.path().to_owned(),
            "test-token".into(),
        )
        .unwrap();
        let send = |path: &str, extra: &str| {
            let host = base.trim_start_matches("http://");
            let mut stream = TcpStream::connect(host).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(
                stream,
                "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n{extra}\r\n"
            )
            .unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).unwrap();
            String::from_utf8_lossy(&response).into_owned()
        };
        assert!(send("/", "").starts_with("HTTP/1.1 200"));
        assert!(send("/p/alpha", "").starts_with("HTTP/1.1 200"));
        assert!(send("/api/projects/unknown/catalog", "").starts_with("HTTP/1.1 404"));
        assert!(send("/api/config", "").starts_with("HTTP/1.1 401"));
        assert!(send("/admin/app.js", "").starts_with("HTTP/1.1 401"));
        let bootstrap = send("/admin/?token=test-token", "");
        assert!(bootstrap.starts_with("HTTP/1.1 303"));
        assert!(bootstrap.contains("HttpOnly; SameSite=Strict"));
        assert!(send("/admin/app.js", "Cookie: tbuild_admin=test-token\r\n")
            .starts_with("HTTP/1.1 200"));
        assert!(send("/admin/", "x-admin-token: test-token\r\n").starts_with("HTTP/1.1 200"));
        assert!(send("/api/user/upload", "").starts_with("HTTP/1.1 404"));
        assert!(
            send("/api/projects/alpha/files?path=%2E%2E%2Fsecret", "").starts_with("HTTP/1.1 200")
        ); // No configured roots exposes no data.
        let before = std::fs::read(directory.path().join("config.json")).unwrap();
        assert!(send("/p/alpha/workspace", "").starts_with("HTTP/1.1 200"));
        let mut stream = TcpStream::connect(base.trim_start_matches("http://")).unwrap();
        write!(stream,"POST /api/config HTTP/1.1\r\nHost: {}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",base.trim_start_matches("http://")).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 401"));
        assert_eq!(
            before,
            std::fs::read(directory.path().join("config.json")).unwrap()
        );
        service.stop();
    }
    fn get(base: &str, path: &str, admin: bool) -> String {
        use std::net::TcpStream;
        let host = base.trim_start_matches("http://");
        let mut socket = TcpStream::connect(host).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(
            socket,
            "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n{}\r\n",
            if admin {
                "x-admin-token: test-token\r\n"
            } else {
                ""
            }
        )
        .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    }
    #[test]
    fn project_routes_downloads_and_fixed_port() {
        let dir = tempfile::tempdir().unwrap();
        for (project, data) in [("a", "alpha-only"), ("b", "beta-only")] {
            std::fs::create_dir_all(dir.path().join(project).join("Builds")).unwrap();
            std::fs::write(dir.path().join(project).join("Builds/result.txt"), data).unwrap();
        }
        let config = json!({"schemaVersion":2,"server":{"host":"127.0.0.1","port":0},"projects":[
            {"id":"a","slug":"alpha","name":"A <client>","enabled":true,"projectPath":"a","outputBase":"Builds","buildNodes":[{"id":"same","name":"Alpha node","profileAssetPath":"Assets/Alpha.asset"}]},
            {"id":"b","slug":"beta","name":"B","enabled":true,"projectPath":"b","outputBase":"Builds","buildNodes":[{"id":"same","name":"Beta node","profileAssetPath":"Assets/Beta.asset"}]},
            {"id":"off","slug":"disabled","name":"Disabled","enabled":false}
        ]});
        let config_path = dir.path().join("config.json");
        std::fs::write(&config_path, config.to_string()).unwrap();
        let (service, base) = Service::start(
            config_path.clone(),
            dir.path().to_owned(),
            "test-token".into(),
        )
        .unwrap();
        let homepage = get(&base, "/", false);
        assert!(
            homepage.contains("/p/alpha")
                && homepage.contains("/p/beta")
                && homepage.contains("A &lt;client&gt;")
        );
        assert!(!homepage.contains("/p/disabled"));
        let alpha = get(&base, "/api/projects/alpha/catalog", false);
        assert!(
            alpha.starts_with("HTTP/1.1 200")
                && alpha.contains("Alpha node")
                && !alpha.contains("Beta node")
        );
        let beta = get(&base, "/api/projects/beta/catalog", false);
        assert!(beta.contains("Beta node") && !beta.contains("Alpha node"));
        assert!(get(
            &base,
            "/api/projects/alpha/download?root=builds&path=result.txt",
            false
        )
        .ends_with("alpha-only"));
        assert!(get(
            &base,
            "/api/projects/beta/download?root=builds&path=result.txt",
            false
        )
        .ends_with("beta-only"));
        assert!(get(
            &base,
            "/api/projects/alpha/download?path=../../b/Builds/result.txt",
            false
        )
        .starts_with("HTTP/1.1 403"));
        for path in [
            "/p/unknown",
            "/p/disabled",
            "/api/projects/unknown/files",
            "/api/projects/disabled/status",
            "/api/projects/alpha/config",
            "/api/user/catalog",
        ] {
            assert!(
                get(&base, path, false).starts_with("HTTP/1.1 404"),
                "{path}"
            );
        }
        assert!(get(&base, "/api/status?project=a", true).starts_with("HTTP/1.1 200"));
        assert!(get(&base, "/api/config?project=a", true).contains("Alpha node"));
        let port: u16 = base.rsplit(':').next().unwrap().parse().unwrap();
        let mut fixed = config;
        fixed["server"]["port"] = json!(port);
        std::fs::write(&config_path, fixed.to_string()).unwrap();
        assert!(
            Service::start(config_path, dir.path().to_owned(), "another-token".into()).is_err()
        );
        service.stop();
    }
    #[test]
    fn address_and_event_scope() {
        assert_eq!(
            listen_address(&json!({})).unwrap(),
            ("127.0.0.1".into(), 50345)
        );
        assert!(listen_address(&json!({"server":{"port":65536}})).is_err());
        assert!(listen_address(&json!({"server":{"host":"0.0.0.0"}})).is_err());
        assert!(project_route("/api/projects/alpha/catalog").is_some());
        assert!(event_matches_project(&json!({"projectId":"a"}), Some("a")));
        assert!(!event_matches_project(&json!({"projectId":"b"}), Some("a")));
        assert!(!event_matches_project(
            &json!({"type":"job-end"}),
            Some("a")
        ));
    }
    #[test]
    fn public_stream_omits_credentials() {
        let event = json!({"type":"profile-start","name":"x","index":0,"target":"Android","cmd":"password","output":"C:/secret","keystorePass":"secret"});
        assert_eq!(
            public_event(&event).unwrap(),
            json!({"type":"profile-start","name":"x","index":0,"target":"Android"})
        );
        assert!(public_event(&json!({"type":"line","text":"secret"})).is_none());
        assert!(!public_route("POST", "/api/user/config"));
        assert!(!public_route("POST", "/api/user/upload"));
    }
}
