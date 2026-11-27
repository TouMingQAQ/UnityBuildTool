use crate::admin_api::{AdminApi, AdminError};
use crate::backend::Backend;
use crate::http::Service;
use eframe::egui;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

pub const CARD: egui::Color32 = egui::Color32::from_rgb(17, 24, 39);
pub const CARD_ALT: egui::Color32 = egui::Color32::from_rgb(15, 23, 42);
pub const BG: egui::Color32 = egui::Color32::from_rgb(7, 10, 18);
pub const BG_PANEL: egui::Color32 = egui::Color32::from_rgb(11, 16, 29);
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(56, 189, 248);
pub const ACCENT_ALT: egui::Color32 = egui::Color32::from_rgb(99, 102, 241);
pub const OK: egui::Color32 = egui::Color32::from_rgb(16, 185, 129);
pub const WARN: egui::Color32 = egui::Color32::from_rgb(245, 158, 11);
pub const ERR: egui::Color32 = egui::Color32::from_rgb(244, 63, 94);
pub const MUTED: egui::Color32 = egui::Color32::from_rgb(148, 163, 184);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(226, 232, 240);

const LOG_LIMIT: usize = 5000;
const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);
const ENV_TARGETS: &[(i64, &str)] = &[
    (19, "Windows x64"),
    (5, "Windows x86"),
    (13, "Android"),
    (9, "iOS"),
    (4, "macOS"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Build,
    Monitor,
    Profiles,
    Check,
    Vcs,
    Settings,
    Help,
}

impl Page {
    pub const ALL: [Page; 7] = [
        Page::Build,
        Page::Monitor,
        Page::Profiles,
        Page::Check,
        Page::Vcs,
        Page::Settings,
        Page::Help,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Page::Build => "构建",
            Page::Monitor => "监控",
            Page::Profiles => "Profile 与节点",
            Page::Check => "环境检测",
            Page::Vcs => "版本管理",
            Page::Settings => "设置",
            Page::Help => "帮助",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Ok,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub created: Instant,
}

#[derive(Default)]
pub struct LogPanel {
    pub lines: VecDeque<String>,
    pub partial: String,
    pub search: String,
    pub follow: bool,
}

impl LogPanel {
    pub fn with_follow() -> Self {
        Self {
            follow: true,
            ..Default::default()
        }
    }

    pub fn push(&mut self, text: &str) {
        let mut current = std::mem::take(&mut self.partial);
        current.push_str(text);
        while let Some(pos) = current.find('\n') {
            let mut line: String = current.drain(..=pos).collect();
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
            self.lines.push_back(line);
        }
        while self.lines.len() > LOG_LIMIT {
            self.lines.pop_front();
        }
        if current.len() > 16_384 {
            self.lines.push_back(current);
            current = String::new();
        }
        self.partial = current;
        if self.lines.len() > LOG_LIMIT {
            self.lines.pop_front();
        }
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.partial.clear();
    }

    pub fn rendered(&self) -> String {
        let query = self.search.trim().to_lowercase();
        let mut out = Vec::new();
        for line in &self.lines {
            if query.is_empty() || line.to_lowercase().contains(&query) {
                out.push(line.as_str());
            }
        }
        if !self.partial.is_empty()
            && (query.is_empty() || self.partial.to_lowercase().contains(&query))
        {
            out.push(self.partial.as_str());
        }
        out.join("\n")
    }
}

pub enum DeferredAction {
    StartBuild(Value),
    StartCheck(Value),
    StartVcs(Value),
}

pub enum PendingKind {
    RootLoad,
    ProjectLoad,
    SaveProject,
    RootSave {
        body: Value,
        message: String,
        next: RootNext,
    },
    Scan,
    BuildPreview,
    BuildStart,
    StopBuild,
    AiAnalyze,
    Outputs,
    ApplicationPaths {
        scope: PathScope,
        asset_path: String,
    },
    ProjectPaths {
        scope: PathScope,
    },
    ArtifactsList,
    OpenArtifactFolder {
        root: String,
        path: String,
    },
    ApplicationPathOpen,
    ProjectPathOpen,
    OpenFolder,
    AndroidScan,
    UnityDetect,
    CliProbe,
    CliOpen,
    CheckPreview,
    CheckStart,
    StopCheck,
    VcsProbeAll,
    VcsProbeEditor,
    VcsStart,
    StopVcs,
    ProfileRead {
        asset_path: String,
    },
    ProfileWrite {
        asset_path: String,
    },
}

pub enum RootNext {
    Keep,
    Select(String),
    Clear,
}

#[derive(Clone, Copy)]
pub enum PathScope {
    Active,
    Monitor,
}

pub struct PendingEntry {
    pub kind: PendingKind,
    pub project_id: Option<String>,
}

pub struct WorkRequest {
    pub id: u64,
    pub project_id: Option<String>,
    pub method: String,
    pub path: String,
    pub body: Value,
}

pub enum UiEvent {
    Response {
        id: u64,
        result: Result<Value, AdminError>,
    },
    Backend(Value),
    Monitor(Result<Value, String>),
    Tray(TrayAction),
}

pub enum TrayAction {
    Toggle,
    Website,
    Quit,
}

pub struct ConfirmDialog {
    pub title: String,
    pub text: String,
    pub action: ConfirmAction,
}

pub enum ConfirmAction {
    DeleteProject(String),
    DeleteBuildNode(String),
    DeleteBuildGroup(String),
    DeleteVcsNode(String),
    DeleteVcsGroup(String),
    ClearLog(LogKind),
}

#[derive(Clone, Copy)]
pub enum LogKind {
    Build,
    Check,
    Vcs,
}

pub struct PreviewDialog {
    pub title: String,
    pub items: Vec<Value>,
}

pub struct AiDialog {
    pub loading: bool,
    pub text: String,
}

pub struct PlayerSettingsEditor {
    pub asset_path: String,
    pub product_name: String,
    pub company_name: String,
    pub bundle_version: String,
    pub revision: String,
    pub editable: bool,
    pub inherited: bool,
    pub fallback: bool,
    pub path: String,
    pub error: String,
    pub loading: bool,
}

impl PlayerSettingsEditor {
    pub fn new(asset_path: String) -> Self {
        Self {
            asset_path,
            product_name: String::new(),
            company_name: String::new(),
            bundle_version: String::new(),
            revision: String::new(),
            editable: true,
            inherited: false,
            fallback: false,
            path: String::new(),
            error: String::new(),
            loading: false,
        }
    }
}

pub struct BuildNodeEditor {
    pub editing_id: Option<String>,
    pub group_id: Option<String>,
    pub name: String,
    pub profile_asset_path: String,
    pub target: i64,
    pub custom_build_dir: String,
    pub name_template: String,
    pub auto_zip: bool,
    pub dev: bool,
    pub build_addressables: bool,
    pub addressables_method: String,
    pub android_build_kind: String,
    pub keystore_name: String,
    pub keyalias_name: String,
    pub keystore_pass: String,
    pub keyalias_pass: String,
    pub remember: bool,
    pub vcs_group_ids: Vec<String>,
    pub error: String,
    pub player: Option<PlayerSettingsEditor>,
}

impl BuildNodeEditor {
    pub fn new(group_id: Option<String>, defaults: &Value) -> Self {
        Self {
            editing_id: None,
            group_id,
            name: String::new(),
            profile_asset_path: String::new(),
            target: 19,
            custom_build_dir: String::new(),
            name_template: String::new(),
            auto_zip: defaults["defaultAutoZip"].as_bool().unwrap_or(true),
            dev: defaults["defaultDevBuild"].as_bool().unwrap_or(false),
            build_addressables: defaults["defaultBuildAddressables"]
                .as_bool()
                .unwrap_or(false),
            addressables_method: String::new(),
            android_build_kind: defaults["defaultAndroidBuildKind"]
                .as_str()
                .unwrap_or("apk")
                .to_owned(),
            keystore_name: String::new(),
            keyalias_name: "key".into(),
            keystore_pass: String::new(),
            keyalias_pass: String::new(),
            remember: true,
            vcs_group_ids: Vec::new(),
            error: String::new(),
            player: None,
        }
    }
}

pub struct VcsNodeEditor {
    pub editing_id: Option<String>,
    pub name: String,
    pub path: String,
    pub branch: String,
    pub revert: String,
    pub vcs_type: String,
    pub probe_text: String,
    pub probe_ok: bool,
    pub branches: Vec<String>,
    pub error: String,
}

impl VcsNodeEditor {
    pub fn new() -> Self {
        Self {
            editing_id: None,
            name: String::new(),
            path: String::new(),
            branch: String::new(),
            revert: String::new(),
            vcs_type: "auto".into(),
            probe_text: String::new(),
            probe_ok: false,
            branches: Vec::new(),
            error: String::new(),
        }
    }
}

#[derive(Clone, Default)]
pub struct MonitorSample {
    pub cpu: f32,
    pub memory: u64,
}

pub struct AdminApp {
    pub page: Page,
    pub page_enter_page: Page,
    pub page_enter_time: f64,
    pub base_url: String,
    pub visible: bool,
    pub build_drawer_open: bool,
    pub brand_icon: Option<egui::TextureHandle>,
    service: Option<Service>,
    api: AdminApi,
    backend: Arc<Backend>,
    tx: Sender<UiEvent>,
    rx: Receiver<UiEvent>,
    work_tx: Sender<WorkRequest>,
    next_id: u64,
    pub pending: HashMap<u64, PendingEntry>,
    pub saving: bool,
    save_flight: Option<(u64, Value)>,
    pub queued_action: Option<DeferredAction>,
    pub last_edit: Option<Instant>,
    pub save_error: Option<String>,
    pub config_loaded: bool,
    pub project_loading: bool,
    pub root_loaded: bool,
    pub root_loading: bool,
    pub selected_project_id: String,
    pub projects: Vec<Value>,
    pub root_config: Value,
    pub root_host: String,
    pub root_port: String,
    pub config: Value,
    pub saved_config: Value,
    pub status: Value,
    pub job: Value,
    pub check_job: Value,
    pub vcs_job: Value,
    pub outputs: Vec<Value>,
    pub profiles: Vec<Value>,
    pub scan_summary: String,
    pub ignored_profiles: Vec<Value>,
    pub unity_version: String,
    pub unity_options: Vec<Value>,
    pub build_selection: HashSet<String>,
    pub build_group: String,
    pub build_rows: Vec<Value>,
    pub build_progress: HashMap<usize, (String, Option<f32>, String)>,
    pub build_log: LogPanel,
    pub check_log: LogPanel,
    pub vcs_log: LogPanel,
    pub check_targets: Vec<i64>,
    pub check_results: HashMap<i64, Value>,
    pub check_run_dir: String,
    pub vcs_states: HashMap<String, Value>,
    pub toasts: VecDeque<Toast>,
    pub next_toast: u64,
    pub confirm: Option<ConfirmDialog>,
    pub preview: Option<PreviewDialog>,
    pub ai: Option<AiDialog>,
    pub build_editor: Option<BuildNodeEditor>,
    pub vcs_editor: Option<VcsNodeEditor>,
    pub project_meta_name: String,
    pub project_meta_slug: String,
    pub project_meta_enabled: bool,
    pub project_meta_for: String,
    pub settings_paths: Option<Value>,
    pub settings_paths_loading: bool,
    pub settings_path_asset: String,
    pub settings_project_paths: Vec<Value>,
    pub monitor_snapshot: Option<Value>,
    pub monitor_error: String,
    pub monitor_selected: String,
    pub monitor_history: HashMap<String, VecDeque<MonitorSample>>,
    pub monitor_artifacts: Option<Value>,
    pub monitor_artifact_loading: bool,
    pub monitor_paths: Option<Value>,
    pub monitor_paths_loading: bool,
    pub monitor_path_asset: String,
    pub monitor_log_channel: String,
    pub monitor_log_query: String,
    pub monitor_log_follow: bool,
    monitor_active: Arc<AtomicBool>,
    monitor_running: Arc<AtomicBool>,
    pub tray: Option<TrayIcon>,
    pub quitting: bool,
    pub server_restart_note: bool,
}

impl AdminApp {
    pub fn new(cc: &eframe::CreationContext<'_>, service: Service, base_url: String) -> Self {
        install_theme_and_fonts(&cc.egui_ctx);
        let backend = service.backend();
        let api = AdminApi::new(backend.clone());
        let (ui_tx, ui_rx) = mpsc::channel();
        let (work_tx, work_rx) = mpsc::channel::<WorkRequest>();
        let ctx = cc.egui_ctx.clone();

        let worker_api = api.clone();
        let worker_tx = ui_tx.clone();
        let worker_ctx = ctx.clone();
        thread::Builder::new()
            .name("tbuild-admin-worker".into())
            .spawn(move || {
                while let Ok(request) = work_rx.recv() {
                    let result = worker_api.call(
                        request.project_id.as_deref(),
                        &request.method,
                        &request.path,
                        request.body,
                    );
                    if worker_tx
                        .send(UiEvent::Response {
                            id: request.id,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                    worker_ctx.request_repaint();
                }
            })
            .expect("start administration worker");

        let events = backend.subscribe();
        let event_tx = ui_tx.clone();
        let event_ctx = ctx.clone();
        thread::Builder::new()
            .name("tbuild-admin-events".into())
            .spawn(move || {
                while let Ok(event) = events.recv() {
                    if event_tx.send(UiEvent::Backend(event)).is_err() {
                        break;
                    }
                    event_ctx.request_repaint();
                }
            })
            .expect("start backend event bridge");

        let monitor_active = Arc::new(AtomicBool::new(false));
        let monitor_running = Arc::new(AtomicBool::new(true));
        let monitor_api = api.clone();
        let monitor_tx = ui_tx.clone();
        let monitor_ctx = ctx.clone();
        let active = monitor_active.clone();
        let running = monitor_running.clone();
        thread::Builder::new()
            .name("tbuild-admin-monitor".into())
            .spawn(move || {
                let mut last = Instant::now()
                    .checked_sub(Duration::from_secs(2))
                    .unwrap_or_else(Instant::now);
                while running.load(Ordering::Acquire) {
                    if active.load(Ordering::Acquire) && last.elapsed() >= Duration::from_secs(2) {
                        let result = monitor_api
                            .call(None, "GET", "/api/monitor", json!({}))
                            .map_err(|e| e.message);
                        if monitor_tx.send(UiEvent::Monitor(result)).is_err() {
                            break;
                        }
                        monitor_ctx.request_repaint();
                        last = Instant::now();
                    }
                    thread::sleep(Duration::from_millis(160));
                }
            })
            .expect("start monitor sampler");

        let (tray_tx, tray_rx) = mpsc::channel::<TrayAction>();
        // Keep tray events on a dedicated channel so callback threads only wake egui.
        let tray_ctx = ctx.clone();
        let tray_sender = tray_tx.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let _ = tray_sender.send(TrayAction::Toggle);
                tray_ctx.request_repaint();
            }
        }));
        let menu_ctx = ctx.clone();
        let menu_sender = tray_tx;
        MenuEvent::set_event_handler(Some(Arc::new(move |event: MenuEvent| {
            let action = match event.id.as_ref() {
                "tbuild-toggle" => Some(TrayAction::Toggle),
                "tbuild-website" => Some(TrayAction::Website),
                "tbuild-quit" => Some(TrayAction::Quit),
                _ => None,
            };
            if let Some(action) = action {
                let _ = menu_sender.send(action);
                menu_ctx.request_repaint();
            }
        })));
        let tray_forward = ui_tx.clone();
        let forward_ctx = ctx.clone();
        thread::Builder::new()
            .name("tbuild-tray-events".into())
            .spawn(move || {
                while let Ok(action) = tray_rx.recv() {
                    if tray_forward.send(UiEvent::Tray(action)).is_err() {
                        break;
                    }
                    forward_ctx.request_repaint();
                }
            })
            .expect("start tray bridge");

        let icon_data = eframe::icon_data::from_png_bytes(include_bytes!("../icons/icon.png")).ok();
        let brand_icon = icon_data.as_ref().map(|icon| {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            );
            cc.egui_ctx.load_texture(
                "unitybuildtool-brand-icon",
                image,
                egui::TextureOptions::LINEAR,
            )
        });
        let tray = icon_data.as_ref().and_then(|icon| {
            let image =
                tray_icon::Icon::from_rgba(icon.rgba.clone(), icon.width, icon.height).ok()?;
            let menu = Menu::new();
            let toggle = MenuItem::with_id(
                "tbuild-toggle",
                "显示 / 隐藏管理端",
                true,
                None::<tray_icon::menu::accelerator::Accelerator>,
            );
            let website = MenuItem::with_id(
                "tbuild-website",
                "打开用户网页",
                true,
                None::<tray_icon::menu::accelerator::Accelerator>,
            );
            let quit = MenuItem::with_id(
                "tbuild-quit",
                "退出",
                true,
                None::<tray_icon::menu::accelerator::Accelerator>,
            );
            menu.append_items(&[&toggle, &website, &quit]).ok()?;
            TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("UnityBuildTool 管理端")
                .with_icon(image)
                .with_menu_on_left_click(false)
                .build()
                .ok()
        });

        let mut app = Self {
            page: Page::Build,
            page_enter_page: Page::Build,
            page_enter_time: 0.0,
            base_url,
            visible: true,
            build_drawer_open: false,
            brand_icon,
            service: Some(service),
            api,
            backend,
            tx: ui_tx,
            rx: ui_rx,
            work_tx,
            next_id: 1,
            pending: HashMap::new(),
            saving: false,
            save_flight: None,
            queued_action: None,
            last_edit: None,
            save_error: None,
            config_loaded: false,
            project_loading: false,
            root_loaded: false,
            root_loading: false,
            selected_project_id: String::new(),
            projects: Vec::new(),
            root_config: json!({"schemaVersion":2,"server":{"host":"127.0.0.1","port":50345},"projects":[]}),
            root_host: "127.0.0.1".into(),
            root_port: "50345".into(),
            config: json!({}),
            saved_config: json!({}),
            status: json!({}),
            job: Value::Null,
            check_job: Value::Null,
            vcs_job: Value::Null,
            outputs: Vec::new(),
            profiles: Vec::new(),
            scan_summary: String::new(),
            ignored_profiles: Vec::new(),
            unity_version: String::new(),
            unity_options: Vec::new(),
            build_selection: HashSet::new(),
            build_group: "all".into(),
            build_rows: Vec::new(),
            build_progress: HashMap::new(),
            build_log: LogPanel::with_follow(),
            check_log: LogPanel::with_follow(),
            vcs_log: LogPanel::with_follow(),
            check_targets: vec![19, 13],
            check_results: HashMap::new(),
            check_run_dir: String::new(),
            vcs_states: HashMap::new(),
            toasts: VecDeque::new(),
            next_toast: 1,
            confirm: None,
            preview: None,
            ai: None,
            build_editor: None,
            vcs_editor: None,
            project_meta_name: String::new(),
            project_meta_slug: String::new(),
            project_meta_enabled: true,
            project_meta_for: String::new(),
            settings_paths: None,
            settings_paths_loading: false,
            settings_path_asset: String::new(),
            settings_project_paths: Vec::new(),
            monitor_snapshot: None,
            monitor_error: String::new(),
            monitor_selected: String::new(),
            monitor_history: HashMap::new(),
            monitor_artifacts: None,
            monitor_artifact_loading: false,
            monitor_paths: None,
            monitor_paths_loading: false,
            monitor_path_asset: String::new(),
            monitor_log_channel: "all".into(),
            monitor_log_query: String::new(),
            monitor_log_follow: true,
            monitor_active,
            monitor_running,
            tray,
            quitting: false,
            server_restart_note: false,
        };
        if app.tray.is_none() {
            app.toast("系统托盘创建失败，管理端仍可使用", ToastKind::Error);
        }
        app.load_root();
        app
    }

    fn request(
        &mut self,
        kind: PendingKind,
        project_id: Option<String>,
        method: &str,
        path: &str,
        body: Value,
    ) -> Option<u64> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.pending.insert(
            id,
            PendingEntry {
                kind,
                project_id: project_id.clone(),
            },
        );
        let request = WorkRequest {
            id,
            project_id,
            method: method.into(),
            path: path.into(),
            body,
        };
        if self.work_tx.send(request).is_err() {
            self.pending.remove(&id);
            self.toast("后台工作线程不可用", ToastKind::Error);
            return None;
        }
        Some(id)
    }

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push_back(Toast {
            text: text.into(),
            kind,
            created: Instant::now(),
        });
        while self.toasts.len() > 6 {
            self.toasts.pop_front();
        }
    }

    pub fn busy(&self) -> bool {
        self.backend.busy()
    }

    pub fn current_project(&self) -> Option<&Value> {
        self.projects
            .iter()
            .find(|p| p["id"].as_str() == Some(self.selected_project_id.as_str()))
    }

    pub fn current_project_slug(&self) -> String {
        self.current_project()
            .and_then(|p| p["slug"].as_str())
            .unwrap_or("")
            .to_owned()
    }

    pub fn current_project_url(&self) -> String {
        let slug = self.current_project_slug();
        if slug.is_empty() {
            String::new()
        } else {
            format!("{}/p/{slug}", self.base_url)
        }
    }

    pub fn open_user_website(&mut self) {
        let url = self.current_project_url();
        if url.is_empty() {
            self.toast("请先创建或选择项目", ToastKind::Error);
        } else if let Err(error) = open::that(&url) {
            self.toast(format!("打开用户网页失败：{error}"), ToastKind::Error);
        }
    }

    pub fn open_project_website(&mut self, slug: &str) {
        if slug.trim().is_empty() {
            self.toast("项目链接标识为空", ToastKind::Error);
            return;
        }
        let url = format!("{}/p/{}", self.base_url, slug);
        if let Err(error) = open::that(&url) {
            self.toast(format!("打开用户网页失败：{error}"), ToastKind::Error);
        }
    }

    pub fn load_root(&mut self) {
        if self.root_loading {
            return;
        }
        self.root_loading = true;
        self.request(PendingKind::RootLoad, None, "GET", "/api/config", json!({}));
    }

    pub fn select_project(&mut self, id: &str) {
        if id == self.selected_project_id {
            return;
        }
        if self.config_loaded
            && (self.config != self.saved_config || self.saving || self.save_flight.is_some())
        {
            self.toast("当前项目配置正在保存，请稍候再切换项目", ToastKind::Info);
            return;
        }
        self.selected_project_id = id.to_owned();
        self.reset_project_view();
        self.sync_project_meta();
        if !id.is_empty() {
            self.load_project();
        }
    }

    fn reset_project_view(&mut self) {
        self.config = json!({});
        self.saved_config = json!({});
        self.status = json!({});
        self.job = Value::Null;
        self.check_job = Value::Null;
        self.vcs_job = Value::Null;
        self.outputs.clear();
        self.profiles.clear();
        self.scan_summary.clear();
        self.ignored_profiles.clear();
        self.unity_version.clear();
        self.unity_options.clear();
        self.build_selection.clear();
        self.build_group = "all".into();
        self.build_rows.clear();
        self.build_progress.clear();
        self.build_log.clear();
        self.build_drawer_open = false;
        self.check_log.clear();
        self.vcs_log.clear();
        self.check_targets = vec![19, 13];
        self.check_results.clear();
        self.check_run_dir.clear();
        self.vcs_states.clear();
        self.config_loaded = false;
        self.project_loading = false;
        self.last_edit = None;
        self.save_error = None;
        self.settings_paths = None;
        self.settings_path_asset.clear();
        self.settings_project_paths.clear();
        self.settings_paths_loading = false;
        self.build_editor = None;
        self.vcs_editor = None;
        self.preview = None;
        self.ai = None;
        self.monitor_artifacts = None;
        self.monitor_paths = None;
        self.monitor_path_asset.clear();
        self.monitor_selected = self.selected_project_id.clone();
    }

    pub fn load_project(&mut self) {
        let id = self.selected_project_id.clone();
        if id.is_empty() {
            return;
        }
        self.project_loading = true;
        self.request(
            PendingKind::ProjectLoad,
            Some(id),
            "GET",
            "/api/status",
            json!({}),
        );
    }

    pub fn reload_project_meta(&mut self) {
        self.sync_project_meta();
    }

    fn sync_project_meta(&mut self) {
        if self.project_meta_for == self.selected_project_id {
            return;
        }
        let meta = self.current_project().map(|project| {
            (
                project["name"].as_str().unwrap_or("").to_owned(),
                project["slug"].as_str().unwrap_or("").to_owned(),
                project["enabled"].as_bool().unwrap_or(true),
            )
        });
        if let Some((name, slug, enabled)) = meta {
            self.project_meta_name = name;
            self.project_meta_slug = slug;
            self.project_meta_enabled = enabled;
            self.project_meta_for = self.selected_project_id.clone();
        }
    }

    fn handle_response(&mut self, id: u64, result: Result<Value, AdminError>) {
        let Some(entry) = self.pending.remove(&id) else {
            return;
        };
        if matches!(
            entry.kind,
            PendingKind::SaveProject
                | PendingKind::ProjectLoad
                | PendingKind::Scan
                | PendingKind::BuildPreview
                | PendingKind::BuildStart
                | PendingKind::StopBuild
                | PendingKind::AiAnalyze
                | PendingKind::Outputs
                | PendingKind::ApplicationPaths { .. }
                | PendingKind::ProjectPaths { .. }
                | PendingKind::ArtifactsList
                | PendingKind::OpenArtifactFolder { .. }
                | PendingKind::OpenFolder
                | PendingKind::AndroidScan
                | PendingKind::UnityDetect
                | PendingKind::CliProbe
                | PendingKind::CliOpen
                | PendingKind::CheckPreview
                | PendingKind::CheckStart
                | PendingKind::StopCheck
                | PendingKind::VcsProbeAll
                | PendingKind::VcsProbeEditor
                | PendingKind::VcsStart
                | PendingKind::StopVcs
                | PendingKind::ProfileRead { .. }
                | PendingKind::ProfileWrite { .. }
        ) && entry.project_id.as_deref() != Some(self.selected_project_id.as_str())
            && !matches!(
                entry.kind,
                PendingKind::ArtifactsList | PendingKind::ProjectPaths { .. }
            )
        {
            return;
        }

        if let Err(error) = &result {
            self.handle_error(&entry.kind, error);
            return;
        }
        let value = result.unwrap_or(Value::Null);
        match entry.kind {
            PendingKind::RootLoad => {
                self.root_loading = false;
                self.root_loaded = true;
                self.root_config = value["config"].clone();
                if !self.root_config.is_object() {
                    self.root_config = value;
                }
                self.projects = self.root_config["projects"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                self.root_host = self.root_config["server"]["host"]
                    .as_str()
                    .unwrap_or("127.0.0.1")
                    .to_owned();
                self.root_port = self.root_config["server"]["port"]
                    .as_u64()
                    .unwrap_or(50345)
                    .to_string();
                if self.selected_project_id.is_empty()
                    || !self
                        .projects
                        .iter()
                        .any(|p| p["id"].as_str() == Some(self.selected_project_id.as_str()))
                {
                    self.selected_project_id = self
                        .projects
                        .first()
                        .and_then(|p| p["id"].as_str())
                        .unwrap_or("")
                        .to_owned();
                    self.project_meta_for.clear();
                    self.reset_project_view();
                    if !self.selected_project_id.is_empty() {
                        self.load_project();
                    }
                }
                self.sync_project_meta();
            }
            PendingKind::ProjectLoad => {
                self.project_loading = false;
                self.config = value["config"].clone();
                self.saved_config = self.config.clone();
                if !self.config.is_object() {
                    self.config = json!({});
                    self.saved_config = json!({});
                }
                self.status = value.clone();
                self.job = value["job"].clone();
                self.check_job = value["check"].clone();
                self.vcs_job = value["vcs"].clone();
                self.outputs = value["outputs"].as_array().cloned().unwrap_or_default();
                self.unity_options = value["unityExes"].as_array().cloned().unwrap_or_default();
                self.config_loaded = true;
                self.save_error = None;
                self.last_edit = None;
                self.project_meta_for.clear();
                self.sync_project_meta();
                self.initialize_build_selection();
                self.initialize_check_targets();
                self.populate_runtime_from_status();
                if !self.cfg_str(&["projectPath"]).trim().is_empty() {
                    self.start_scan();
                }
                self.load_settings_paths();
                let nodes = self.vcs_nodes();
                if !nodes.is_empty() {
                    self.probe_vcs_all();
                }
                if self.monitor_selected.is_empty() {
                    self.monitor_selected = self.selected_project_id.clone();
                }
            }
            PendingKind::SaveProject => {
                self.saving = false;
                let Some((flight_id, sent)) = self.save_flight.take() else {
                    return;
                };
                if flight_id != id {
                    return;
                }
                self.saved_config = sent;
                self.save_error = None;
                self.last_edit = None;
                if let Some(action) = self.queued_action.take() {
                    if self.config == self.saved_config {
                        self.execute_deferred(action);
                    } else {
                        self.queued_action = Some(action);
                        self.save_now();
                    }
                }
            }
            PendingKind::RootSave {
                body,
                message,
                next,
            } => {
                self.root_config = body.clone();
                self.projects = body["projects"].as_array().cloned().unwrap_or_default();
                self.saved_config = self.config.clone();
                if !message.is_empty() {
                    self.toast(message, ToastKind::Ok);
                }
                match next {
                    RootNext::Select(project_id) => {
                        self.selected_project_id = project_id;
                        self.project_meta_for.clear();
                        self.reset_project_view();
                        self.sync_project_meta();
                        if !self.selected_project_id.is_empty() {
                            self.load_project();
                        }
                    }
                    RootNext::Clear => {
                        self.selected_project_id.clear();
                        self.project_meta_for.clear();
                        self.reset_project_view();
                    }
                    RootNext::Keep => {}
                }
                self.load_root();
            }
            PendingKind::Scan => {
                self.profiles = value["profiles"].as_array().cloned().unwrap_or_default();
                self.ignored_profiles = value["ignored"].as_array().cloned().unwrap_or_default();
                self.unity_version = value["unityVersion"].as_str().unwrap_or("").to_owned();
                let abs = value["projectAbs"].as_str().unwrap_or("");
                let fallback = self
                    .profiles
                    .iter()
                    .any(|p| p["fallback"].as_bool() == Some(true));
                self.scan_summary = format!(
                    "项目：{abs}\nUnity：{} | Profile：{}{}",
                    if self.unity_version.is_empty() {
                        "未识别"
                    } else {
                        &self.unity_version
                    },
                    self.profiles.len(),
                    if fallback {
                        "（旧版工程，使用项目默认配置）"
                    } else {
                        ""
                    }
                );
                self.sync_nodes_with_profiles();
                self.initialize_build_selection();
                self.toast(
                    if fallback {
                        "扫描完成：旧版 Unity 工程已使用项目默认配置".into()
                    } else {
                        format!("扫描完成，发现 {} 个 Profile", self.profiles.len())
                    },
                    ToastKind::Ok,
                );
            }
            PendingKind::BuildPreview => {
                let items = value["items"].as_array().cloned().unwrap_or_default();
                self.preview = Some(PreviewDialog {
                    title: "构建命令预览".into(),
                    items,
                });
            }
            PendingKind::BuildStart => {
                self.toast("构建任务已提交", ToastKind::Ok);
                self.build_log
                    .push("[系统] 构建任务已提交，等待 Unity 输出日志…\n");
                self.job = json!({"state":"queued"});
            }
            PendingKind::StopBuild => {
                self.toast("已请求终止构建", ToastKind::Info);
            }
            PendingKind::AiAnalyze => {
                if let Some(ai) = &mut self.ai {
                    ai.loading = false;
                    ai.text = value["analysis"].as_str().unwrap_or("").to_owned();
                }
            }
            PendingKind::Outputs => {
                self.outputs = value["outputs"].as_array().cloned().unwrap_or_default();
            }
            PendingKind::ApplicationPaths {
                scope,
                asset_path: _,
            } => match scope {
                PathScope::Active => {
                    self.settings_paths_loading = false;
                    if self.settings_path_asset.is_empty() {
                        self.settings_path_asset =
                            value["assetPath"].as_str().unwrap_or("").to_owned();
                    }
                    self.settings_paths = Some(value);
                }
                PathScope::Monitor => {
                    self.monitor_paths_loading = false;
                    if self.monitor_path_asset.is_empty() {
                        self.monitor_path_asset =
                            value["assetPath"].as_str().unwrap_or("").to_owned();
                    }
                    self.monitor_paths = Some(value);
                }
            },
            PendingKind::ProjectPaths { scope } => {
                let paths = value["paths"].as_array().cloned().unwrap_or_default();
                if matches!(scope, PathScope::Active) {
                    self.settings_project_paths = paths;
                }
            }
            PendingKind::ArtifactsList => {
                self.monitor_artifact_loading = false;
                self.monitor_artifacts = Some(value);
            }
            PendingKind::OpenArtifactFolder { .. }
            | PendingKind::ApplicationPathOpen
            | PendingKind::ProjectPathOpen
            | PendingKind::OpenFolder => {
                self.toast("已打开目录", ToastKind::Ok);
            }
            PendingKind::AndroidScan => {
                if !value["sdk"].is_null() {
                    self.set_cfg(&["androidTools", "sdk"], value["sdk"].clone());
                }
                if !value["ndk"].is_null() {
                    self.set_cfg(&["androidTools", "ndk"], value["ndk"].clone());
                }
                if !value["jdk"].is_null() {
                    self.set_cfg(&["androidTools", "jdk"], value["jdk"].clone());
                }
                let missing = ["sdk", "ndk", "jdk"]
                    .iter()
                    .filter(|key| value[key].is_null())
                    .copied()
                    .collect::<Vec<_>>();
                if missing.len() == 3 {
                    self.toast("未自动找到 Android 工具目录，请手动填写", ToastKind::Error);
                } else {
                    self.toast(
                        format!(
                            "Android 工具目录已扫描{}",
                            if missing.is_empty() {
                                String::new()
                            } else {
                                format!("，未找到：{}", missing.join(" / "))
                            }
                        ),
                        ToastKind::Info,
                    );
                }
            }
            PendingKind::UnityDetect => {
                self.unity_options = value["exes"].as_array().cloned().unwrap_or_default();
                self.toast(
                    format!("Unity 引擎探测完成，共 {} 个", self.unity_options.len()),
                    ToastKind::Ok,
                );
            }
            PendingKind::CliProbe => {
                self.unity_options = value["exes"].as_array().cloned().unwrap_or_default();
                self.toast(
                    format!(
                        "Unity CLI：{}{}",
                        value["cli"].as_str().unwrap_or("unity"),
                        value["version"]
                            .as_str()
                            .filter(|v| !v.trim().is_empty())
                            .map(|v| format!(" · {}", v.trim()))
                            .unwrap_or_default()
                    ),
                    if value["ok"].as_bool() == Some(true) {
                        ToastKind::Ok
                    } else {
                        ToastKind::Info
                    },
                );
            }
            PendingKind::CliOpen => {
                self.toast("已启动 Unity 编辑器", ToastKind::Ok);
            }
            PendingKind::CheckPreview => {
                let items = value["items"].as_array().cloned().unwrap_or_default();
                self.preview = Some(PreviewDialog {
                    title: "环境检测命令预览".into(),
                    items,
                });
            }
            PendingKind::CheckStart => {
                self.check_run_dir = value["runDir"].as_str().unwrap_or("").to_owned();
                self.check_job = json!({"state":"starting"});
                self.toast("环境编译检测已启动", ToastKind::Ok);
            }
            PendingKind::StopCheck => {
                self.toast("已请求终止环境检测", ToastKind::Info);
            }
            PendingKind::VcsProbeAll | PendingKind::VcsProbeEditor => {
                let results = value["results"].as_array().cloned().unwrap_or_default();
                if matches!(entry.kind, PendingKind::VcsProbeEditor) {
                    if let Some(editor) = &mut self.vcs_editor {
                        if let Some(result) = results.first() {
                            editor.branches = result["branches"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect();
                            if result["type"].as_str() == Some("git")
                                || result["type"].as_str() == Some("svn")
                            {
                                let kind = result["type"].as_str().unwrap_or("").to_uppercase();
                                let mut parts = Vec::new();
                                if let Some(branch) = result["branch"].as_str() {
                                    if !branch.is_empty() {
                                        parts.push(format!("分支 {branch}"));
                                    }
                                }
                                if let Some(revision) = result["revision"].as_str() {
                                    if !revision.is_empty() {
                                        parts.push(format!("#{revision}"));
                                    }
                                }
                                if result["dirty"].as_bool() == Some(true) {
                                    parts.push(format!(
                                        "{} 处未提交",
                                        result["dirtyCount"].as_u64().unwrap_or(0)
                                    ));
                                }
                                if result["hasRemote"].as_bool() == Some(false) {
                                    parts.push("无远端".into());
                                } else if result["remoteOnline"].as_bool() == Some(true) {
                                    parts.push("远端在线".into());
                                }
                                editor.probe_ok = true;
                                editor.probe_text = format!("检测到 {kind}：{}", parts.join(" · "));
                            } else {
                                editor.probe_ok = false;
                                editor.probe_text = result["error"]
                                    .as_str()
                                    .filter(|v| !v.is_empty())
                                    .unwrap_or("未检测到 Git / SVN 工作副本")
                                    .to_owned();
                            }
                        }
                    }
                } else {
                    for result in results {
                        if let Some(id) = result["id"].as_str() {
                            self.vcs_states.insert(id.to_owned(), result);
                        }
                    }
                }
            }
            PendingKind::VcsStart => {
                self.toast("版本更新已启动", ToastKind::Ok);
                self.vcs_job = json!({"state":"starting"});
            }
            PendingKind::StopVcs => {
                self.toast("已请求终止版本更新", ToastKind::Info);
            }
            PendingKind::ProfileRead { asset_path } => {
                if let Some(editor) = self.build_editor.as_mut() {
                    if let Some(player) = editor.player.as_mut() {
                        if player.asset_path == asset_path {
                            player.loading = false;
                            player.product_name =
                                value["productName"].as_str().unwrap_or("").to_owned();
                            player.company_name =
                                value["companyName"].as_str().unwrap_or("").to_owned();
                            player.bundle_version =
                                value["bundleVersion"].as_str().unwrap_or("").to_owned();
                            player.revision = value["revision"].as_str().unwrap_or("").to_owned();
                            player.editable = value["editable"].as_bool().unwrap_or(false);
                            player.inherited = value["inherited"].as_bool().unwrap_or(false);
                            player.fallback = value["fallback"].as_bool().unwrap_or(false);
                            player.path = value["path"].as_str().unwrap_or("").to_owned();
                            player.error.clear();
                        }
                    }
                }
            }
            PendingKind::ProfileWrite { asset_path } => {
                if let Some(editor) = self.build_editor.as_mut() {
                    if let Some(player) = editor.player.as_mut() {
                        if player.asset_path == asset_path {
                            player.loading = false;
                            player.product_name =
                                value["productName"].as_str().unwrap_or("").to_owned();
                            player.company_name =
                                value["companyName"].as_str().unwrap_or("").to_owned();
                            player.bundle_version =
                                value["bundleVersion"].as_str().unwrap_or("").to_owned();
                            player.revision = value["revision"].as_str().unwrap_or("").to_owned();
                            player.inherited = value["inherited"].as_bool().unwrap_or(false);
                            player.error.clear();
                            self.toast("Player Settings 已写入文件", ToastKind::Ok);
                        }
                    }
                }
            }
        }
    }

    fn handle_error(&mut self, kind: &PendingKind, error: &AdminError) {
        match kind {
            PendingKind::RootLoad => {
                self.root_loading = false;
            }
            PendingKind::ProjectLoad => {
                self.project_loading = false;
            }
            PendingKind::SaveProject => {
                self.saving = false;
                self.save_flight = None;
                self.save_error = Some(error.message.clone());
                if self.queued_action.take().is_some() {
                    self.toast("配置保存失败，已取消任务", ToastKind::Error);
                }
            }
            PendingKind::ApplicationPaths { scope, .. } => match scope {
                PathScope::Active => self.settings_paths_loading = false,
                PathScope::Monitor => self.monitor_paths_loading = false,
            },
            PendingKind::ArtifactsList => self.monitor_artifact_loading = false,
            PendingKind::AiAnalyze => {
                if let Some(ai) = &mut self.ai {
                    ai.loading = false;
                    ai.text = format!("分析失败：{}", error.message);
                }
            }
            PendingKind::ProfileRead { .. } | PendingKind::ProfileWrite { .. } => {
                if let Some(editor) = self.build_editor.as_mut() {
                    if let Some(player) = editor.player.as_mut() {
                        player.loading = false;
                        player.error = error.message.clone();
                    }
                }
            }
            PendingKind::VcsProbeEditor => {
                if let Some(editor) = &mut self.vcs_editor {
                    editor.probe_ok = false;
                    editor.probe_text = format!("探测失败：{}", error.message);
                }
            }
            _ => {}
        }
        self.toast(error.message.clone(), ToastKind::Error);
    }

    fn handle_backend_event(&mut self, event: Value) {
        let kind = event["type"].as_str().unwrap_or("");
        let event_project = event["projectId"].as_str().unwrap_or("");
        if !event_project.is_empty() && event_project != self.selected_project_id {
            return;
        }
        match kind {
            "job-queued" => {
                self.job = json!({"state":"queued"});
                self.build_drawer_open = true;
                self.build_log.push("[系统] 任务已进入全局构建队列\n");
            }
            "job-start" => {
                self.job = json!({"state":"running","count":event["count"]});
                self.build_progress.clear();
                self.build_rows.clear();
                if let Some(nodes) = self.job["queue"].as_array() {
                    self.build_rows = nodes.clone();
                }
                self.build_log.push("\n[系统] 批量构建已启动\n");
            }
            "profile-start" => {
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                self.build_progress
                    .insert(index, ("启动中…".into(), None, "running".into()));
                self.build_log.push(&format!(
                    "\n========== [{}/?] {} · {} ==========\n输出：{}\n命令：{}\n",
                    index + 1,
                    event["name"].as_str().unwrap_or(""),
                    event["target"].as_str().unwrap_or(""),
                    event["output"].as_str().unwrap_or(""),
                    event["cmd"].as_str().unwrap_or("")
                ));
            }
            "progress" => {
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                let stage = event["stage"].as_str().unwrap_or("处理中").to_owned();
                let pct = event["pct"].as_f64().map(|v| v as f32);
                self.build_progress
                    .insert(index, (stage, pct, "running".into()));
            }
            "line" | "notice" => {
                self.build_log.push(event["text"].as_str().unwrap_or(""));
            }
            "profile-end" => {
                let index = event["index"].as_u64().unwrap_or(0) as usize;
                let ok = event["ok"].as_bool().unwrap_or(false);
                let (stage, pct, _) = self
                    .build_progress
                    .get(&index)
                    .cloned()
                    .unwrap_or_else(|| ("".into(), None, "".into()));
                self.build_progress.insert(
                    index,
                    (
                        if ok {
                            "构建成功".into()
                        } else {
                            format!(
                                "失败（退出码 {}）",
                                event["exitCode"].as_i64().unwrap_or(-1)
                            )
                        },
                        if ok { Some(100.0) } else { pct },
                        if ok { "done".into() } else { "fail".into() },
                    ),
                );
                if stage.is_empty() && ok {
                    // Keep the structured stage even if no progress event arrived.
                }
            }
            "output" => {
                let file = event["file"].clone();
                self.outputs.retain(|o| o["file"] != file);
                self.outputs.insert(0, event);
            }
            "job-end" | "done" if kind == "job-end" || event.get("end").is_some() => {
                let end = event.get("end").unwrap_or(&event);
                self.job = json!({"state":"done","end":end});
                if end["ok"].as_bool() == Some(true) {
                    self.toast("批量构建完成", ToastKind::Ok);
                } else if end["reason"].as_str() == Some("cancelled") {
                    self.toast("构建已终止", ToastKind::Info);
                } else {
                    self.toast(
                        format!(
                            "构建结束：{}",
                            end["message"].as_str().unwrap_or("存在失败项")
                        ),
                        ToastKind::Error,
                    );
                }
                self.request(
                    PendingKind::Outputs,
                    Some(self.selected_project_id.clone()),
                    "GET",
                    "/api/outputs",
                    json!({}),
                );
            }
            "check-start" => {
                self.check_results.clear();
                self.check_job = json!({"state":"running"});
                self.check_run_dir = event["runDir"].as_str().unwrap_or("").to_owned();
                if let Some(targets) = event["targets"].as_array() {
                    self.check_targets = targets.iter().filter_map(Value::as_i64).collect();
                }
                for target in &self.check_targets {
                    self.check_results
                        .insert(*target, json!({"status":"running","message":"排队中…"}));
                }
            }
            "check-target-start" => {
                let target = event["target"].as_i64().unwrap_or(0);
                self.check_results.insert(
                    target,
                    json!({"status":"running","name":event["name"],"message":"检测中…（启动 Unity 编译）"}),
                );
            }
            "check-line" => self.check_log.push(event["text"].as_str().unwrap_or("")),
            "check-result" => {
                let target = event["target"].as_i64().unwrap_or(0);
                self.check_results.insert(target, event["result"].clone());
            }
            "check-end" => {
                let end = event["end"].clone();
                self.check_job = json!({"state":"done","end":end});
                if end["ok"].as_bool() == Some(true) {
                    self.toast("环境编译检测全部通过", ToastKind::Ok);
                } else if end["reason"].as_str() == Some("cancelled") {
                    self.toast("环境检测已终止", ToastKind::Info);
                } else {
                    self.toast("环境检测完成，存在未通过项", ToastKind::Error);
                }
            }
            "vcs-start" => {
                self.vcs_job = json!({"state":"running","count":event["count"]});
                self.vcs_log.push("\n[系统] 版本管理更新已启动\n");
            }
            "vcs-node-start" => {
                let node_id = event["nodeId"].as_str().unwrap_or("").to_owned();
                self.vcs_states.insert(
                    node_id,
                    json!({"status":"updating","message":"还原并更新中…"}),
                );
                self.vcs_log.push(&format!(
                    "\n▶ {}（{}）\n",
                    event["name"].as_str().unwrap_or(""),
                    event["path"].as_str().unwrap_or("路径为空")
                ));
            }
            "vcs-line" => self.vcs_log.push(event["text"].as_str().unwrap_or("")),
            "vcs-node-end" => {
                let node_id = event["nodeId"].as_str().unwrap_or("").to_owned();
                self.vcs_states.insert(node_id, event);
            }
            "vcs-end" => {
                let end = event["end"].clone();
                self.vcs_job = json!({"state":"done","end":end});
                if end["ok"].as_bool() == Some(true) {
                    self.toast("版本管理更新完成", ToastKind::Ok);
                } else if end["reason"].as_str() == Some("cancelled") {
                    self.toast("版本更新已终止", ToastKind::Info);
                } else {
                    self.toast("版本管理更新存在失败项", ToastKind::Error);
                }
                let ids = self.vcs_states.keys().cloned().collect::<Vec<_>>();
                if !ids.is_empty() {
                    self.probe_vcs_ids(ids);
                }
            }
            _ => {}
        }
    }

    fn handle_monitor(&mut self, result: Result<Value, String>) {
        match result {
            Ok(snapshot) => {
                self.monitor_error.clear();
                for (id, resource) in [("backend", &snapshot["backend"])].into_iter().chain(
                    snapshot["projects"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|project| {
                            (project["id"].as_str().unwrap_or(""), &project["resources"])
                        }),
                ) {
                    if id.is_empty() {
                        continue;
                    }
                    let history = self.monitor_history.entry(id.to_owned()).or_default();
                    history.push_back(MonitorSample {
                        cpu: resource["cpuPercent"].as_f64().unwrap_or(0.0) as f32,
                        memory: resource["memoryBytes"].as_u64().unwrap_or(0),
                    });
                    while history.len() > 60 {
                        history.pop_front();
                    }
                }
                if self.monitor_selected.is_empty() {
                    self.monitor_selected = snapshot["projects"]
                        .as_array()
                        .and_then(|projects| projects.first())
                        .and_then(|project| project["id"].as_str())
                        .unwrap_or("")
                        .to_owned();
                }
                self.monitor_snapshot = Some(snapshot);
            }
            Err(error) => {
                self.monitor_error = error;
            }
        }
    }

    fn handle_tray(&mut self, action: TrayAction, ctx: &egui::Context) {
        match action {
            TrayAction::Toggle => self.toggle_window(ctx),
            TrayAction::Website => self.open_user_website(),
            TrayAction::Quit => self.quit(ctx),
        }
    }

    fn toggle_window(&mut self, ctx: &egui::Context) {
        self.visible = !self.visible;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(self.visible));
        if self.visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
    }

    pub fn quit(&mut self, ctx: &egui::Context) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        self.monitor_running.store(false, Ordering::Release);
        if let Some(service) = self.service.take() {
            service.stop();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn initialize_build_selection(&mut self) {
        self.build_group = self.cfg_str(&["activeBuildGroupId"]);
        if self.build_group.is_empty() {
            self.build_group = "all".into();
        }
        let groups = self.build_groups();
        if self.build_group != "all"
            && !groups
                .iter()
                .any(|g| g["id"].as_str() == Some(self.build_group.as_str()))
        {
            self.build_group = "all".into();
        }
        let ids = self.active_group_node_ids();
        if self.build_selection.is_empty() {
            self.build_selection = ids.into_iter().collect();
        } else {
            let available = self
                .build_nodes()
                .iter()
                .filter_map(|node| node["id"].as_str().map(str::to_owned))
                .collect::<HashSet<_>>();
            self.build_selection.retain(|id| available.contains(id));
        }
    }

    fn initialize_check_targets(&mut self) {
        let targets = self.config["check"]["targets"]
            .as_array()
            .map(|targets| {
                targets
                    .iter()
                    .filter_map(Value::as_i64)
                    .filter(|target| ENV_TARGETS.iter().any(|(id, _)| id == target))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !targets.is_empty() {
            self.check_targets = targets;
        }
        if self.check_targets.is_empty() {
            self.check_targets = vec![19, 13];
        }
    }

    fn populate_runtime_from_status(&mut self) {
        if let Some(queue) = self.job["queue"].as_array() {
            self.build_rows = queue.clone();
        }
        if let Some(results) = self.check_job["results"].as_object() {
            self.check_results.clear();
            for (target, result) in results {
                if let Ok(target) = target.parse::<i64>() {
                    self.check_results.insert(target, result.clone());
                }
            }
            self.check_run_dir = self.check_job["runDir"].as_str().unwrap_or("").to_owned();
        }
        if let Some(results) = self.vcs_job["results"].as_object() {
            self.vcs_states = results
                .iter()
                .map(|(id, result)| (id.clone(), result.clone()))
                .collect();
        }
    }

    fn tick_save(&mut self) {
        if !self.config_loaded || self.selected_project_id.is_empty() {
            return;
        }
        if self.config == self.saved_config {
            self.last_edit = None;
            return;
        }
        let Some(edited) = self.last_edit else {
            self.last_edit = Some(Instant::now());
            return;
        };
        if self.saving
            || self.save_flight.is_some()
            || self.busy()
            || self.save_error.is_some()
            || edited.elapsed() < SAVE_DEBOUNCE
        {
            return;
        }
        self.save_now();
    }

    pub fn save_now(&mut self) {
        if !self.config_loaded || self.selected_project_id.is_empty() || self.busy() {
            if self.busy() {
                self.toast("任务运行或排队期间不能保存项目配置", ToastKind::Error);
            }
            return;
        }
        if self.config == self.saved_config {
            if let Some(action) = self.queued_action.take() {
                self.execute_deferred(action);
            }
            return;
        }
        let body = self.config.clone();
        let Some(id) = self.request(
            PendingKind::SaveProject,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/config",
            body.clone(),
        ) else {
            return;
        };
        self.saving = true;
        self.save_flight = Some((id, body));
        self.save_error = None;
    }

    pub fn retry_save(&mut self) {
        self.save_error = None;
        self.last_edit = Some(Instant::now() - SAVE_DEBOUNCE);
    }

    pub fn ensure_saved_then(&mut self, action: DeferredAction) {
        if self.busy() {
            self.toast("已有构建、检测或仓库任务运行中", ToastKind::Error);
            return;
        }
        if self.config != self.saved_config || self.saving || self.save_flight.is_some() {
            self.queued_action = Some(action);
            if !self.saving && self.save_flight.is_none() {
                self.save_now();
            }
            return;
        }
        self.execute_deferred(action);
    }

    fn execute_deferred(&mut self, action: DeferredAction) {
        match action {
            DeferredAction::StartBuild(body) => {
                self.build_log.clear();
                self.build_log.push("[系统] 正在启动批量构建…\n");
                self.build_progress.clear();
                self.build_drawer_open = true;
                self.request(
                    PendingKind::BuildStart,
                    Some(self.selected_project_id.clone()),
                    "POST",
                    "/api/build/start",
                    body,
                );
            }
            DeferredAction::StartCheck(body) => {
                self.check_results.clear();
                self.check_log.clear();
                self.request(
                    PendingKind::CheckStart,
                    Some(self.selected_project_id.clone()),
                    "POST",
                    "/api/check/start",
                    body,
                );
            }
            DeferredAction::StartVcs(body) => {
                self.vcs_log.clear();
                self.request(
                    PendingKind::VcsStart,
                    Some(self.selected_project_id.clone()),
                    "POST",
                    "/api/vcs/update",
                    body,
                );
            }
        }
    }

    pub fn cfg_str(&self, keys: &[&str]) -> String {
        self.cfg_value(keys).as_str().unwrap_or("").to_owned()
    }

    pub fn cfg_bool(&self, keys: &[&str], default: bool) -> bool {
        self.cfg_value(keys).as_bool().unwrap_or(default)
    }

    pub fn cfg_i64(&self, keys: &[&str], default: i64) -> i64 {
        let current = self.cfg_value(keys);
        current
            .as_i64()
            .or_else(|| current.as_str().and_then(|v| v.trim().parse().ok()))
            .unwrap_or(default)
    }

    pub fn cfg_value(&self, keys: &[&str]) -> &Value {
        let mut current = &self.config;
        for key in keys {
            current = current.get(*key).unwrap_or(&Value::Null);
        }
        current
    }

    pub fn set_cfg(&mut self, keys: &[&str], value: Value) {
        if keys.is_empty() || self.config == self.saved_config && !self.config_loaded {
            return;
        }
        let slot = nested_mut(&mut self.config, keys);
        if *slot != value {
            *slot = value;
            self.mark_dirty();
        }
    }

    pub fn mark_dirty(&mut self) {
        self.last_edit = Some(Instant::now());
        self.save_error = None;
    }

    pub fn build_nodes(&self) -> Vec<Value> {
        self.config["buildNodes"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    pub fn build_groups(&self) -> Vec<Value> {
        self.config["buildGroups"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    pub fn vcs_nodes(&self) -> Vec<Value> {
        self.config["vcs"]["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    pub fn vcs_groups(&self) -> Vec<Value> {
        self.config["vcs"]["groups"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    pub fn group_of_node(&self, node_id: &str) -> Option<Value> {
        self.build_groups().into_iter().find(|group| {
            group["nodeIds"]
                .as_array()
                .map(|ids| ids.iter().any(|id| id.as_str() == Some(node_id)))
                .unwrap_or(false)
        })
    }

    pub fn ungrouped_build_nodes(&self) -> Vec<Value> {
        self.build_nodes()
            .into_iter()
            .filter(|node| {
                node["id"]
                    .as_str()
                    .map(|id| self.group_of_node(id).is_none())
                    .unwrap_or(false)
            })
            .collect()
    }

    pub fn active_group_node_ids(&self) -> Vec<String> {
        let nodes = self.build_nodes();
        if self.build_group == "all" {
            return nodes
                .iter()
                .filter_map(|node| node["id"].as_str().map(str::to_owned))
                .collect();
        }
        self.build_groups()
            .into_iter()
            .find(|group| group["id"].as_str() == Some(self.build_group.as_str()))
            .and_then(|group| group["nodeIds"].as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .filter(|id| nodes.iter().any(|node| node["id"].as_str() == Some(id)))
            .collect()
    }

    pub fn selected_build_nodes(&self) -> Vec<Value> {
        self.build_nodes()
            .into_iter()
            .filter(|node| {
                node["id"]
                    .as_str()
                    .map(|id| self.build_selection.contains(id))
                    .unwrap_or(false)
            })
            .collect()
    }

    pub fn build_body(&self) -> Value {
        let defaults = &self.config;
        let profiles = self
            .selected_build_nodes()
            .into_iter()
            .map(|node| {
                let profile = self
                    .profiles
                    .iter()
                    .find(|profile| {
                        profile["assetPath"].as_str()
                            == node["profileAssetPath"].as_str()
                    })
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let target = node["target"]
                    .as_i64()
                    .or_else(|| profile["target"].as_i64())
                    .unwrap_or(19);
                let target_name = node["targetName"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .or_else(|| profile["targetName"].as_str())
                    .unwrap_or(if target == 13 {
                        "Android"
                    } else {
                        "Windows (x64)"
                    });
                let asset_path = node["profileAssetPath"]
                    .as_str()
                    .or_else(|| profile["assetPath"].as_str())
                    .unwrap_or("");
                let custom_build_dir = node["customBuildDir"].as_str().unwrap_or("");
                let name_template = node["nameTemplate"].as_str().unwrap_or("");
                let auto_zip = node["autoZip"]
                    .as_bool()
                    .unwrap_or(defaults["defaultAutoZip"].as_bool().unwrap_or(true));
                let dev = node["dev"]
                    .as_bool()
                    .unwrap_or(defaults["defaultDevBuild"].as_bool().unwrap_or(false));
                let addressables = node["buildAddressables"].as_bool().unwrap_or(
                    defaults["defaultBuildAddressables"]
                        .as_bool()
                        .unwrap_or(false),
                );
                let addressables_method = node["addressablesMethod"].as_str().unwrap_or("");
                let android_kind = node["androidBuildKind"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| {
                        defaults["defaultAndroidBuildKind"]
                            .as_str()
                            .unwrap_or("apk")
                    });
                json!({
                    "id": node["id"],
                    "name": node["name"].as_str().filter(|v| !v.is_empty()).or_else(|| profile["name"].as_str()).unwrap_or("未命名节点"),
                    "target": target,
                    "targetName": target_name,
                    "assetPath": asset_path,
                    "absPath": node["absPath"].as_str().filter(|v| !v.is_empty()).or_else(|| profile["absPath"].as_str()).unwrap_or(""),
                    "productName": node["productName"].as_str().filter(|v| !v.is_empty()).or_else(|| profile["productName"].as_str()),
                    "subtarget": node["subtarget"].as_i64().or_else(|| profile["subtarget"].as_i64()).unwrap_or(0),
                    "sceneCount": node["sceneCount"].as_i64().or_else(|| profile["sceneCount"].as_i64()).unwrap_or(0),
                    "bundleVersion": node["bundleVersion"].as_str().filter(|v| !v.is_empty()).or_else(|| profile["bundleVersion"].as_str()),
                    "customBuildDir": custom_build_dir,
                    "nameTemplate": name_template,
                    "autoZip": auto_zip,
                    "dev": dev,
                    "buildAddressables": addressables,
                    "addressablesMethod": addressables_method,
                    "androidBuildKind": android_kind,
                    "vcsGroupIds": node["vcsGroupIds"].clone(),
                    "sign": {
                        "keystoreName": node["keystoreName"].as_str().filter(|v| !v.is_empty()).or_else(|| profile["keystoreName"].as_str()).unwrap_or(""),
                        "keyaliasName": node["keyaliasName"].as_str().filter(|v| !v.is_empty()).or_else(|| profile["keyaliasName"].as_str()).unwrap_or(""),
                        "keystorePass": node["keystorePass"].as_str().unwrap_or(""),
                        "keyaliasPass": node["keyaliasPass"].as_str().unwrap_or(""),
                        "remember": node["remember"].as_bool().unwrap_or(false),
                        "customBuildDir": custom_build_dir,
                        "nameTemplate": name_template,
                        "autoZip": auto_zip,
                        "dev": dev,
                        "buildAddressables": addressables,
                        "addressablesMethod": addressables_method,
                        "androidBuildKind": android_kind,
                    },
                })
            })
            .collect::<Vec<_>>();
        json!({
            "projectPath": self.cfg_str(&["projectPath"]),
            "profiles": profiles,
            "engine": self.config["engine"].clone(),
            "outputBase": self.cfg_str(&["outputBase"]),
            "successDir": self.cfg_str(&["successDir"]),
            "artifactNameTemplate": self.cfg_str(&["artifactNameTemplate"]),
            "autoZip": self.cfg_bool(&["defaultAutoZip"], true),
            "dev": self.cfg_bool(&["defaultDevBuild"], false),
            "buildAddressables": self.cfg_bool(&["defaultBuildAddressables"], false),
            "androidBuildKind": self.cfg_str(&["defaultAndroidBuildKind"]),
            "stopOnError": self.cfg_bool(&["stopOnError"], true),
            "buildNumber": self.cfg_str(&["buildNumber"]),
            "proxy": self.config["proxy"].clone(),
        })
    }

    pub fn check_body(&self) -> Value {
        json!({
            "projectPath": self.cfg_str(&["check", "projectPath"]),
            "engine": {
                "unityExe": self.cfg_str(&["check", "unityExe"]),
                "nographics": self.cfg_bool(&["check", "nographics"], true),
            },
            "targets": self.check_targets,
            "timeoutMinutes": self.cfg_i64(&["check", "timeoutMinutes"], 20).max(1),
        })
    }

    pub fn save_check_targets(&mut self) {
        self.set_cfg(&["check", "targets"], json!(self.check_targets));
    }

    pub fn start_scan(&mut self) {
        let body = json!({
            "projectPath": self.cfg_str(&["projectPath"]),
            "profileDir": self.cfg_str(&["profileDir"]),
        });
        self.request(
            PendingKind::Scan,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/scan",
            body,
        );
    }

    fn sync_nodes_with_profiles(&mut self) -> bool {
        let profiles = self.profiles.clone();
        let changed = sync_profile_metadata(&mut self.config, &profiles);
        if changed {
            // Scanning refreshes metadata for display and build-time resolution, but it
            // must not rewrite config.json before the user actually edits something.
            sync_profile_metadata(&mut self.saved_config, &profiles);
        }
        changed
    }

    pub fn probe_vcs_all(&mut self) {
        let ids = self
            .vcs_nodes()
            .iter()
            .filter_map(|node| node["id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        if !ids.is_empty() {
            self.probe_vcs_ids(ids);
        }
    }

    pub fn probe_vcs_ids(&mut self, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        self.request(
            PendingKind::VcsProbeAll,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/vcs/probe",
            json!({"ids":ids}),
        );
    }

    pub fn vcs_group_of(&self, node_id: &str) -> Option<Value> {
        self.vcs_groups().into_iter().find(|group| {
            group["nodeIds"]
                .as_array()
                .map(|ids| ids.iter().any(|id| id.as_str() == Some(node_id)))
                .unwrap_or(false)
        })
    }

    pub fn ungrouped_vcs_nodes(&self) -> Vec<Value> {
        self.vcs_nodes()
            .into_iter()
            .filter(|node| {
                node["id"]
                    .as_str()
                    .map(|id| self.vcs_group_of(id).is_none())
                    .unwrap_or(false)
            })
            .collect()
    }

    pub fn load_settings_paths(&mut self) {
        let id = self.selected_project_id.clone();
        if id.is_empty() {
            return;
        }
        self.settings_paths_loading = true;
        self.request(
            PendingKind::ApplicationPaths {
                scope: PathScope::Active,
                asset_path: self.settings_path_asset.clone(),
            },
            Some(id.clone()),
            "POST",
            "/api/application-paths/read",
            json!({"assetPath":self.settings_path_asset}),
        );
        self.request(
            PendingKind::ProjectPaths {
                scope: PathScope::Active,
            },
            Some(id),
            "GET",
            "/api/project-paths",
            json!({}),
        );
    }

    pub fn load_monitor_paths(&mut self) {
        let id = self.monitor_selected.clone();
        if id.is_empty() {
            return;
        }
        self.monitor_paths_loading = true;
        self.request(
            PendingKind::ApplicationPaths {
                scope: PathScope::Monitor,
                asset_path: String::new(),
            },
            Some(id),
            "POST",
            "/api/application-paths/read",
            json!({"assetPath":self.monitor_path_asset}),
        );
    }

    pub fn load_artifacts(&mut self, root: &str, path: &str) {
        let id = self.monitor_selected.clone();
        if id.is_empty() {
            return;
        }
        self.monitor_artifact_loading = true;
        self.request(
            PendingKind::ArtifactsList,
            Some(id),
            "POST",
            "/api/artifacts/list",
            json!({"root":root,"path":path}),
        );
    }

    pub fn open_artifact_folder(&mut self, root: String, path: String) {
        let id = self.monitor_selected.clone();
        if id.is_empty() {
            return;
        }
        self.request(
            PendingKind::OpenArtifactFolder {
                root: root.clone(),
                path: path.clone(),
            },
            Some(id),
            "POST",
            "/api/artifacts/open-folder",
            json!({"root":root,"path":path}),
        );
    }

    pub fn open_application_path(&mut self, scope: PathScope, asset_path: String, id: String) {
        let project_id = match scope {
            PathScope::Active => self.selected_project_id.clone(),
            PathScope::Monitor => self.monitor_selected.clone(),
        };
        if project_id.is_empty() {
            return;
        }
        self.request(
            PendingKind::ApplicationPathOpen,
            Some(project_id),
            "POST",
            "/api/application-paths/open",
            json!({"assetPath":asset_path,"id":id}),
        );
    }

    pub fn open_project_path(&mut self, id: &str) {
        let project_id = self.selected_project_id.clone();
        if project_id.is_empty() {
            return;
        }
        self.request(
            PendingKind::ProjectPathOpen,
            Some(project_id),
            "POST",
            "/api/project-paths/open",
            json!({"id":id}),
        );
    }

    pub fn refresh_outputs(&mut self) {
        self.request(
            PendingKind::Outputs,
            Some(self.selected_project_id.clone()),
            "GET",
            "/api/outputs",
            json!({}),
        );
    }

    pub fn select_monitor_project(&mut self, id: &str) {
        if self.monitor_selected == id {
            return;
        }
        self.monitor_selected = id.to_owned();
        self.monitor_path_asset.clear();
        self.monitor_artifacts = None;
        self.monitor_paths = None;
        self.load_monitor_paths();
        self.load_artifacts("", "");
    }

    pub fn open_folder(&mut self, project_id: Option<String>, path: &str) {
        self.request(
            PendingKind::OpenFolder,
            project_id,
            "POST",
            "/api/open-folder",
            json!({"path":path}),
        );
    }

    pub fn add_build_group(&mut self) {
        let mut groups = self.build_groups();
        let id = unique_id("bg");
        groups.push(json!({
            "id": id,
            "name": format!("构建组 {}", groups.len() + 1),
            "nodeIds": []
        }));
        self.set_cfg(&["buildGroups"], Value::Array(groups));
        self.build_group = id;
        self.set_cfg(&["activeBuildGroupId"], json!(self.build_group));
    }

    pub fn add_vcs_group(&mut self) {
        let mut groups = self.vcs_groups();
        groups.push(json!({
            "id": unique_id("vg"),
            "name": format!("版本分组 {}", groups.len() + 1),
            "nodeIds": []
        }));
        self.set_cfg(&["vcs", "groups"], Value::Array(groups));
    }

    pub fn move_build_node(&mut self, node_id: &str, group_id: Option<&str>) {
        self.move_build_node_before(node_id, group_id, None);
    }

    pub fn move_build_node_before(
        &mut self,
        node_id: &str,
        group_id: Option<&str>,
        before_node_id: Option<&str>,
    ) {
        let mut groups = self.build_groups();
        let mut nodes = self.build_nodes();
        if reorder_node(&mut nodes, &mut groups, node_id, group_id, before_node_id) {
            self.set_cfg(&["buildNodes"], Value::Array(nodes));
        }
        self.set_cfg(&["buildGroups"], Value::Array(groups));
    }

    pub fn move_vcs_node(&mut self, node_id: &str, group_id: Option<&str>) {
        self.move_vcs_node_before(node_id, group_id, None);
    }

    pub fn move_vcs_node_before(
        &mut self,
        node_id: &str,
        group_id: Option<&str>,
        before_node_id: Option<&str>,
    ) {
        let mut groups = self.vcs_groups();
        let mut nodes = self.vcs_nodes();
        if reorder_node(&mut nodes, &mut groups, node_id, group_id, before_node_id) {
            self.set_cfg(&["vcs", "nodes"], Value::Array(nodes));
        }
        self.set_cfg(&["vcs", "groups"], Value::Array(groups));
    }

    pub fn save_server_settings(&mut self) {
        let mut root = self.root_config.clone();
        root["schemaVersion"] = json!(2);
        root["server"] = json!({
            "host": self.root_host.trim(),
            "port": self.root_port.trim().parse::<u16>().unwrap_or(50345),
        });
        self.save_root(root, "服务设置已保存", RootNext::Keep);
    }

    pub fn open_build_editor(&mut self, node_id: Option<&str>, group_id: Option<&str>) {
        let defaults = self.config.clone();
        let mut editor = BuildNodeEditor::new(group_id.map(str::to_owned), &defaults);
        if let Some(node_id) = node_id {
            let Some(node) = self
                .build_nodes()
                .into_iter()
                .find(|node| node["id"].as_str() == Some(node_id))
            else {
                return;
            };
            editor.editing_id = Some(node_id.to_owned());
            editor.name = node["name"].as_str().unwrap_or("").to_owned();
            editor.profile_asset_path = node["profileAssetPath"]
                .as_str()
                .or_else(|| node["assetPath"].as_str())
                .unwrap_or("")
                .to_owned();
            editor.target = node["target"].as_i64().unwrap_or(19);
            editor.custom_build_dir = node["customBuildDir"].as_str().unwrap_or("").to_owned();
            editor.name_template = node["nameTemplate"].as_str().unwrap_or("").to_owned();
            editor.auto_zip = node["autoZip"]
                .as_bool()
                .unwrap_or_else(|| defaults["defaultAutoZip"].as_bool().unwrap_or(true));
            editor.dev = node["dev"]
                .as_bool()
                .unwrap_or_else(|| defaults["defaultDevBuild"].as_bool().unwrap_or(false));
            editor.build_addressables = node["buildAddressables"].as_bool().unwrap_or_else(|| {
                defaults["defaultBuildAddressables"]
                    .as_bool()
                    .unwrap_or(false)
            });
            editor.addressables_method =
                node["addressablesMethod"].as_str().unwrap_or("").to_owned();
            editor.android_build_kind = node["androidBuildKind"]
                .as_str()
                .filter(|kind| ["apk", "aab", "gradleProject"].contains(kind))
                .unwrap_or_else(|| {
                    defaults["defaultAndroidBuildKind"]
                        .as_str()
                        .unwrap_or("apk")
                })
                .to_owned();
            editor.keystore_name = node["keystoreName"].as_str().unwrap_or("").to_owned();
            editor.keyalias_name = node["keyaliasName"]
                .as_str()
                .filter(|value| !value.is_empty())
                .unwrap_or("key")
                .to_owned();
            editor.keystore_pass = node["keystorePass"].as_str().unwrap_or("").to_owned();
            editor.keyalias_pass = node["keyaliasPass"].as_str().unwrap_or("").to_owned();
            editor.remember = node["remember"].as_bool().unwrap_or(false);
            editor.vcs_group_ids = node["vcsGroupIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
        } else if let Some(profile) = self.profiles.first() {
            editor.profile_asset_path = profile["assetPath"].as_str().unwrap_or("").to_owned();
            editor.name = profile["name"]
                .as_str()
                .unwrap_or("新建打包节点")
                .to_owned();
            editor.target = profile["target"].as_i64().unwrap_or(19);
            editor.keystore_name = profile["keystoreName"].as_str().unwrap_or("").to_owned();
            editor.keyalias_name = profile["keyaliasName"]
                .as_str()
                .filter(|value| !value.is_empty())
                .unwrap_or("key")
                .to_owned();
            editor.remember = true;
        }
        self.build_editor = Some(editor);
        self.read_player_settings();
    }

    pub fn clone_build_node(&mut self, node_id: &str) {
        let Some(mut node) = self
            .build_nodes()
            .into_iter()
            .find(|node| node["id"].as_str() == Some(node_id))
        else {
            return;
        };
        let new_id = unique_id("bn");
        let name = node["name"].as_str().unwrap_or("节点");
        node["name"] = json!(format!("{name}（副本）"));
        node["id"] = json!(new_id);
        let group_id = self
            .group_of_node(node_id)
            .and_then(|group| group["id"].as_str().map(str::to_owned));
        let mut nodes = self.build_nodes();
        nodes.push(node);
        self.set_cfg(&["buildNodes"], Value::Array(nodes));
        if let Some(group_id) = group_id {
            let mut groups = self.build_groups();
            if let Some(group) = groups
                .iter_mut()
                .find(|group| group["id"].as_str() == Some(group_id.as_str()))
            {
                if !group["nodeIds"].is_array() {
                    group["nodeIds"] = json!([]);
                }
                let ids = group["nodeIds"].as_array_mut().unwrap();
                let position = ids
                    .iter()
                    .position(|id| id.as_str() == Some(node_id))
                    .map(|position| position + 1)
                    .unwrap_or(ids.len());
                ids.insert(position, json!(new_id));
            }
            self.set_cfg(&["buildGroups"], Value::Array(groups));
        }
        self.build_selection.insert(new_id);
        self.toast("打包节点已复制", ToastKind::Ok);
    }

    pub fn open_vcs_editor(&mut self, node_id: Option<&str>) {
        let mut editor = VcsNodeEditor::new();
        if let Some(node_id) = node_id {
            let Some(node) = self
                .vcs_nodes()
                .into_iter()
                .find(|node| node["id"].as_str() == Some(node_id))
            else {
                return;
            };
            editor.editing_id = Some(node_id.to_owned());
            editor.name = node["name"].as_str().unwrap_or("").to_owned();
            editor.path = node["path"].as_str().unwrap_or("").to_owned();
            editor.branch = node["branch"].as_str().unwrap_or("").to_owned();
            editor.revert = node["revert"].as_str().unwrap_or("").to_owned();
            editor.vcs_type = node["type"].as_str().unwrap_or("auto").to_owned();
            if let Some(state) = self.vcs_states.get(node_id) {
                editor.branches = state["branches"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
            }
        }
        self.vcs_editor = Some(editor);
    }

    pub fn probe_vcs_editor(&mut self) {
        let body = {
            let Some(editor) = &mut self.vcs_editor else {
                return;
            };
            editor.probe_text = "正在探测…".into();
            editor.probe_ok = false;
            json!({"nodes":[{
                "id": editor.editing_id.clone().unwrap_or_else(|| "editor".into()),
                "name": editor.name,
                "path": editor.path,
                "branch": editor.branch,
                "revert": editor.revert,
                "type": editor.vcs_type,
            }]})
        };
        self.request(
            PendingKind::VcsProbeEditor,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/vcs/probe",
            body,
        );
    }

    pub fn confirm_action(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::DeleteProject(id) => {
                let mut root = self.root_config.clone();
                if let Some(projects) = root["projects"].as_array_mut() {
                    projects.retain(|project| project["id"].as_str() != Some(id.as_str()));
                }
                let next = if id == self.selected_project_id {
                    RootNext::Clear
                } else {
                    RootNext::Keep
                };
                self.save_root(root, "项目配置已删除", next);
            }
            ConfirmAction::DeleteBuildNode(id) => {
                let mut nodes = self.build_nodes();
                nodes.retain(|node| node["id"].as_str() != Some(id.as_str()));
                self.set_cfg(&["buildNodes"], Value::Array(nodes));
                let mut groups = self.build_groups();
                for group in &mut groups {
                    if let Some(ids) = group["nodeIds"].as_array_mut() {
                        ids.retain(|node_id| node_id.as_str() != Some(id.as_str()));
                    }
                }
                self.set_cfg(&["buildGroups"], Value::Array(groups));
                self.build_selection.remove(&id);
                self.toast("打包节点已删除", ToastKind::Info);
            }
            ConfirmAction::DeleteBuildGroup(id) => {
                let mut groups = self.build_groups();
                groups.retain(|group| group["id"].as_str() != Some(id.as_str()));
                self.set_cfg(&["buildGroups"], Value::Array(groups));
                if self.build_group == id {
                    self.build_group = "all".into();
                    self.set_cfg(&["activeBuildGroupId"], json!("all"));
                }
                self.toast("构建组已删除，节点保留", ToastKind::Info);
            }
            ConfirmAction::DeleteVcsNode(id) => {
                let mut nodes = self.vcs_nodes();
                nodes.retain(|node| node["id"].as_str() != Some(id.as_str()));
                self.set_cfg(&["vcs", "nodes"], Value::Array(nodes));
                let mut groups = self.vcs_groups();
                for group in &mut groups {
                    if let Some(ids) = group["nodeIds"].as_array_mut() {
                        ids.retain(|node_id| node_id.as_str() != Some(id.as_str()));
                    }
                }
                self.set_cfg(&["vcs", "groups"], Value::Array(groups));
                self.vcs_states.remove(&id);
                self.toast("版本节点已删除", ToastKind::Info);
            }
            ConfirmAction::DeleteVcsGroup(id) => {
                let mut groups = self.vcs_groups();
                groups.retain(|group| group["id"].as_str() != Some(id.as_str()));
                self.set_cfg(&["vcs", "groups"], Value::Array(groups));
                self.toast("版本分组已删除，节点保留", ToastKind::Info);
            }
            ConfirmAction::ClearLog(kind) => {
                match kind {
                    LogKind::Build => self.build_log.clear(),
                    LogKind::Check => self.check_log.clear(),
                    LogKind::Vcs => self.vcs_log.clear(),
                }
                self.toast("当前日志视图已清空", ToastKind::Info);
            }
        }
    }

    pub fn save_root(&mut self, mut root: Value, message: &str, next: RootNext) {
        if root["schemaVersion"].is_null() {
            root["schemaVersion"] = json!(2);
        }
        root["server"] = json!({
            "host": self.root_host.trim(),
            "port": self.root_port.trim().parse::<u16>().unwrap_or(50345),
        });
        let mut shared_ai = None;
        if let Some(projects) = root["projects"].as_array_mut() {
            if let Some(current) = projects
                .iter_mut()
                .find(|project| project["id"].as_str() == Some(self.selected_project_id.as_str()))
            {
                let mut config = self.config.clone();
                if let Some(ai) = config.as_object_mut().and_then(|obj| obj.remove("ai")) {
                    shared_ai = Some(ai);
                }
                *current = config;
            }
        }
        if let Some(ai) = shared_ai {
            root["ai"] = ai;
        }
        if root["server"]["port"].as_u64()
            != self
                .base_url
                .rsplit(':')
                .next()
                .and_then(|p| p.parse().ok())
        {
            self.server_restart_note = true;
        }
        let body = root.clone();
        self.request(
            PendingKind::RootSave {
                body,
                message: message.to_owned(),
                next,
            },
            None,
            "POST",
            "/api/config",
            root,
        );
    }

    pub fn save_project_metadata(&mut self) {
        let id = self.selected_project_id.clone();
        if id.is_empty() {
            return;
        }
        let name = self.project_meta_name.trim();
        let slug = self.project_meta_slug.trim();
        if name.is_empty() {
            self.toast("请输入项目名称", ToastKind::Error);
            return;
        }
        if slug.is_empty()
            || slug.len() > 64
            || !slug
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            self.toast(
                "链接标识仅允许字母、数字、下划线和短横线，长度不超过 64",
                ToastKind::Error,
            );
            return;
        }
        if self.projects.iter().any(|project| {
            project["id"].as_str() != Some(id.as_str()) && project["slug"].as_str() == Some(slug)
        }) {
            self.toast("链接标识已被其他项目使用", ToastKind::Error);
            return;
        }
        let mut root = self.root_config.clone();
        if let Some(project) = root["projects"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .find(|project| project["id"].as_str() == Some(id.as_str()))
        {
            project["name"] = json!(name);
            project["slug"] = json!(slug);
            project["enabled"] = json!(self.project_meta_enabled);
        }
        self.save_root(root, "项目信息已保存", RootNext::Keep);
    }

    pub fn add_project(&mut self) {
        let id = unique_id("project");
        let mut slug = "project".to_owned();
        let mut suffix = 2;
        while self
            .projects
            .iter()
            .any(|project| project["slug"].as_str() == Some(slug.as_str()))
        {
            slug = format!("project-{suffix}");
            suffix += 1;
        }
        let mut root = self.root_config.clone();
        if !root["projects"].is_array() {
            root["projects"] = json!([]);
        }
        root["projects"].as_array_mut().unwrap().push(json!({
            "id": id,
            "slug": slug,
            "name": "新项目",
            "enabled": true,
            "projectPath": "",
            "engine": {"mode":"batchmode","unityExe":"","nographics":true},
            "buildNodes": [],
            "buildGroups": [],
            "profileConfigs": {},
            "vcs": {"nodes":[],"groups":[]},
            "buildNumber": "-1",
            "outputBase": "Builds",
            "successDir": "构建成功"
        }));
        self.save_root(root, "项目已创建", RootNext::Select(id));
    }

    pub fn request_android_scan(&mut self) {
        let body = json!({
            "projectPath": self.cfg_str(&["projectPath"]),
            "unityExe": self.cfg_str(&["engine", "unityExe"]),
        });
        self.request(
            PendingKind::AndroidScan,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/android-tools/scan",
            body,
        );
    }

    pub fn request_unity_detect(&mut self) {
        self.request(
            PendingKind::UnityDetect,
            Some(self.selected_project_id.clone()),
            "GET",
            "/api/unity-detect",
            json!({}),
        );
    }

    pub fn request_cli_probe(&mut self) {
        self.request(
            PendingKind::CliProbe,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/cli-probe",
            json!({}),
        );
    }

    pub fn request_cli_open(&mut self) {
        self.request(
            PendingKind::CliOpen,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/cli-open",
            json!({"projectPath":self.cfg_str(&["projectPath"])}),
        );
    }

    pub fn request_build_preview(&mut self) {
        self.request(
            PendingKind::BuildPreview,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/build/preview",
            self.build_body(),
        );
    }

    pub fn request_check_preview(&mut self) {
        self.request(
            PendingKind::CheckPreview,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/check/preview",
            self.check_body(),
        );
    }

    pub fn request_ai(&mut self) {
        self.ai = Some(AiDialog {
            loading: true,
            text: "正在请求 AI 分析…".into(),
        });
        self.request(
            PendingKind::AiAnalyze,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/ai/analyze",
            json!({}),
        );
    }

    pub fn start_build(&mut self) {
        if self.selected_build_nodes().is_empty() {
            self.toast("当前构建选择为空，请先选择节点", ToastKind::Error);
            self.page = Page::Profiles;
            return;
        }
        self.ensure_saved_then(DeferredAction::StartBuild(self.build_body()));
    }

    pub fn stop_build(&mut self) {
        self.request(
            PendingKind::StopBuild,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/build/stop",
            json!({}),
        );
    }

    pub fn start_check(&mut self) {
        if self.check_targets.is_empty() {
            self.toast("请至少选择一条环境线", ToastKind::Error);
            return;
        }
        let body = self.check_body();
        if body["projectPath"].as_str().unwrap_or("").trim().is_empty() {
            self.toast("请填写环境检测的 Unity 项目路径", ToastKind::Error);
            return;
        }
        self.ensure_saved_then(DeferredAction::StartCheck(body));
    }

    pub fn stop_check(&mut self) {
        self.request(
            PendingKind::StopCheck,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/check/stop",
            json!({}),
        );
    }

    pub fn start_vcs_update(&mut self, ids: Vec<String>) {
        if ids.is_empty() {
            self.toast("没有可更新的版本节点", ToastKind::Error);
            return;
        }
        self.ensure_saved_then(DeferredAction::StartVcs(json!({"ids":ids})));
    }

    pub fn stop_vcs(&mut self) {
        self.request(
            PendingKind::StopVcs,
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/vcs/stop",
            json!({}),
        );
    }

    pub fn read_player_settings(&mut self) {
        let asset_path = {
            let Some(editor) = &mut self.build_editor else {
                return;
            };
            let asset_path = editor.profile_asset_path.clone();
            if asset_path.is_empty() {
                editor.error = "请先选择关联 Profile".into();
                return;
            }
            let player = editor
                .player
                .get_or_insert_with(|| PlayerSettingsEditor::new(asset_path.clone()));
            player.asset_path = asset_path.clone();
            player.loading = true;
            player.error.clear();
            asset_path
        };
        self.request(
            PendingKind::ProfileRead {
                asset_path: asset_path.clone(),
            },
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/profile-settings/read",
            json!({"assetPath": asset_path}),
        );
    }

    pub fn write_player_settings(&mut self) {
        let (asset_path, body) = {
            let Some(editor) = &mut self.build_editor else {
                return;
            };
            let Some(player) = &mut editor.player else {
                return;
            };
            if !player.editable {
                player.error = "当前文件不可安全编辑，请在 Unity 中启用覆盖".into();
                return;
            }
            let body = json!({
                "assetPath": player.asset_path,
                "revision": player.revision,
                "productName": player.product_name,
                "companyName": player.company_name,
                "bundleVersion": player.bundle_version,
            });
            player.loading = true;
            player.error.clear();
            (player.asset_path.clone(), body)
        };
        self.request(
            PendingKind::ProfileWrite {
                asset_path: asset_path.clone(),
            },
            Some(self.selected_project_id.clone()),
            "POST",
            "/api/profile-settings/write",
            body,
        );
    }

    pub fn save_build_editor(&mut self) {
        let Some(editor) = self.build_editor.as_ref() else {
            return;
        };
        if editor.profile_asset_path.trim().is_empty() {
            if let Some(editor) = &mut self.build_editor {
                editor.error = "请选择关联 Profile".into();
            }
            return;
        }
        if editor.name.trim().is_empty() {
            if let Some(editor) = &mut self.build_editor {
                editor.error = "节点名称不能为空".into();
            }
            return;
        }
        let profile = self
            .profiles
            .iter()
            .find(|profile| {
                profile["assetPath"].as_str() == Some(editor.profile_asset_path.as_str())
            })
            .cloned()
            .unwrap_or_else(|| json!({}));
        let existing = editor.editing_id.as_ref().and_then(|id| {
            self.build_nodes()
                .into_iter()
                .find(|node| node["id"].as_str() == Some(id.as_str()))
        });
        let target = profile["target"]
            .as_i64()
            .or_else(|| existing.as_ref().and_then(|node| node["target"].as_i64()))
            .unwrap_or(editor.target);
        let node_id = editor.editing_id.clone().unwrap_or_else(|| unique_id("bn"));
        let player = editor.player.as_ref();
        let node = json!({
            "id": node_id,
            "name": editor.name.trim(),
            "profileAssetPath": editor.profile_asset_path,
            "target": target,
            "targetName": profile["targetName"].as_str().or_else(|| existing.as_ref().and_then(|node| node["targetName"].as_str())).unwrap_or(if target == 13 {"Android"} else {"Windows (x64)"}),
            "productName": player.map(|p| p.product_name.clone()).or_else(|| profile["productName"].as_str().map(str::to_owned)),
            "companyName": player.map(|p| p.company_name.clone()).or_else(|| profile["companyName"].as_str().map(str::to_owned)),
            "bundleVersion": player.map(|p| p.bundle_version.clone()).or_else(|| profile["bundleVersion"].as_str().map(str::to_owned)),
            "sceneCount": profile["sceneCount"].as_i64().unwrap_or(0),
            "absPath": profile["absPath"].as_str().unwrap_or(""),
            "subtarget": profile["subtarget"].as_i64().unwrap_or(0),
            "customBuildDir": editor.custom_build_dir.trim(),
            "nameTemplate": editor.name_template.trim(),
            "autoZip": editor.auto_zip,
            "dev": editor.dev,
            "buildAddressables": editor.build_addressables,
            "addressablesMethod": editor.addressables_method.trim(),
            "androidBuildKind": editor.android_build_kind,
            "keystoreName": editor.keystore_name.trim(),
            "keyaliasName": editor.keyalias_name.trim(),
            "keystorePass": editor.keystore_pass,
            "keyaliasPass": editor.keyalias_pass,
            "remember": editor.remember,
            "vcsGroupIds": editor.vcs_group_ids,
        });
        let editing_id = editor.editing_id.clone();
        let group_id = editor.group_id.clone();
        let mut nodes = self.build_nodes();
        if let Some(id) = &editing_id {
            if let Some(existing) = nodes
                .iter_mut()
                .find(|node| node["id"].as_str() == Some(id.as_str()))
            {
                *existing = node.clone();
            }
        } else {
            nodes.push(node.clone());
        }
        self.set_cfg(&["buildNodes"], Value::Array(nodes));
        if editing_id.is_none() {
            if let Some(group_id) = group_id {
                let mut groups = self.build_groups();
                if let Some(group) = groups
                    .iter_mut()
                    .find(|group| group["id"].as_str() == Some(group_id.as_str()))
                {
                    if !group["nodeIds"].is_array() {
                        group["nodeIds"] = json!([]);
                    }
                    group["nodeIds"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(node_id));
                }
                self.set_cfg(&["buildGroups"], Value::Array(groups));
            }
            self.build_selection.insert(node_id.clone());
        }
        self.build_editor = None;
        self.toast(
            if editing_id.is_some() {
                "打包节点已更新"
            } else {
                "打包节点已添加"
            },
            ToastKind::Ok,
        );
    }

    pub fn save_vcs_editor(&mut self) {
        let (id, editing_id, node) = {
            let Some(editor) = self.vcs_editor.as_ref() else {
                return;
            };
            if editor.name.trim().is_empty() {
                if let Some(editor) = &mut self.vcs_editor {
                    editor.error = "节点名称不能为空".into();
                }
                return;
            }
            let id = editor.editing_id.clone().unwrap_or_else(|| unique_id("vn"));
            let node = json!({
                "id": id,
                "name": editor.name.trim(),
                "path": editor.path.trim(),
                "branch": editor.branch.trim(),
                "revert": editor.revert.trim(),
                "type": editor.vcs_type,
            });
            (id, editor.editing_id.clone(), node)
        };
        let mut nodes = self.vcs_nodes();
        if let Some(editing_id) = &editing_id {
            if let Some(existing) = nodes
                .iter_mut()
                .find(|node| node["id"].as_str() == Some(editing_id.as_str()))
            {
                *existing = node;
            }
        } else {
            nodes.push(node);
        }
        self.set_cfg(&["vcs", "nodes"], Value::Array(nodes));
        self.vcs_editor = None;
        self.toast(
            if editing_id.is_some() {
                "版本节点已更新"
            } else {
                "版本节点已添加"
            },
            ToastKind::Ok,
        );
        self.probe_vcs_ids(vec![id]);
    }

    fn poll_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                UiEvent::Response { id, result } => self.handle_response(id, result),
                UiEvent::Backend(event) => self.handle_backend_event(event),
                UiEvent::Monitor(result) => self.handle_monitor(result),
                UiEvent::Tray(action) => self.handle_tray(action, ctx),
            }
        }
    }
}

impl eframe::App for AdminApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events(ctx);
        if ctx.input(|input| input.viewport().close_requested()) {
            if self.quitting {
                return;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.visible = false;
        }
        self.monitor_active.store(
            self.page == Page::Monitor && self.visible,
            Ordering::Release,
        );
        self.tick_save();
        if self.saving
            || self.pending.values().any(|entry| {
                matches!(
                    entry.kind,
                    PendingKind::ProjectLoad
                        | PendingKind::BuildStart
                        | PendingKind::CheckStart
                        | PendingKind::VcsStart
                )
            })
        {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if !self.toasts.is_empty() {
            let animating = self.toasts.iter().any(|toast| {
                let age = toast.created.elapsed().as_secs_f32();
                age < 0.24 || age > 4.68
            });
            ctx.request_repaint_after(Duration::from_millis(if animating { 16 } else { 250 }));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        crate::ui::draw_shell(self, ui);
        crate::ui::draw_overlays(self, &ctx);
    }
}

impl Drop for AdminApp {
    fn drop(&mut self) {
        self.monitor_running.store(false, Ordering::Release);
        if let Some(service) = self.service.take() {
            service.stop();
        }
    }
}

pub fn nested_mut<'a>(value: &'a mut Value, keys: &[&str]) -> &'a mut Value {
    let mut current = value;
    for key in keys {
        if !current.is_object() {
            *current = json!({});
        }
        current = current
            .as_object_mut()
            .unwrap()
            .entry((*key).to_owned())
            .or_insert(Value::Null);
    }
    current
}

fn reorder_node(
    nodes: &mut Vec<Value>,
    groups: &mut [Value],
    node_id: &str,
    group_id: Option<&str>,
    before_node_id: Option<&str>,
) -> bool {
    for group in groups.iter_mut() {
        if let Some(ids) = group["nodeIds"].as_array_mut() {
            ids.retain(|id| id.as_str() != Some(node_id));
        }
    }
    if let Some(group_id) = group_id {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| group["id"].as_str() == Some(group_id))
        {
            if !group["nodeIds"].is_array() {
                group["nodeIds"] = json!([]);
            }
            let ids = group["nodeIds"].as_array_mut().unwrap();
            let position = before_node_id
                .and_then(|before| ids.iter().position(|id| id.as_str() == Some(before)));
            if let Some(position) = position {
                ids.insert(position, json!(node_id));
            } else {
                ids.push(json!(node_id));
            }
        }
        return false;
    }
    let Some(before) = before_node_id else {
        return false;
    };
    if before == node_id {
        return false;
    }
    let Some(from) = nodes
        .iter()
        .position(|node| node["id"].as_str() == Some(node_id))
    else {
        return false;
    };
    let Some(target) = nodes
        .iter()
        .position(|candidate| candidate["id"].as_str() == Some(before))
    else {
        return false;
    };
    let node = nodes.remove(from);
    let target = if from < target { target - 1 } else { target };
    nodes.insert(target, node);
    true
}

pub fn unique_id(prefix: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{prefix}_{millis:x}_{:x}", std::process::id())
}

pub fn env_targets() -> &'static [(i64, &'static str)] {
    ENV_TARGETS
}

fn sync_profile_metadata(config: &mut Value, profiles: &[Value]) -> bool {
    const PROFILE_KEYS: [&str; 7] = [
        "target",
        "targetName",
        "productName",
        "companyName",
        "bundleVersion",
        "sceneCount",
        "absPath",
    ];
    let Some(nodes) = config.get_mut("buildNodes").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    for node in nodes {
        let Some(profile) = profiles
            .iter()
            .find(|profile| profile["assetPath"].as_str() == node["profileAssetPath"].as_str())
        else {
            continue;
        };
        for key in PROFILE_KEYS {
            let next = profile.get(key).cloned().unwrap_or(Value::Null);
            if node.get(key) != Some(&next) {
                node[key] = next;
                changed = true;
            }
        }
    }
    changed
}

fn install_theme_and_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        ("Microsoft YaHei UI", r"C:\Windows\Fonts\msyh.ttc"),
        ("Microsoft YaHei", r"C:\Windows\Fonts\msyh.ttc"),
        (
            "Noto Sans CJK SC",
            r"C:\Windows\Fonts\NotoSansCJKsc-Regular.otf",
        ),
        ("SimHei", r"C:\Windows\Fonts\simhei.ttf"),
    ];
    let mut loaded = false;
    for (name, path) in candidates {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let key = format!("cjk-{}", name.replace(' ', "-"));
        fonts
            .font_data
            .insert(key.clone(), Arc::new(egui::FontData::from_owned(bytes)));
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.insert(0, key.clone());
        }
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            family.push(key);
        }
        loaded = true;
        break;
    }
    if !loaded {
        if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\simhei.ttf") {
            fonts.font_data.insert(
                "cjk-simhei".into(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "cjk-simhei".into());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.push("cjk-simhei".into());
            }
        }
    }
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG_PANEL;
    visuals.window_fill = CARD;
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = CARD_ALT;
    visuals.code_bg_color = BG;
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.widgets.noninteractive.bg_fill = CARD_ALT;
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.bg_fill = CARD;
    visuals.widgets.inactive.weak_bg_fill = CARD;
    visuals.widgets.inactive.bg_stroke =
        egui::Stroke::new(1.0, egui::Color32::from_rgb(30, 41, 59));
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(30, 41, 59);
    visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(30, 41, 59);
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT.gamma_multiply(0.7));
    visuals.widgets.active.bg_fill = ACCENT.gamma_multiply(0.25);
    visuals.widgets.active.weak_bg_fill = ACCENT.gamma_multiply(0.25);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.override_text_color = Some(TEXT);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: egui::Color32::from_black_alpha(150),
    };
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.interact_size.y = 30.0;
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(22.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::new(13.0, egui::FontFamily::Monospace),
    );
    ctx.set_style_of(egui::Theme::Dark, style);
}

