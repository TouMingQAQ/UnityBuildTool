#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod http;

use http::Service;
use std::path::PathBuf;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

fn random_token() -> Result<String, Box<dyn std::error::Error>> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("无法生成管理令牌: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn start_service(
    app: &tauri::App,
    token: &str,
) -> Result<(Service, String), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let config = if cfg!(debug_assertions) {
        root.join("web/config.json")
    } else {
        let data = app.path().app_data_dir()?;
        std::fs::create_dir_all(&data)?;
        data.join("config.json")
    };
    // Installed builds resolve relative project paths from the config directory.
    let runtime_root = if cfg!(debug_assertions) {
        root.join("web")
    } else {
        config.parent().unwrap().to_path_buf()
    };
    Service::start(config, runtime_root, token.to_owned())
}

fn toggle_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("admin") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
}

fn tray_image() -> Image<'static> {
    let mut pixels = vec![0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let i = (y * 32 + x) * 4;
            let bar = (6..26).contains(&x) && (7..12).contains(&y)
                || (13..19).contains(&x) && (7..26).contains(&y);
            pixels[i..i + 4].copy_from_slice(if bar {
                &[248, 250, 252, 255]
            } else {
                &[14, 116, 185, 255]
            });
        }
    }
    Image::new_owned(pixels, 32, 32)
}

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let token = random_token()?;
            let (service, base_url) = start_service(app, &token)?;
            println!("TBUILDTOOL_READY {base_url}");
            app.manage(service);
            let admin_url = format!("{base_url}/admin/?token={token}");
            let init = format!(
                "if (location.origin === {} && location.pathname.startsWith('/admin/')) {{ window.__TBUILDTOOL_ADMIN_TOKEN = {}; }}",
                serde_json::to_string(&base_url)?, serde_json::to_string(&token)?
            );
            let allowed_origin = base_url.clone();
            WebviewWindowBuilder::new(app, "admin", WebviewUrl::External(admin_url.parse()?))
                .title("管理端")
                .inner_size(800.0, 600.0)
                .min_inner_size(640.0, 480.0)
                .initialization_script(&init)
                .on_navigation(move |url| url.origin().ascii_serialization() == allowed_origin && url.path().starts_with("/admin/"))
                .build()?;
            let toggle = MenuItem::with_id(app, "toggle", "显示 / 隐藏管理端", true, None::<&str>)?;
            let website = MenuItem::with_id(app, "website", "打开用户网页", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &website, &quit])?;
            TrayIconBuilder::new()
                .icon(tray_image())
                .tooltip("TBuildTool 管理端")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "toggle" => toggle_window(app),
                    "website" => { let _ = open::that(format!("{base_url}/")); }
                    "quit" => { app.state::<Service>().stop(); app.exit(0); }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(event, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) {
                        toggle_window(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|app, event| {
            if matches!(event, RunEvent::Exit | RunEvent::ExitRequested { .. }) {
                if let Some(service) = app.try_state::<Service>() {
                    service.stop();
                }
            }
        }),
        Err(error) => {
            eprintln!("管理端启动失败: {error}");
            std::process::exit(1);
        }
    }
}
