#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod admin_api;
mod app;
mod backend;
mod http;
mod ui;

use app::AdminApp;
use eframe::egui;
use http::Service;
use std::path::PathBuf;

fn config_paths() -> Result<(PathBuf, PathBuf), String> {
    if cfg!(debug_assertions) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        Ok((root.join("web/config.json"), root.join("web")))
    } else {
        let appdata = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "APPDATA 不可用，无法定位配置目录".to_string())?;
        let data = appdata.join("com.unitybuildtool.admin");
        std::fs::create_dir_all(&data)
            .map_err(|e| format!("无法创建配置目录 {}: {e}", data.display()))?;
        Ok((data.join("config.json"), data))
    }
}

fn main() {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let (config, runtime_root) =
            config_paths().map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        let (service, base_url) = Service::start(config, runtime_root)?;
        println!("TBUILDTOOL_READY {base_url}");
        let icon = eframe::icon_data::from_png_bytes(include_bytes!("../icons/icon.png")).ok();
        let mut viewport = egui::ViewportBuilder::default()
            .with_title("UnityBuildTool 管理端")
            .with_app_id("com.unitybuildtool.admin")
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([640.0, 480.0]);
        if let Some(icon) = icon {
            viewport = viewport.with_icon(icon);
        }
        let options = eframe::NativeOptions {
            viewport,
            centered: true,
            ..Default::default()
        };
        eframe::run_native(
            "UnityBuildTool 管理端",
            options,
            Box::new(move |cc| Ok(Box::new(AdminApp::new(cc, service, base_url)))),
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("管理端启动失败: {error}");
        show_error(&format!("管理端启动失败：\n{error}"));
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn show_error(message: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let title: Vec<u16> = std::ffi::OsStr::new("UnityBuildTool 管理端")
        .encode_wide()
        .chain(Some(0))
        .collect();
    let text: Vec<u16> = std::ffi::OsStr::new(message)
        .encode_wide()
        .chain(Some(0))
        .collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_error(message: &str) {
    eprintln!("{message}");
}