pub fn join_path_value(base_url: &str, slug: &str) -> String {
    if slug.is_empty() {
        String::new()
    } else {
        format!("{base_url}/p/{slug}")
    }
}

pub fn format_bytes(value: u64) -> String {
    match value {
        0..=1023 => format!("{value} B"),
        1024..=1_048_575 => format!("{:.1} KB", value as f64 / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", value as f64 / 1_048_576.0),
        _ => format!("{:.2} GB", value as f64 / 1_073_741_824.0),
    }
}

pub fn format_timestamp(ms: u64) -> String {
    if ms == 0 {
        return "—".into();
    }
    let seconds = (ms / 1000) as i64;
    let datetime = chrono::DateTime::from_timestamp(seconds, 0)
        .unwrap_or_default()
        .with_timezone(&chrono::Local);
    datetime.format("%Y-%m-%d %H:%M").to_string()
}

pub fn safe_join(base: &str, path: &str) -> String {
    if path.trim().is_empty() {
        base.to_owned()
    } else if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

pub fn file_name(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

pub fn async_repaint(ctx: &egui::Context) {
    ctx.request_repaint();
}

pub fn mutex<T>(value: T) -> Mutex<T> {
    Mutex::new(value)
}

pub fn _kept_types(_: Instant, _: Duration) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(value: &Value) -> Vec<&str> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    }

    #[test]
    fn log_panel_limits_renders_searches_and_clears() {
        let mut panel = LogPanel::default();
        for index in 0..LOG_LIMIT + 3 {
            panel.push(&format!("line {index}\n"));
        }
        assert_eq!(panel.lines.len(), LOG_LIMIT);
        assert_eq!(panel.lines.front().map(String::as_str), Some("line 3"));
        assert_eq!(
            panel.lines.back().map(String::as_str),
            Some(format!("line {}", LOG_LIMIT + 2).as_str())
        );

        panel.push("partial NO-LINE");
        panel.search = "LINE 4997".into();
        assert_eq!(panel.rendered(), "line 4997");
        panel.search = "no-line".into();
        assert_eq!(panel.rendered(), "partial NO-LINE");
        panel.search = "not-present".into();
        assert!(panel.rendered().is_empty());

        panel.clear();
        assert!(panel.lines.is_empty());
        assert!(panel.partial.is_empty());
        assert!(panel.rendered().is_empty());
    }

    #[test]
    fn node_reordering_moves_between_groups_and_keeps_explicit_order() {
        let mut nodes = vec![json!({"id":"a"}), json!({"id":"b"}), json!({"id":"c"})];
        let mut groups = vec![
            json!({"id":"g","nodeIds":["a","b"]}),
            json!({"id":"h","nodeIds":[]}),
        ];

        assert!(!reorder_node(
            &mut nodes,
            &mut groups,
            "b",
            Some("g"),
            Some("a")
        ));
        assert_eq!(ids(&groups[0]["nodeIds"]), vec!["b", "a"]);

        reorder_node(&mut nodes, &mut groups, "b", Some("h"), None);
        assert_eq!(ids(&groups[0]["nodeIds"]), vec!["a"]);
        assert_eq!(ids(&groups[1]["nodeIds"]), vec!["b"]);

        reorder_node(&mut nodes, &mut groups, "b", None, Some("a"));
        assert_eq!(ids(&groups[1]["nodeIds"]), Vec::<&str>::new());
        assert_eq!(
            nodes
                .iter()
                .filter_map(|node| node["id"].as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );

        assert!(!reorder_node(&mut nodes, &mut groups, "b", None, Some("b")));
        assert_eq!(
            nodes
                .iter()
                .filter_map(|node| node["id"].as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
    }

    #[test]
    fn profile_scan_refreshes_metadata_without_leaving_a_save_pending() {
        let profiles = vec![json!({
            "assetPath":"Assets/Settings/Build Profiles/Windows.asset",
            "target":19,
            "targetName":"Windows (x64)",
            "productName":"Game",
            "companyName":"Studio",
            "bundleVersion":"2.0.1",
            "sceneCount":12,
            "absPath":"Project\\Assets\\Settings\\Build Profiles\\Windows.asset"
        })];
        let mut config = json!({
            "projectPath":"Project",
            "buildNodes":[{
                "id":"node",
                "name":"Windows",
                "profileAssetPath":"Assets/Settings/Build Profiles/Windows.asset",
                "bundleVersion":"1.9.0",
                "customBuildDir":"SharedCache",
                "unknownField":{"keep":true}
            }]
        });
        let mut saved = config.clone();

        assert!(sync_profile_metadata(&mut config, &profiles));
        assert_ne!(config, saved);
        assert!(sync_profile_metadata(&mut saved, &profiles));
        assert_eq!(config, saved);

        let node = &config["buildNodes"][0];
        assert_eq!(node["bundleVersion"], "2.0.1");
        assert_eq!(node["sceneCount"], 12);
        assert_eq!(node["customBuildDir"], "SharedCache");
        assert_eq!(node["unknownField"]["keep"], true);
    }
}
