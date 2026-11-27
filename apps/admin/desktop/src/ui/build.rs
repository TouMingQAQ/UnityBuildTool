use super::{
    card, cfg_checkbox, cfg_text, danger_button, draw_log_panel, hint, pill, primary_button,
    secondary_button, section_title, status_color, status_label, ACCENT, ACCENT_ALT, CARD_ALT, ERR,
    MUTED, OK, WARN,
};
use crate::app::AdminApp;
use eframe::egui;
use egui::RichText;

pub fn draw(ui: &mut egui::Ui, app: &mut AdminApp) {
    let panel_rect = ui.max_rect();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    section_title(ui, "构建", "选择节点、校验命令并跟踪原生构建进度");
                    hint(
                        ui,
                        format!(
                            "{} · {}",
                            app.current_project()
                                .and_then(|project| project["name"].as_str())
                                .unwrap_or("未选择项目"),
                            app.cfg_str(&["projectPath"])
                        ),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let state = [&app.job, &app.check_job, &app.vcs_job]
                        .into_iter()
                        .filter_map(|job| job["state"].as_str())
                        .find(|state| matches!(*state, "queued" | "starting" | "running"))
                        .unwrap_or("idle");
                    ui.label(
                        RichText::new(format!("● {}", super::task_label(app)))
                            .small()
                            .strong()
                            .color(status_color(state)),
                    );
                    let label = if app.build_drawer_open {
                        "收起详情"
                    } else {
                        "构建详情"
                    };
                    let button = if app.busy() {
                        primary_button(label)
                    } else {
                        secondary_button(label)
                    };
                    if ui
                        .add(button)
                        .on_hover_text("查看当前构建进度与实时日志")
                        .clicked()
                    {
                        app.build_drawer_open = !app.build_drawer_open;
                    }
                });
            });
            ui.add_space(10.0);

            build_selection(ui, app);
            ui.add_space(10.0);
            build_outputs(ui, app);
        });
    draw_build_drawer(ui.ctx(), panel_rect, app);
}

fn draw_build_drawer(ctx: &egui::Context, panel_rect: egui::Rect, app: &mut AdminApp) {
    if app.build_drawer_open && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        app.build_drawer_open = false;
    }
    let raw = ctx.animate_bool_with_time(
        egui::Id::unique("build-detail-drawer"),
        app.build_drawer_open,
        0.22,
    );
    let openness = egui::emath::easing::cubic_out(raw);
    if openness <= 0.001 {
        return;
    }

    let mut close = false;
    let width = (panel_rect.width() * 0.82)
        .clamp(380.0, 620.0)
        .min((panel_rect.width() - 16.0).max(280.0));
    let height = (panel_rect.height() - 16.0).max(240.0);
    let x = panel_rect.right() - 8.0 - width + (1.0 - openness) * width;
    let origin = egui::pos2(x, panel_rect.top() + 8.0);

    egui::Area::new(egui::Id::unique("build-detail-scrim"))
        .order(egui::Order::Middle)
        .fixed_pos(panel_rect.min)
        .show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(panel_rect.size(), egui::Sense::click());
            ui.painter().rect_filled(
                rect,
                0.0,
                egui::Color32::from_black_alpha((74.0 * openness).round() as u8),
            );
            if response.clicked() {
                close = true;
            }
        });

    egui::Area::new(egui::Id::unique("build-detail-drawer"))
        .order(egui::Order::Foreground)
        .fixed_pos(origin)
        .show(ctx, |ui| {
            ui.set_min_size(egui::vec2(width, height));
            ui.set_max_size(egui::vec2(width, height));
            ui.set_opacity(openness);
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(9, 14, 26))
                .stroke(egui::Stroke::new(
                    1.0,
                    ACCENT.gamma_multiply(0.35 + 0.25 * openness),
                ))
                .corner_radius(12.0)
                .inner_margin(egui::Margin::same(14))
                .shadow(egui::epaint::Shadow {
                    offset: [-8, 0],
                    blur: 30,
                    spread: 2,
                    color: egui::Color32::from_black_alpha(150),
                })
                .show(ui, |ui| {
                    ui.set_width(width - 28.0);
                    ui.set_height(height - 28.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("构建详情").strong().size(18.0));
                        let state = [&app.job, &app.check_job, &app.vcs_job]
                            .into_iter()
                            .filter_map(|job| job["state"].as_str())
                            .find(|state| matches!(*state, "queued" | "starting" | "running"))
                            .unwrap_or("idle");
                        pill(ui, &super::task_label(app), status_color(state));
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui.small_button("收起").clicked() {
                                    close = true;
                                }
                            },
                        );
                    });
                    hint(
                        ui,
                        "实时进度与日志抽屉；产物仍保留在页面主区域，构建归档不会包含日志文件。",
                    );
                    ui.add_space(8.0);
                    let ctx = ui.ctx().clone();
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            build_runtime(ui, app);
                            ui.add_space(10.0);
                            card(ui, |ui| {
                                draw_log_panel(
                                    ui,
                                    &ctx,
                                    &mut app.build_log,
                                    "构建日志",
                                    |log| log.clear(),
                                    390.0,
                                );
                                hint(
                                    ui,
                                    "仅保留当前 UI 最近 5000 行；完整构建日志仍保存在各节点输出目录的 build.log。",
                                );
                            });
                        });
                });
        });

    if close {
        app.build_drawer_open = false;
    }
}

