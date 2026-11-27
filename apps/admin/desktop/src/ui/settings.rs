use super::{
    card, cfg_checkbox, cfg_combo, cfg_text, cfg_textarea, danger_button, hint, pill,
    primary_button, secondary_button, section_title, show_password, ACCENT, ACCENT_ALT, CARD_ALT,
    ERR, MUTED, OK, WARN,
};
use crate::app::{AdminApp, ConfirmAction, PathScope};
use eframe::egui;
use egui::RichText;

const DEFAULT_ARTIFACT_TEMPLATE: &str =
    "{Product}_{Platform}_v{Version}_b{VersionCode}_{Time}{Dev}";

pub fn draw(ui: &mut egui::Ui, app: &mut AdminApp) {
    let busy = app.busy();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    section_title(
                        ui,
                        "设置",
                        "项目管理、全局服务、Unity 引擎、Android 工具与 AI 诊断",
                    );
                    hint(
                        ui,
                        "项目设置按项目独立保存；AI、外部 HTTP 服务与监听端口为全局配置。",
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if busy {
                        pill(ui, "任务运行中，写入已锁定", WARN);
                    } else if app.save_error.is_some() {
                        pill(ui, "配置保存失败", ERR);
                    } else if app.config_loaded && app.config != app.saved_config {
                        pill(ui, "正在自动保存", ACCENT);
                    } else {
                        pill(ui, "配置已同步", OK);
                    }
                });
            });
            ui.add_space(10.0);

            ui.add_enabled_ui(!busy, |ui| {
                project_management(ui, app);
                ui.add_space(10.0);
                server_settings(ui, app);
                ui.add_space(10.0);
                engine_settings(ui, app);
                ui.add_space(10.0);
                naming_and_archive(ui, app);
                ui.add_space(10.0);
                android_tools(ui, app);
                ui.add_space(10.0);
                network_and_ai(ui, app);
                ui.add_space(10.0);
                application_paths(ui, app);
                ui.add_space(10.0);
                project_paths(ui, app);
            });

            if busy {
                ui.add_space(8.0);
                hint(
                    ui,
                    "构建、环境检测或版本更新运行/排队期间，服务端拒绝修改项目配置。请停止任务后再编辑。",
                );
            }
        });
}

