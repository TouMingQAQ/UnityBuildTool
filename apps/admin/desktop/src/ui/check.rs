use super::{
    card, cfg_checkbox, cfg_i64, cfg_text, danger_button, draw_log_panel, hint, pill,
    primary_button, secondary_button, section_title, status_color, status_label, ACCENT, CARD_ALT,
    ERR, MUTED, OK, WARN,
};
use crate::app::{env_targets, AdminApp};
use eframe::egui;
use egui::RichText;
use serde_json::Value;

pub fn draw(ui: &mut egui::Ui, app: &mut AdminApp) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    section_title(ui, "环境检测", "切换目标平台做真实编译检查，不执行完整打包");
                    hint(ui, "结果和日志保存在检测工程 .buildtool/check-results");
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let state = app.check_job["state"].as_str().unwrap_or("idle");
                    pill(ui, status_label(state), status_color(state));
                });
            });
            ui.add_space(10.0);
            check_config(ui, app);
            ui.add_space(10.0);
            check_results(ui, app);
            ui.add_space(10.0);
            let ctx = ui.ctx().clone();
            card(ui, |ui| {
                draw_log_panel(
                    ui,
                    &ctx,
                    &mut app.check_log,
                    "检测日志",
                    |log| log.clear(),
                    300.0,
                );
            });
        });
}

fn check_config(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("检测配置").strong().size(16.0));
            hint(ui, "可独立于打包工程使用另一个工程副本");
        });
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Unity 工程").small().color(MUTED));
            cfg_text(
                ui,
                app,
                &["check", "projectPath"],
                "例如 D:\\UnityProject\\Game 或 ..\\..",
                480.0,
            );
            if ui.small_button("使用构建工程").clicked() {
                let value = app.cfg_str(&["projectPath"]);
                app.set_cfg(&["check", "projectPath"], serde_json::json!(value));
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Unity.exe").small().color(MUTED));
            cfg_text(
                ui,
                app,
                &["check", "unityExe"],
                "留空则使用构建引擎配置",
                480.0,
            );
            if ui.small_button("自动探测").clicked() {
                app.request_unity_detect();
            }
        });
        ui.horizontal_wrapped(|ui| {
            cfg_i64(
                ui,
                app,
                &["check", "timeoutMinutes"],
                "单条超时（分钟）",
                20,
                70.0,
            );
            cfg_checkbox(
                ui,
                app,
                &["check", "nographics"],
                "无图形模式 -nographics",
                true,
            );
            if ui.small_button("使用构建 Unity 路径").clicked() {
                let value = app.cfg_str(&["engine", "unityExe"]);
                app.set_cfg(&["check", "unityExe"], serde_json::json!(value));
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("环境线").small().color(MUTED));
        ui.horizontal_wrapped(|ui| {
            for (target, name) in env_targets() {
                let active = app.check_targets.contains(target);
                let button = egui::Button::new(
                    RichText::new(format!("{name}  ·  BuildTarget {target}"))
                        .small()
                        .color(if active { egui::Color32::WHITE } else { MUTED }),
                )
                .fill(if active {
                    ACCENT.gamma_multiply(0.22)
                } else {
                    CARD_ALT
                })
                .stroke(egui::Stroke::new(
                    1.0,
                    if active {
                        ACCENT
                    } else {
                        egui::Color32::from_rgb(30, 41, 59)
                    },
                ));
                if ui.add(button).clicked() {
                    if active {
                        app.check_targets.retain(|value| value != target);
                    } else {
                        app.check_targets.push(*target);
                    }
                    app.check_targets.sort_unstable();
                    app.save_check_targets();
                }
            }
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.add(primary_button("开始检测")).clicked() {
                app.start_check();
            }
            if ui.add(danger_button("停止")).clicked() {
                app.stop_check();
            }
            if ui.add(secondary_button("命令预览")).clicked() {
                app.request_check_preview();
            }
            if !app.check_run_dir.is_empty() && ui.button("打开结果目录").clicked() {
                let path = app.check_run_dir.clone();
                app.open_folder(Some(app.selected_project_id.clone()), &path);
            }
            hint(
                ui,
                format!(
                    "{} 条环境线 · 预计最长 {} 分钟/条",
                    app.check_targets.len(),
                    app.cfg_i64(&["check", "timeoutMinutes"], 20)
                ),
            );
        });
        if let Some(end) = app.check_job.get("end").filter(|end| end.is_object()) {
            ui.add_space(5.0);
            let ok = end["ok"].as_bool() == Some(true);
            let reason = end["reason"].as_str().unwrap_or("");
            let summary = &end["summary"];
            ui.horizontal_wrapped(|ui| {
                pill(
                    ui,
                    if ok {
                        "全部通过"
                    } else if reason == "cancelled" {
                        "已终止"
                    } else {
                        "存在失败项"
                    },
                    if ok {
                        OK
                    } else if reason == "cancelled" {
                        WARN
                    } else {
                        ERR
                    },
                );
                for key in ["ok", "fail", "error", "timeout", "unsupported"] {
                    if let Some(value) = summary[key].as_u64() {
                        if value > 0 {
                            hint(ui, format!("{key}: {value}"));
                        }
                    }
                }
            });
        }
    });
}

fn check_results(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("检测结果").strong().size(16.0));
            hint(ui, "按环境线展示编译结果和耗时");
        });
        if app.check_targets.is_empty() {
            hint(ui, "尚未选择环境线。");
            return;
        }
        ui.add_space(5.0);
        let targets = app.check_targets.clone();
        egui::Grid::new("check-result-grid")
            .num_columns(4)
            .striped(true)
            .spacing([16.0, 7.0])
            .show(ui, |ui| {
                ui.label(RichText::new("环境线").small().color(MUTED));
                ui.label(RichText::new("状态").small().color(MUTED));
                ui.label(RichText::new("耗时").small().color(MUTED));
                ui.label(RichText::new("信息").small().color(MUTED));
                ui.end_row();
                for target in targets {
                    let name = env_targets()
                        .iter()
                        .find(|(value, _)| *value == target)
                        .map(|(_, name)| *name)
                        .unwrap_or("未知环境");
                    let result = app
                        .check_results
                        .get(&target)
                        .cloned()
                        .unwrap_or(Value::Null);
                    let status = result["status"].as_str().unwrap_or("pending");
                    ui.label(RichText::new(format!("{name} · BuildTarget {target}")).strong());
                    pill(ui, status_label(status), status_color(status));
                    let ms = result["ms"]
                        .as_u64()
                        .map(|ms| format!("{:.1}s", ms as f64 / 1000.0))
                        .unwrap_or_else(|| {
                            if status == "running" {
                                "进行中".to_owned()
                            } else {
                                "—".to_owned()
                            }
                        });
                    ui.label(RichText::new(ms).small().color(MUTED));
                    let message = result["message"]
                        .as_str()
                        .filter(|message| !message.is_empty())
                        .unwrap_or("等待检测…");
                    ui.add(
                        egui::Label::new(RichText::new(message).small().color(MUTED)).truncate(),
                    )
                    .on_hover_text(message);
                    ui.end_row();
                }
            });
    });
}