fn build_selection(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("构建选择").strong().size(16.0));
            hint(ui, "构建组用于筛选，勾选决定本次实际执行节点");
        });
        ui.add_space(6.0);
        let groups = app.build_groups();
        let before = app.build_group.clone();
        let selected_group_name = if app.build_group == "all" {
            "全部构建组".to_owned()
        } else {
            groups
                .iter()
                .find(|group| group["id"].as_str() == Some(app.build_group.as_str()))
                .and_then(|group| group["name"].as_str())
                .unwrap_or("全部构建组")
                .to_owned()
        };
        let mut switch_group = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("构建组").small().color(MUTED));
            egui::ComboBox::from_id_salt("build-page-group")
                .selected_text(selected_group_name)
                .width(240.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(app.build_group == "all", "全部构建组")
                        .clicked()
                    {
                        switch_group = Some("all".to_owned());
                    }
                    for group in &groups {
                        let id = group["id"].as_str().unwrap_or("");
                        let label = format!(
                            "{} · {} 个节点",
                            group["name"].as_str().unwrap_or("未命名构建组"),
                            group["nodeIds"].as_array().map(Vec::len).unwrap_or(0)
                        );
                        if ui.selectable_label(app.build_group == id, label).clicked() {
                            switch_group = Some(id.to_owned());
                        }
                    }
                });
            if let Some(group) = switch_group {
                if group != before {
                    app.build_group = group;
                    app.set_cfg(&["activeBuildGroupId"], serde_json::json!(app.build_group));
                    let ids = app.active_group_node_ids();
                    app.build_selection = ids.into_iter().collect();
                }
            }
            if ui.add(secondary_button("全选当前组")).clicked() {
                let ids = app.active_group_node_ids();
                app.build_selection = ids.into_iter().collect();
            }
            if ui.button("清空选择").clicked() {
                app.build_selection.clear();
            }
            pill(
                ui,
                &format!("已选 {} 个", app.build_selection.len()),
                if app.build_selection.is_empty() {
                    WARN
                } else {
                    ACCENT
                },
            );
        });

        ui.add_space(6.0);
        let nodes = app.build_nodes();
        let visible = nodes
            .into_iter()
            .filter(|node| {
                app.build_group == "all"
                    || app
                        .group_of_node(node["id"].as_str().unwrap_or(""))
                        .and_then(|group| group["id"].as_str().map(str::to_owned))
                        .as_deref()
                        == Some(app.build_group.as_str())
            })
            .collect::<Vec<_>>();
        if visible.is_empty() {
            egui::Frame::new()
                .fill(CARD_ALT)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("当前构建组没有可选节点。").color(WARN));
                        if ui.small_button("扫描 Profile").clicked() {
                            app.start_scan();
                        }
                        if ui.small_button("前往 Profile 与节点").clicked() {
                            app.page = crate::app::Page::Profiles;
                        }
                    });
                });
        } else {
            egui::Grid::new("build-node-selection")
                .num_columns(4)
                .spacing([16.0, 8.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.label(RichText::new("执行").small().color(MUTED));
                    ui.label(RichText::new("节点").small().color(MUTED));
                    ui.label(RichText::new("平台").small().color(MUTED));
                    ui.label(RichText::new("Profile").small().color(MUTED));
                    ui.end_row();
                    for node in visible {
                        let id = node["id"].as_str().unwrap_or("").to_owned();
                        let mut selected = app.build_selection.contains(&id);
                        if ui.checkbox(&mut selected, "").changed() {
                            if selected {
                                app.build_selection.insert(id.clone());
                            } else {
                                app.build_selection.remove(&id);
                            }
                        }
                        ui.label(
                            RichText::new(node["name"].as_str().unwrap_or("未命名节点")).strong(),
                        );
                        let target = node["targetName"].as_str().unwrap_or("未指定平台");
                        pill(
                            ui,
                            target,
                            if target.contains("Android") {
                                OK
                            } else {
                                ACCENT_ALT
                            },
                        );
                        let profile = node["profileAssetPath"].as_str().unwrap_or("项目默认");
                        ui.label(RichText::new(profile).small().color(MUTED))
                            .on_hover_text(profile);
                        ui.end_row();
                    }
                });
        }

        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            let build_number = app.cfg_str(&["buildNumber"]);
            let mut automatic = build_number.trim().is_empty()
                || build_number
                    .trim()
                    .parse::<i64>()
                    .map(|n| n < 0)
                    .unwrap_or(true);
            if ui.checkbox(&mut automatic, "自动构建号").changed() {
                app.set_cfg(
                    &["buildNumber"],
                    serde_json::json!(if automatic { "-1" } else { "1" }),
                );
            }
            if !automatic {
                ui.label(RichText::new("固定构建号").small().color(MUTED));
                cfg_text(ui, app, &["buildNumber"], "例如 108", 90.0);
                hint(ui, "所有节点共用该构建号；Android 会写回版本号文件");
            } else {
                hint(ui, "自动模式读取并递增 Unity AndroidBundleVersionCode");
            }
            ui.separator();
            cfg_checkbox(ui, app, &["stopOnError"], "失败后停止后续节点", true);
            cfg_checkbox(ui, app, &["defaultAutoZip"], "默认自动压缩", true);
            cfg_checkbox(ui, app, &["defaultDevBuild"], "默认 DEV 包", false);
        });

        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.add(primary_button("开始构建")).clicked() {
                app.start_build();
            }
            if ui.add(danger_button("停止")).clicked() {
                app.stop_build();
            }
            if ui.add(secondary_button("命令预览")).clicked() {
                app.request_build_preview();
            }
            if ui
                .add(secondary_button("AI 失败分析"))
                .on_hover_text("使用设置页中的 AI 配置分析最近一次构建日志")
                .clicked()
            {
                app.request_ai();
            }
            ui.add_space(6.0);
            if app.job["state"].as_str() == Some("queued") {
                pill(ui, "等待全局构建队列", WARN);
            } else if app.busy() {
                pill(ui, "任务执行中，写操作已禁用", WARN);
            } else if app.save_error.is_some() {
                pill(ui, "配置未保存", ERR);
                if ui.small_button("重试保存").clicked() {
                    app.retry_save();
                }
            } else if app.config != app.saved_config {
                pill(ui, "开始前将自动保存", MUTED);
            }
        });
    });
}