fn project_management(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("项目").strong().size(16.0));
            pill(ui, &format!("{} 个项目", app.projects.len()), ACCENT);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(primary_button("新建项目")).clicked() {
                    app.add_project();
                }
            });
        });
        hint(
            ui,
            "切换项目会重新扫描该项目的 Profile、节点、仓库和运行状态。",
        );
        ui.add_space(6.0);

        let projects = app.projects.clone();
        let mut switch_to = None;
        let mut delete_project = None;
        egui::Frame::new()
            .fill(CARD_ALT)
            .corner_radius(7.0)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                for project in &projects {
                    let id = project["id"].as_str().unwrap_or("");
                    let name = project["name"].as_str().unwrap_or("未命名项目");
                    let slug = project["slug"].as_str().unwrap_or("");
                    let enabled = project["enabled"].as_bool().unwrap_or(true);
                    let selected = app.selected_project_id == id;
                    ui.horizontal(|ui| {
                        let label = format!("{name}  ·  /p/{slug}");
                        if ui
                            .selectable_label(
                                selected,
                                RichText::new(label).strong().color(if selected {
                                    ACCENT
                                } else {
                                    MUTED
                                }),
                            )
                            .clicked()
                            && !selected
                        {
                            switch_to = Some(id.to_owned());
                        }
                        pill(
                            ui,
                            if enabled { "已启用" } else { "已停用" },
                            if enabled { OK } else { WARN },
                        );
                        if selected {
                            pill(ui, "当前项目", ACCENT_ALT);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(danger_button("删除")).clicked() {
                                delete_project = Some((id.to_owned(), name.to_owned()));
                            }
                        });
                    });
                }
                if projects.is_empty() {
                    hint(ui, "尚未创建项目。");
                }
            });
        if let Some(id) = switch_to {
            app.select_project(&id);
        }
        if let Some((id, name)) = delete_project {
            app.confirm = Some(crate::app::ConfirmDialog {
                title: "删除项目".into(),
                text: format!(
                    "删除项目“{name}”的配置？Unity 工程和已生成文件不会被删除。此操作不可撤销。"
                ),
                action: ConfirmAction::DeleteProject(id),
            });
        }

        ui.add_space(10.0);
        ui.label(RichText::new("当前项目信息").strong());
        egui::Grid::new("project-metadata-grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(RichText::new("项目名称").small().color(MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut app.project_meta_name)
                        .desired_width(320.0)
                        .char_limit(100),
                );
                ui.end_row();

                ui.label(RichText::new("链接标识").small().color(MUTED));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut app.project_meta_slug)
                            .desired_width(320.0)
                            .char_limit(64),
                    );
                    hint(ui, "仅字母、数字、_、-；修改后旧 URL 失效");
                });
                ui.end_row();

                ui.label(RichText::new("用户页面").small().color(MUTED));
                ui.horizontal(|ui| {
                    ui.checkbox(&mut app.project_meta_enabled, "启用用户页面与构建入口");
                    let slug = app.project_meta_slug.trim();
                    if !slug.is_empty() {
                        hint(ui, format!("{}/p/{slug}", app.base_url));
                    }
                });
                ui.end_row();
            });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if ui.add(primary_button("保存项目信息")).clicked() {
                app.save_project_metadata();
            }
            if ui.add(secondary_button("打开用户网页")).clicked() {
                app.open_user_website();
            }
            hint(ui, "项目名称、slug 与启停状态保存在根配置中。");
        });
    });
}

fn server_settings(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("全局 Web 服务").strong().size(16.0));
            hint(ui, "用户 Web、项目页面与构建 API 共用此监听地址");
        });
        ui.add_space(5.0);
        egui::Grid::new("server-settings-grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(RichText::new("监听地址").small().color(MUTED));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut app.root_host)
                            .desired_width(220.0)
                            .hint_text("127.0.0.1"),
                    );
                    hint(ui, "默认仅本机监听；如无特殊需要请保持 127.0.0.1");
                });
                ui.end_row();

                ui.label(RichText::new("固定端口").small().color(MUTED));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut app.root_port)
                            .desired_width(110.0)
                            .char_limit(5)
                            .hint_text("50345"),
                    );
                    hint(ui, "0 表示随机端口；修改后重启管理端生效");
                });
                ui.end_row();
            });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if ui.add(primary_button("保存服务设置")).clicked() {
                app.save_server_settings();
            }
            hint(
                ui,
                format!(
                    "当前运行地址：{} · 配置启动后由内嵌 tiny_http 提供用户 Web",
                    app.base_url
                ),
            );
        });
        if app.server_restart_note {
            ui.add_space(5.0);
            ui.label(
                RichText::new("监听地址或端口已更改，重启管理端后生效。")
                    .small()
                    .color(WARN),
            );
        }
    });
}

fn engine_settings(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Unity 引擎").strong().size(16.0));
            hint(ui, "批处理模式用于常规打包，也可完全接管命令行模板");
        });
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("调用模式").small().color(MUTED));
            let mut mode = app.cfg_str(&["engine", "mode"]);
            if mode.is_empty() {
                mode = "batchmode".into();
            }
            if ui
                .selectable_label(mode == "batchmode", "Unity 批处理模式")
                .clicked()
            {
                app.set_cfg(&["engine", "mode"], serde_json::json!("batchmode"));
            }
            if ui
                .selectable_label(mode == "template", "自定义命令模板")
                .clicked()
            {
                app.set_cfg(&["engine", "mode"], serde_json::json!("template"));
            }
            pill(
                ui,
                if mode == "template" {
                    "模板模式"
                } else {
                    "batchmode"
                },
                if mode == "template" {
                    ACCENT_ALT
                } else {
                    ACCENT
                },
            );
        });

        if app.cfg_str(&["engine", "mode"]) == "template" {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("CLI 可执行文件").small().color(MUTED));
                let mut cli_path = app.cfg_str(&["engine", "cliPath"]);
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut cli_path)
                            .desired_width(500.0)
                            .hint_text("例如 G:\\Tools\\build.cmd"),
                    )
                    .changed()
                {
                    app.set_cfg(&["engine", "cliPath"], serde_json::json!(cli_path));
                }
            });
            ui.add_space(4.0);
            ui.label(RichText::new("命令模板").small().color(MUTED));
            cfg_textarea(
                ui,
                app,
                &["engine", "template"],
                "例如 {cli} --project {project} --profile {profilePath}",
                5,
            );
            hint(
                ui,
                "可用占位符：{cli} {project} {profilePath} {profileName} {target} {output} {product} {version} {versionCode} {log}",
            );
        } else {
            ui.add_space(6.0);
            egui::Grid::new("engine-batch-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Unity.exe").small().color(MUTED));
                    ui.horizontal(|ui| {
                        cfg_text(
                            ui,
                            app,
                            &["engine", "unityExe"],
                            "例如 G:\\UnityEditor\\6000.0.52f1\\Editor\\Unity.exe",
                            500.0,
                        );
                        if ui.add(secondary_button("自动探测")).clicked() {
                            app.request_unity_detect();
                        }
                        if ui.button("CLI 探测").clicked() {
                            app.request_cli_probe();
                        }
                        if ui.button("CLI 打开工程").clicked() {
                            app.request_cli_open();
                        }
                    });
                    ui.end_row();
                });
            if !app.unity_options.is_empty() {
                let options = app.unity_options.clone();
                let current = app.cfg_str(&["engine", "unityExe"]);
                let selected_label = options
                    .iter()
                    .find(|entry| entry["path"].as_str() == Some(current.as_str()))
                    .and_then(|entry| entry["version"].as_str())
                    .unwrap_or("选择已探测版本");
                let mut chosen = None;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("已探测版本").small().color(MUTED));
                    egui::ComboBox::from_id_salt("settings-unity-version")
                        .selected_text(selected_label)
                        .width(350.0)
                        .show_ui(ui, |ui| {
                            for entry in &options {
                                let label = format!(
                                    "{}  ·  {}",
                                    entry["version"].as_str().unwrap_or("未知版本"),
                                    entry["path"].as_str().unwrap_or("")
                                );
                                let path = entry["path"].as_str().unwrap_or("");
                                if ui.selectable_label(current == path, label).clicked() {
                                    chosen = Some(path.to_owned());
                                }
                            }
                        });
                    if ui.small_button("CLI 打开工程").clicked() {
                        app.request_cli_open();
                    }
                });
                if let Some(path) = chosen {
                    app.set_cfg(&["engine", "unityExe"], serde_json::json!(path));
                }
            }
            ui.add_space(4.0);
            cfg_checkbox(
                ui,
                app,
                &["engine", "nographics"],
                "无图形模式（-nographics）",
                true,
            );
            hint(
                ui,
                "批量构建建议启用 -nographics；首次导入或需要图形设备时可关闭。",
            );
        }
        if !app.unity_version.is_empty() {
            ui.add_space(4.0);
            hint(
                ui,
                format!("当前扫描工程 Unity 版本：{}", app.unity_version),
            );
        }
    });
}