fn build_runtime(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("运行状态").strong().size(16.0));
            let scheduler = &app.status["scheduler"];
            let active = scheduler["activeProjectId"]
                .as_str()
                .and_then(|id| {
                    app.projects
                        .iter()
                        .find(|project| project["id"].as_str() == Some(id))
                })
                .and_then(|project| project["name"].as_str())
                .unwrap_or("空闲");
            let pending = scheduler["pendingProjectIds"]
                .as_array()
                .map(Vec::len)
                .unwrap_or(0);
            hint(ui, format!("全局队列：{active} · {pending} 个项目等待"));
        });
        let state = app.job["state"].as_str().unwrap_or("");
        if state.is_empty() || state == "done" {
            if let Some(end) = app.job.get("end").filter(|value| value.is_object()) {
                let ok = end["ok"].as_bool() == Some(true);
                ui.horizontal_wrapped(|ui| {
                    pill(
                        ui,
                        if ok {
                            "构建完成"
                        } else {
                            "上次构建失败"
                        },
                        if ok { OK } else { ERR },
                    );
                    if let Some(message) = end["message"]
                        .as_str()
                        .filter(|message| !message.is_empty())
                    {
                        hint(ui, message);
                    }
                    if let Some(reason) = end["reason"].as_str() {
                        hint(ui, format!("原因：{reason}"));
                    }
                });
            } else {
                hint(ui, "尚未运行构建。选择节点后可先预览命令，再开始构建。");
            }
        } else {
            let index = app.job["index"].as_u64().unwrap_or(0) as usize;
            let count = app.job["count"]
                .as_u64()
                .unwrap_or_else(|| app.build_rows.len() as u64)
                .max(1) as usize;
            let (stage, pct) = app
                .build_progress
                .get(&index)
                .map(|(stage, pct, _)| (stage.clone(), *pct))
                .unwrap_or_else(|| ("等待开始…".into(), None));
            let overall = if state == "queued" {
                0.0
            } else {
                ((index as f32 + pct.unwrap_or(0.0) / 100.0).min(count as f32) / count as f32)
                    * 100.0
            };
            ui.add_space(6.0);
            ui.add(
                egui::ProgressBar::new(overall / 100.0)
                    .desired_width(f32::INFINITY)
                    .text(format!(
                        "{stage} · 第 {}/{} 项 · {:.0}%",
                        (index + 1).min(count),
                        count,
                        overall
                    ))
                    .fill(ACCENT.gamma_multiply(0.82)),
            );
        }

        ui.add_space(8.0);
        let rows = if app.build_rows.is_empty() {
            app.selected_build_nodes()
        } else {
            app.build_rows.clone()
        };
        if rows.is_empty() {
            hint(ui, "队列为空。");
            return;
        }
        let current = app.job["index"].as_u64().unwrap_or(0) as usize;
        egui::Grid::new("build-progress-grid")
            .num_columns(4)
            .striped(true)
            .spacing([16.0, 7.0])
            .show(ui, |ui| {
                ui.label(RichText::new("节点").small().color(MUTED));
                ui.label(RichText::new("平台").small().color(MUTED));
                ui.label(RichText::new("阶段").small().color(MUTED));
                ui.label(RichText::new("状态").small().color(MUTED));
                ui.end_row();
                for (index, row) in rows.iter().enumerate() {
                    let progress = app.build_progress.get(&index);
                    let (stage, pct, progress_state) = progress
                        .map(|(stage, pct, state)| (stage.clone(), *pct, state.as_str()))
                        .unwrap_or_else(|| {
                            if state == "running" && index == current {
                                ("执行中…".into(), None, "running")
                            } else if state == "done" && index < current {
                                ("完成".into(), Some(100.0), "done")
                            } else {
                                ("待执行".into(), None, "pending")
                            }
                        });
                    ui.label(RichText::new(row["name"].as_str().unwrap_or("未命名")).strong());
                    ui.label(
                        RichText::new(row["targetName"].as_str().unwrap_or(""))
                            .small()
                            .color(MUTED),
                    );
                    ui.label(
                        RichText::new(match pct {
                            Some(pct) => format!("{stage} · {pct:.0}%"),
                            None => stage,
                        })
                        .small()
                        .color(MUTED),
                    );
                    pill(
                        ui,
                        status_label(progress_state),
                        status_color(progress_state),
                    );
                    ui.end_row();
                }
            });
    });
}