fn naming_and_archive(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("命名、归档与构建默认值").strong().size(16.0));
            hint(ui, "节点未单独覆盖时使用这里的默认值");
        });
        ui.add_space(5.0);
        egui::Grid::new("naming-defaults-grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(RichText::new("成品命名模板").small().color(MUTED));
                ui.vertical(|ui| {
                    cfg_text(
                        ui,
                        app,
                        &["artifactNameTemplate"],
                        DEFAULT_ARTIFACT_TEMPLATE,
                        620.0,
                    );
                    hint(
                        ui,
                        "占位符：{Product} {Company} {Platform} {Profile} {Version} {VersionCode} {Time} {Dev}",
                    );
                });
                ui.end_row();

                ui.label(RichText::new("构建输出根目录").small().color(MUTED));
                ui.horizontal(|ui| {
                    cfg_text(ui, app, &["outputBase"], "留空使用 Builds", 360.0);
                    hint(ui, "相对 Unity 工程根目录");
                });
                ui.end_row();

                ui.label(RichText::new("成功归档目录").small().color(MUTED));
                cfg_text(ui, app, &["successDir"], "构建成功", 360.0);
                ui.end_row();

                ui.label(RichText::new("附加 Profile 目录").small().color(MUTED));
                ui.horizontal(|ui| {
                    cfg_text(
                        ui,
                        app,
                        &["profileDir"],
                        "留空使用 Assets/Settings/Build Profiles",
                        500.0,
                    );
                    hint(ui, "扫描时会与默认目录一同递归查找 .asset");
                });
                ui.end_row();

                ui.label(RichText::new("构建号").small().color(MUTED));
                ui.horizontal(|ui| {
                    cfg_text(ui, app, &["buildNumber"], "-1", 150.0);
                    hint(ui, "-1 为自动递增 AndroidBundleVersionCode；正数固定构建号");
                });
                ui.end_row();

                ui.label(RichText::new("Android 默认目标").small().color(MUTED));
                cfg_combo(
                    ui,
                    app,
                    &["defaultAndroidBuildKind"],
                    "settings-android-kind",
                    &[
                        ("apk", "APK 安装包"),
                        ("aab", "AAB（Google Play）"),
                        ("gradleProject", "导出 Android Studio 工程"),
                    ],
                    "apk",
                    280.0,
                );
                ui.end_row();
            });

        ui.add_space(5.0);
        ui.horizontal_wrapped(|ui| {
            cfg_checkbox(ui, app, &["defaultAutoZip"], "默认自动压缩产物", true);
            cfg_checkbox(ui, app, &["defaultDevBuild"], "默认 DEV 调试包", false);
            cfg_checkbox(
                ui,
                app,
                &["defaultBuildAddressables"],
                "默认构建 Addressables",
                false,
            );
            cfg_checkbox(ui, app, &["stopOnError"], "失败后停止后续节点", true);
        });
        hint(
            ui,
            "节点级 autoZip / dev / buildAddressables / androidBuildKind 可独立覆盖这些默认值。",
        );
    });
}

fn android_tools(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Android 工具目录").strong().size(16.0));
            hint(ui, "SDK / NDK / JDK 会作为环境变量注入 Unity 构建进程");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(primary_button("自动扫描")).clicked() {
                    app.request_android_scan();
                }
            });
        });
        ui.add_space(5.0);
        let project_id = app.selected_project_id.clone();
        for (label, key, placeholder) in [
            (
                "Android SDK",
                "sdk",
                "例如 C:\\Android\\Sdk，或 Unity PlaybackEngines/AndroidPlayer/SDK",
            ),
            (
                "Android NDK",
                "ndk",
                "例如 C:\\Android\\Sdk\\ndk\\27.2.12479018",
            ),
            (
                "Android JDK",
                "jdk",
                "例如 C:\\Program Files\\Unity\\Hub\\Editor\\...\\OpenJDK",
            ),
        ] {
            let current = app.cfg_str(&["androidTools", key]);
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).small().color(MUTED));
                cfg_text(ui, app, &["androidTools", key], placeholder, 500.0);
                if ui
                    .add_enabled(!current.trim().is_empty(), egui::Button::new("打开目录"))
                    .clicked()
                {
                    app.open_folder(Some(project_id.clone()), &current);
                }
            });
        }
        hint(
            ui,
            "自动扫描顺序：系统环境变量 → Unity Editor 偏好 → Unity.exe 安装目录下的 PlaybackEngines。",
        );
    });
}

fn network_and_ai(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("网络代理").strong().size(16.0));
            cfg_checkbox(ui, app, &["proxy", "enabled"], "启用代理", false);
            hint(ui, "用于 Android Gradle / AGP 依赖下载");
        });
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("代理地址").small().color(MUTED));
            cfg_text(ui, app, &["proxy", "host"], "127.0.0.1", 220.0);
            ui.label(RichText::new("端口").small().color(MUTED));
            cfg_text(ui, app, &["proxy", "port"], "10808", 100.0);
        });
    });

    ui.add_space(10.0);
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("AI 失败分析").strong().size(16.0));
            cfg_checkbox(ui, app, &["ai", "enabled"], "启用", false);
            hint(
                ui,
                "仅发送最近构建日志与必要上下文；API Key 保存在配置文件中",
            );
        });
        ui.add_space(5.0);
        egui::Grid::new("ai-settings-grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(RichText::new("接口地址").small().color(MUTED));
                cfg_text(
                    ui,
                    app,
                    &["ai", "baseUrl"],
                    "https://api.deepseek.com",
                    400.0,
                );
                ui.end_row();

                ui.label(RichText::new("模型").small().color(MUTED));
                cfg_text(ui, app, &["ai", "model"], "deepseek-chat", 260.0);
                ui.end_row();

                ui.label(RichText::new("API Key").small().color(MUTED));
                ui.horizontal(|ui| {
                    let mut value = app.cfg_str(&["ai", "apiKey"]);
                    let visible = show_password(ui, "settings-ai-api-key");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut value)
                            .desired_width(400.0)
                            .hint_text("sk-...")
                            .password(!visible),
                    );
                    if response.changed() {
                        app.set_cfg(&["ai", "apiKey"], serde_json::json!(value));
                    }
                    hint(ui, "建议使用独立、限额的 API Key");
                });
                ui.end_row();
            });
    });
}