fn build_outputs(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("构建产物").strong().size(16.0));
            hint(ui, "点击“打开”在资源管理器中定位");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("刷新产物").clicked() {
                    app.refresh_outputs();
                }
                if ui.small_button("打开输出目录").clicked() {
                    app.open_project_path("builds");
                }
            });
        });
        ui.add_space(5.0);
        if app.outputs.is_empty() {
            hint(
                ui,
                format!("暂无构建产物。输出目录：{}", app.cfg_str(&["outputBase"])),
            );
            return;
        }
        let outputs = app.outputs.clone();
        let project_id = app.selected_project_id.clone();
        egui::Grid::new("output-table")
            .num_columns(7)
            .striped(true)
            .spacing([12.0, 7.0])
            .show(ui, |ui| {
                for title in ["产物", "平台", "版本", "构建号", "大小", "时间", ""] {
                    ui.label(RichText::new(title).small().color(MUTED));
                }
                ui.end_row();
                for output in outputs {
                    let name = output["name"]
                        .as_str()
                        .or_else(|| output["file"].as_str())
                        .or_else(|| output["dir"].as_str())
                        .unwrap_or("未命名产物");
                    ui.label(RichText::new(name).strong()).on_hover_text(
                        output["file"]
                            .as_str()
                            .or_else(|| output["dir"].as_str())
                            .unwrap_or(""),
                    );
                    ui.label(
                        RichText::new(output["targetName"].as_str().unwrap_or(""))
                            .small()
                            .color(MUTED),
                    );
                    ui.label(
                        RichText::new(
                            output["version"]
                                .as_str()
                                .map(|version| format!("v{version}"))
                                .unwrap_or_default(),
                        )
                        .small()
                        .color(MUTED),
                    );
                    ui.label(
                        RichText::new(
                            output["versionCode"]
                                .as_i64()
                                .map(|code| format!("b{code}"))
                                .unwrap_or_default(),
                        )
                        .small()
                        .color(MUTED),
                    );
                    ui.label(
                        RichText::new(
                            output["size"]
                                .as_u64()
                                .map(crate::app::format_bytes)
                                .unwrap_or_default(),
                        )
                        .small()
                        .color(MUTED),
                    );
                    ui.label(
                        RichText::new(output["date"].as_str().unwrap_or(""))
                            .small()
                            .color(MUTED),
                    );
                    if ui.small_button("打开").clicked() {
                        let path = output["file"]
                            .as_str()
                            .or_else(|| output["dir"].as_str())
                            .unwrap_or("");
                        if !path.is_empty() {
                            app.open_folder(Some(project_id.clone()), path);
                        }
                    }
                    ui.end_row();
                }
            });
    });
}