fn application_paths(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Application 路径").strong().size(16.0));
            hint(ui, "由 Unity Editor 采样，未采集项会显示可验证的本机估算");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("刷新路径").clicked() {
                    app.load_settings_paths();
                }
            });
        });
        if app.settings_paths_loading {
            ui.horizontal(|ui| {
                ui.spinner();
                hint(ui, "正在读取 Profile 与 Application 路径…");
            });
            return;
        }
        let Some(snapshot) = app.settings_paths.clone() else {
            hint(ui, "尚未读取路径。请先配置 Unity 工程并扫描 Profile。");
            return;
        };
        let profiles = snapshot["profiles"].as_array().cloned().unwrap_or_default();
        if profiles.is_empty() {
            hint(
                ui,
                "当前工程没有可用 Profile；请先在“Profile 与节点”中扫描。",
            );
            return;
        }
        let current_asset = app.settings_path_asset.clone();
        let selected_name = profiles
            .iter()
            .find(|profile| profile["assetPath"].as_str() == Some(current_asset.as_str()))
            .and_then(|profile| profile["name"].as_str())
            .or_else(|| snapshot["assetPath"].as_str())
            .unwrap_or("选择 Profile");
        let mut chosen_asset = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Profile").small().color(MUTED));
            egui::ComboBox::from_id_salt("settings-path-profile")
                .selected_text(selected_name)
                .width(360.0)
                .show_ui(ui, |ui| {
                    for profile in &profiles {
                        let path = profile["assetPath"].as_str().unwrap_or("");
                        if ui
                            .selectable_label(
                                current_asset == path,
                                profile["name"].as_str().unwrap_or(path),
                            )
                            .clicked()
                        {
                            chosen_asset = Some(path.to_owned());
                        }
                    }
                });
            hint(
                ui,
                snapshot["capturedAt"]
                    .as_str()
                    .map(|time| format!("采集时间：{time}"))
                    .unwrap_or_else(|| "尚未由 Unity 采集本次 Profile".into()),
            );
        });
        if let Some(asset) = chosen_asset {
            if asset != current_asset {
                app.settings_path_asset = asset;
                app.load_settings_paths();
            }
        }
        hint(
            ui,
            snapshot["context"]
                .as_str()
                .unwrap_or("本机 Unity Editor；Player 和移动设备的路径可能不同"),
        );
        ui.add_space(4.0);
        let asset_path = app.settings_path_asset.clone();
        let paths = snapshot["paths"].as_array().cloned().unwrap_or_default();
        egui::Frame::new()
            .fill(CARD_ALT)
            .corner_radius(6.0)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                if paths.is_empty() {
                    hint(ui, "没有可显示的路径。");
                }
                for entry in &paths {
                    let id = entry["id"].as_str().unwrap_or("");
                    let name = entry["name"].as_str().unwrap_or(id);
                    let path = entry["path"].as_str().unwrap_or("");
                    let exists = entry["exists"].as_bool().unwrap_or(false);
                    let source = entry["source"].as_str().unwrap_or("");
                    let note = entry["note"].as_str().unwrap_or("");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).strong());
                        pill(
                            ui,
                            if exists { "可用" } else { "未生成" },
                            if exists { OK } else { WARN },
                        );
                        if ui
                            .add_enabled(
                                exists,
                                egui::Button::new(if entry["file"] == true {
                                    "打开所在目录"
                                } else {
                                    "打开"
                                }),
                            )
                            .clicked()
                        {
                            app.open_application_path(
                                PathScope::Active,
                                asset_path.clone(),
                                id.to_owned(),
                            );
                        }
                    });
                    ui.add(
                        egui::Label::new(
                            RichText::new(if path.is_empty() {
                                "等待 Unity 采集"
                            } else {
                                path
                            })
                            .monospace()
                            .small()
                            .color(if exists { MUTED } else { WARN }),
                        )
                        .selectable(true),
                    );
                    if !source.is_empty() || !note.is_empty() {
                        hint(
                            ui,
                            if note.is_empty() {
                                source.to_owned()
                            } else {
                                format!("{source} · {note}")
                            },
                        );
                    }
                    ui.separator();
                }
            });
    });
}

fn project_paths(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("项目快捷目录").strong().size(16.0));
            hint(ui, "工程、Assets、Profile、输出、归档与仓库目录");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("刷新").clicked() {
                    app.load_settings_paths();
                }
            });
        });
        ui.add_space(4.0);
        let paths = app.settings_project_paths.clone();
        if paths.is_empty() {
            hint(ui, "请先配置并保存 Unity 工程路径。");
        } else {
            egui::Grid::new("settings-project-paths-grid")
                .num_columns(3)
                .spacing([12.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.label(RichText::new("目录").small().color(MUTED));
                    ui.label(RichText::new("路径").small().color(MUTED));
                    ui.label(RichText::new("操作").small().color(MUTED));
                    ui.end_row();
                    for entry in &paths {
                        let id = entry["id"].as_str().unwrap_or("");
                        let name = entry["name"].as_str().unwrap_or(id);
                        let path = entry["path"].as_str().unwrap_or("");
                        let exists = entry["exists"].as_bool().unwrap_or(false);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).strong());
                            if !exists {
                                pill(ui, "未生成", WARN);
                            }
                        });
                        ui.add(
                            egui::Label::new(RichText::new(path).monospace().small().color(MUTED))
                                .truncate(),
                        )
                        .on_hover_text(path);
                        ui.add_enabled_ui(exists, |ui| {
                            if ui.small_button("打开").clicked() {
                                app.open_project_path(id);
                            }
                        });
                        ui.end_row();
                    }
                });
        }
        ui.add_space(6.0);
        hint(
            ui,
            "安装版配置目录：%APPDATA%\\com.unitybuildtool.admin\\config.json；升级与卸载默认保留此配置。",
        );
    });
}
