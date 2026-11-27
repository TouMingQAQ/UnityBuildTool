use super::{
    card, hint, pill, primary_button, section_title, status_color, status_label, ACCENT,
    ACCENT_ALT, BG, CARD, CARD_ALT, ERR, MUTED, OK, WARN,
};
use crate::app::{format_bytes, AdminApp, MonitorSample, PathScope};
use eframe::egui;
use egui::{Color32, RichText, Stroke};
use serde_json::Value;

pub fn draw(ui: &mut egui::Ui, app: &mut AdminApp) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            monitor_header(ui, app);
            ui.add_space(10.0);

            if app.monitor_snapshot.is_none() {
                card(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(18.0);
                        ui.spinner();
                        hint(ui, "正在采集全部项目与系统资源…");
                        if !app.monitor_error.is_empty() {
                            ui.label(RichText::new(&app.monitor_error).color(ERR));
                            hint(ui, "监控每 2 秒自动重试；切换页面后重新进入也会继续采样。");
                        }
                        ui.add_space(18.0);
                    });
                });
                return;
            }

            project_cards(ui, app);
            ui.add_space(10.0);
            scheduler_and_host(ui, app);
            ui.add_space(10.0);
            selected_project_detail(ui, app);
        });
}

fn monitor_header(ui: &mut egui::Ui, app: &mut AdminApp) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            section_title(
                ui,
                "监控",
                "跨项目调度、进程资源、Application 路径、产物与实时日志",
            );
            let sampled = app
                .monitor_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot["sampledAt"].as_str())
                .map(display_time)
                .unwrap_or_else(|| "等待采样".into());
            hint(
                ui,
                format!("仅监控页可见时每 2 秒采样 · 最近一次：{sampled}"),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(project) = selected_monitor_project(app) {
                let slug = project["slug"].as_str().unwrap_or("").to_owned();
                if ui.add(primary_button("打开用户网页")).clicked() {
                    app.open_project_website(&slug);
                }
            }
            if app.monitor_error.is_empty() {
                pill(ui, "实时采样", OK);
            } else {
                pill(ui, "采样失败", ERR);
            }
        });
    });
    if !app.monitor_error.is_empty() {
        ui.label(RichText::new(&app.monitor_error).small().color(ERR));
    }
}

fn project_cards(ui: &mut egui::Ui, app: &mut AdminApp) {
    let snapshot = app.monitor_snapshot.clone().unwrap_or(Value::Null);
    let projects = snapshot["projects"].as_array().cloned().unwrap_or_default();
    let mut selected_id = app.monitor_selected.clone();
    let mut switch_to = None;
    let mut open_slug = None;
    let mut ensure_selected = false;

    if selected_id.is_empty()
        || !projects
            .iter()
            .any(|project| project["id"].as_str() == Some(selected_id.as_str()))
    {
        selected_id = projects
            .first()
            .and_then(|project| project["id"].as_str())
            .unwrap_or("")
            .to_owned();
        ensure_selected = true;
    }

    ui.horizontal(|ui| {
        ui.label(RichText::new("全部项目").strong().size(16.0));
        pill(ui, &format!("{} 个项目", projects.len()), ACCENT_ALT);
        hint(
            ui,
            "点击卡片切换下方详情；构建由全局 FIFO 队列跨项目串行调度",
        );
    });
    ui.add_space(7.0);

    let columns = if ui.available_width() >= 700.0 { 2 } else { 1 };
    let gap = 10.0;
    let card_width =
        ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32).max(240.0);
    for row in projects.chunks(columns) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for project in row {
                let actions = project_card(ui, project, card_width, selected_id.as_str());
                if actions.select {
                    switch_to = Some(project["id"].as_str().unwrap_or("").to_owned());
                }
                if actions.open_slug.is_some() {
                    open_slug = actions.open_slug;
                }
            }
        });
        ui.add_space(gap);
    }

    if projects.is_empty() {
        card(ui, |ui| hint(ui, "尚未配置项目。请到“设置”创建项目。"));
    }
    if let Some(slug) = open_slug {
        app.open_project_website(&slug);
    }
    if let Some(id) = switch_to {
        app.select_monitor_project(&id);
    } else if ensure_selected && !selected_id.is_empty() && app.monitor_selected != selected_id {
        app.select_monitor_project(&selected_id);
    }
}

#[derive(Default)]
struct ProjectCardActions {
    select: bool,
    open_slug: Option<String>,
}

fn project_card(
    ui: &mut egui::Ui,
    project: &Value,
    width: f32,
    selected_id: &str,
) -> ProjectCardActions {
    let id = project["id"].as_str().unwrap_or("");
    let selected = id == selected_id;
    let busy = project["busy"].as_bool().unwrap_or(false);
    let enabled = project["enabled"].as_bool().unwrap_or(true);
    let resources = &project["resources"];
    let job_state = project["job"]["state"].as_str().unwrap_or("idle");
    let check_state = project["check"]["state"].as_str().unwrap_or("idle");
    let vcs_state = project["vcs"]["state"].as_str().unwrap_or("idle");
    let log_count = project["logs"].as_array().map(Vec::len).unwrap_or(0);
    let mut actions = ProjectCardActions::default();

    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            let response = ui
                .scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
                    let hovered = ui.response().hovered();
                    let hover_t = ui.ctx().animate_bool_with_time(
                        egui::Id::unique(("monitor-card-hover", id)),
                        hovered,
                        0.12,
                    );
                    let select_t = ui.ctx().animate_bool_with_time(
                        egui::Id::unique(("monitor-card-selected", id)),
                        selected,
                        0.18,
                    );
                    let fill = CARD_ALT
                        .lerp_to_gamma(CARD, hover_t.max(select_t))
                        .lerp_to_gamma(ACCENT.gamma_multiply(0.16), select_t * 0.55);
                    let border = Color32::from_rgb(31, 41, 55)
                        .lerp_to_gamma(Color32::from_rgb(64, 78, 99), hover_t)
                        .lerp_to_gamma(ACCENT, select_t);
                    egui::Frame::new()
                        .fill(fill)
                        .stroke(Stroke::new(1.0 + select_t * 0.5, border))
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width((width - 24.0).max(120.0));
                            ui.horizontal(|ui| {
                                status_dot(
                                    ui,
                                    if busy {
                                        WARN
                                    } else if enabled {
                                        OK
                                    } else {
                                        MUTED
                                    },
                                );
                                let name = project["name"].as_str().unwrap_or("未命名项目");
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(name).strong().size(15.0).color(
                                            if selected || hovered {
                                                ACCENT
                                            } else {
                                                Color32::WHITE
                                            },
                                        ),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(name);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        pill(
                                            ui,
                                            if busy {
                                                "忙碌"
                                            } else if enabled {
                                                "启用"
                                            } else {
                                                "停用"
                                            },
                                            if busy {
                                                WARN
                                            } else if enabled {
                                                OK
                                            } else {
                                                MUTED
                                            },
                                        );
                                    },
                                );
                            });
                            ui.horizontal(|ui| {
                                hint(ui, format!("/p/{}", project["slug"].as_str().unwrap_or("")));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        hint(ui, format!("{log_count} 条日志"));
                                    },
                                );
                            });
                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);
                            ui.horizontal_wrapped(|ui| {
                                mini_metric(
                                    ui,
                                    "CPU",
                                    &format!(
                                        "{:.0}%",
                                        resources["cpuPercent"].as_f64().unwrap_or(0.0)
                                    ),
                                    ACCENT,
                                );
                                mini_metric(
                                    ui,
                                    "内存",
                                    &format_bytes(resources["memoryBytes"].as_u64().unwrap_or(0)),
                                    ACCENT_ALT,
                                );
                                mini_metric(
                                    ui,
                                    "进程",
                                    &resources["processCount"].as_u64().unwrap_or(0).to_string(),
                                    MUTED,
                                );
                            });
                            ui.add_space(7.0);
                            ui.horizontal(|ui| {
                                task_pill(ui, "构建", job_state);
                                task_pill(ui, "检测", check_state);
                                task_pill(ui, "版本", vcs_state);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui.small_button("详情").clicked() {
                                            actions.select = true;
                                        }
                                        if ui.small_button("网页").clicked() {
                                            actions.open_slug =
                                                project["slug"].as_str().map(str::to_owned);
                                        }
                                    },
                                );
                            });
                        });
                })
                .response;
            if response.clicked() {
                actions.select = true;
            }
            response.on_hover_cursor(egui::CursorIcon::PointingHand);
        },
    );
    actions
}

fn mini_metric(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).small().color(MUTED));
        ui.label(RichText::new(value).small().strong().color(color));
    });
}

fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

fn scheduler_and_host(ui: &mut egui::Ui, app: &AdminApp) {
    let snapshot = app.monitor_snapshot.clone().unwrap_or(Value::Null);
    let scheduler = &snapshot["scheduler"];
    let active_id = scheduler["activeProjectId"].as_str().unwrap_or("");
    let pending = scheduler["pendingProjectIds"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let project_name = |id: &str| {
        snapshot["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|project| project["id"].as_str() == Some(id))
            .and_then(|project| project["name"].as_str())
            .unwrap_or(id)
            .to_owned()
    };
    let used = snapshot["hostMemoryUsed"].as_u64().unwrap_or(0);
    let total = snapshot["hostMemoryTotal"].as_u64().unwrap_or(0);
    let backend = &snapshot["backend"];
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("调度与主机").strong().size(16.0));
            if active_id.is_empty() {
                pill(ui, "全局队列空闲", OK);
            } else {
                pill(ui, &format!("执行中 · {}", project_name(active_id)), ACCENT);
            }
            if !pending.is_empty() {
                pill(ui, &format!("{} 个项目等待", pending.len()), WARN);
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for id in &pending {
                if let Some(id) = id.as_str() {
                    pill(ui, &project_name(id), ACCENT_ALT);
                }
            }
            if pending.is_empty() && active_id.is_empty() {
                hint(ui, "当前没有构建、检测或版本更新任务。");
            }
        });
        ui.add_space(5.0);
        ui.horizontal_wrapped(|ui| {
            resource_pill(
                ui,
                "管理端 CPU",
                &format!("{:.0}%", backend["cpuPercent"].as_f64().unwrap_or(0.0)),
                ACCENT,
            );
            resource_pill(
                ui,
                "管理端内存",
                &format_bytes(backend["memoryBytes"].as_u64().unwrap_or(0)),
                ACCENT_ALT,
            );
            resource_pill(
                ui,
                "管理端进程",
                &backend["processCount"].as_u64().unwrap_or(0).to_string(),
                MUTED,
            );
            if total > 0 {
                resource_pill(
                    ui,
                    "主机内存",
                    &format!("{} / {}", format_bytes(used), format_bytes(total)),
                    OK,
                );
            }
        });
    });
}

fn selected_project_detail(ui: &mut egui::Ui, app: &mut AdminApp) {
    let Some(project) = selected_monitor_project(app).cloned() else {
        card(ui, |ui| hint(ui, "请选择一个项目查看监控详情。"));
        return;
    };
    let id = project["id"].as_str().unwrap_or("").to_owned();
    let name = project["name"].as_str().unwrap_or("未命名项目").to_owned();
    let slug = project["slug"].as_str().unwrap_or("").to_owned();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{name} · 实时详情"))
                .strong()
                .size(17.0),
        );
        pill(
            ui,
            if project["busy"].as_bool() == Some(true) {
                "忙碌"
            } else {
                "空闲"
            },
            if project["busy"].as_bool() == Some(true) {
                WARN
            } else {
                OK
            },
        );
        hint(ui, format!("/p/{slug}"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("打开用户网页").clicked() {
                app.open_project_website(&slug);
            }
        });
    });
    ui.add_space(5.0);

    trend_card(ui, app, &id);
    ui.add_space(10.0);
    task_card(ui, &project);
    ui.add_space(10.0);
    application_paths_card(ui, app);
    ui.add_space(10.0);
    artifacts_card(ui, app);
    ui.add_space(10.0);
    logs_card(ui, app, &project);
}

fn trend_card(ui: &mut egui::Ui, app: &AdminApp, project_id: &str) {
    let project_history = app
        .monitor_history
        .get(project_id)
        .cloned()
        .unwrap_or_default();
    let backend_history = app
        .monitor_history
        .get("backend")
        .cloned()
        .unwrap_or_default();
    let resources = selected_monitor_project(app)
        .map(|project| project["resources"].clone())
        .unwrap_or(Value::Null);
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("CPU / 内存趋势").strong().size(16.0));
            hint(ui, "最近约 2 分钟；每项目保留 60 个采样点");
        });
        ui.add_space(5.0);
        ui.columns(2, |columns| {
            columns[0].allocate_ui_with_layout(
                egui::vec2(columns[0].available_width(), 160.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("当前项目").strong());
                        pill(
                            ui,
                            &format!(
                                "CPU {:.0}%",
                                resources["cpuPercent"].as_f64().unwrap_or(0.0)
                            ),
                            ACCENT,
                        );
                        pill(
                            ui,
                            &format_bytes(resources["memoryBytes"].as_u64().unwrap_or(0)),
                            ACCENT_ALT,
                        );
                    });
                    trend_chart(ui, &project_history);
                },
            );
            columns[1].allocate_ui_with_layout(
                egui::vec2(columns[1].available_width(), 160.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("管理端进程树").strong());
                        pill(ui, "CPU", ACCENT);
                        pill(ui, "内存", ACCENT_ALT);
                    });
                    trend_chart(ui, &backend_history);
                },
            );
        });
    });
}

fn trend_chart(ui: &mut egui::Ui, samples: &std::collections::VecDeque<MonitorSample>) {
    let width = ui.available_width().max(180.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 95.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 5.0, BG);
    painter.rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, Color32::from_rgb(30, 41, 59)),
        egui::StrokeKind::Inside,
    );
    for index in 1..4 {
        let y = rect.top() + rect.height() * index as f32 / 4.0;
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            Stroke::new(1.0, Color32::from_rgb(20, 30, 48)),
        );
    }
    if samples.len() < 2 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "等待更多采样点…",
            egui::FontId::proportional(12.0),
            MUTED,
        );
        return;
    }
    let max_memory = samples
        .iter()
        .map(|sample| sample.memory)
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let point = |index: usize, value: f32| {
        let x = rect.left() + rect.width() * index as f32 / (samples.len().max(2) - 1) as f32;
        let y = rect.bottom() - rect.height() * value.clamp(0.0, 1.0);
        egui::pos2(x, y)
    };
    let cpu_points = samples
        .iter()
        .enumerate()
        .map(|(index, sample)| point(index, sample.cpu / 100.0))
        .collect::<Vec<_>>();
    let memory_points = samples
        .iter()
        .enumerate()
        .map(|(index, sample)| point(index, sample.memory as f32 / max_memory))
        .collect::<Vec<_>>();
    painter.add(egui::Shape::line(
        memory_points,
        Stroke::new(1.7, ACCENT_ALT),
    ));
    painter.add(egui::Shape::line(cpu_points, Stroke::new(1.7, ACCENT)));
    let latest = samples.back().unwrap();
    painter.text(
        rect.left_top() + egui::vec2(7.0, 5.0),
        egui::Align2::LEFT_TOP,
        format!(
            "CPU {:.1}%  ·  内存 {}",
            latest.cpu,
            format_bytes(latest.memory)
        ),
        egui::FontId::monospace(11.0),
        Color32::from_rgb(203, 213, 225),
    );
    painter.text(
        rect.left_top() + egui::vec2(7.0, 20.0),
        egui::Align2::LEFT_TOP,
        format!(
            "峰值 {}",
            format_bytes(samples.iter().map(|s| s.memory).max().unwrap_or(0))
        ),
        egui::FontId::monospace(10.0),
        MUTED,
    );
    response.on_hover_text(format!(
        "CPU 蓝线 · 内存靛线\n{} 个采样点\nCPU 最近 {:.1}%\n内存最近 {}",
        samples.len(),
        latest.cpu,
        format_bytes(latest.memory)
    ));
}

fn task_card(ui: &mut egui::Ui, project: &Value) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("任务状态").strong().size(16.0));
            hint(ui, "构建、环境检测与版本更新互斥；全局队列按提交顺序执行");
        });
        ui.add_space(5.0);
        ui.columns(3, |columns| {
            task_summary(
                &mut columns[0],
                "构建",
                &project["job"],
                &[("队列", "queue"), ("输出", "outputs")],
            );
            task_summary(
                &mut columns[1],
                "环境检测",
                &project["check"],
                &[("环境线", "targets"), ("结果", "results")],
            );
            task_summary(
                &mut columns[2],
                "版本更新",
                &project["vcs"],
                &[("节点", "nodes"), ("结果", "results")],
            );
        });
    });
}

fn task_summary(ui: &mut egui::Ui, label: &str, job: &Value, collections: &[(&str, &str)]) {
    let state = job["state"].as_str().unwrap_or("idle");
    egui::Frame::new()
        .fill(CARD_ALT)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).strong());
                pill(ui, status_label(state), status_color(state));
            });
            if let Some(index) = job["index"].as_u64() {
                if let Some(count) = job["count"].as_u64() {
                    hint(ui, format!("进度 {}/{}", (index + 1).min(count), count));
                }
            }
            for (title, key) in collections {
                let count = match job[*key].as_object() {
                    Some(values) => values.len(),
                    None => job[*key].as_array().map(Vec::len).unwrap_or(0),
                };
                if count > 0 {
                    hint(ui, format!("{title}：{count}"));
                }
            }
            if let Some(end) = job.get("end").filter(|end| end.is_object()) {
                let ok = end["ok"].as_bool() == Some(true);
                let message = end["message"]
                    .as_str()
                    .filter(|message| !message.is_empty())
                    .or_else(|| end["reason"].as_str())
                    .unwrap_or(if ok { "完成" } else { "失败" });
                ui.label(
                    RichText::new(message)
                        .small()
                        .color(if ok { OK } else { ERR }),
                );
            } else if job.is_null() || state == "idle" {
                hint(ui, "尚未运行");
            }
        });
}

fn application_paths_card(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Application 路径").strong().size(16.0));
            hint(
                ui,
                "来自本机 Unity Editor 采样；Player / 移动设备路径可能不同",
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("刷新").clicked() {
                    app.load_monitor_paths();
                }
            });
        });
        if app.monitor_paths_loading {
            ui.horizontal(|ui| {
                ui.spinner();
                hint(ui, "正在读取路径…");
            });
            return;
        }
        if app.monitor_paths.is_none() {
            app.load_monitor_paths();
            hint(ui, "正在准备路径读取…");
            return;
        }
        let snapshot = app.monitor_paths.clone().unwrap_or(Value::Null);
        let profiles = snapshot["profiles"].as_array().cloned().unwrap_or_default();
        if profiles.is_empty() {
            hint(ui, "项目尚未配置工程路径或没有可扫描的 Profile。");
            return;
        }
        let current_asset = if app.monitor_path_asset.is_empty() {
            snapshot["assetPath"].as_str().unwrap_or("").to_owned()
        } else {
            app.monitor_path_asset.clone()
        };
        let selected_name = profiles
            .iter()
            .find(|profile| profile["assetPath"].as_str() == Some(current_asset.as_str()))
            .and_then(|profile| profile["name"].as_str())
            .unwrap_or("选择 Profile");
        let mut chosen_asset = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Profile").small().color(MUTED));
            egui::ComboBox::from_id_salt("monitor-application-profile")
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
                    .map(|time| format!("采集：{}", display_time(time)))
                    .unwrap_or_else(|| "尚未采集".into()),
            );
        });
        if let Some(asset) = chosen_asset {
            if asset != current_asset {
                app.monitor_path_asset = asset;
                app.load_monitor_paths();
            }
        }
        ui.add_space(4.0);
        let asset_path = if app.monitor_path_asset.is_empty() {
            snapshot["assetPath"].as_str().unwrap_or("").to_owned()
        } else {
            app.monitor_path_asset.clone()
        };
        egui::Frame::new()
            .fill(CARD_ALT)
            .corner_radius(6.0)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                let paths = snapshot["paths"].as_array().cloned().unwrap_or_default();
                if paths.is_empty() {
                    hint(ui, "没有可显示的路径。");
                }
                for entry in &paths {
                    let entry_id = entry["id"].as_str().unwrap_or("");
                    let exists = entry["exists"].as_bool().unwrap_or(false);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(entry["name"].as_str().unwrap_or(entry_id)).strong(),
                        );
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
                                PathScope::Monitor,
                                asset_path.clone(),
                                entry_id.to_owned(),
                            );
                        }
                        hint(ui, entry["note"].as_str().unwrap_or(""));
                    });
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                entry["path"]
                                    .as_str()
                                    .filter(|path| !path.is_empty())
                                    .unwrap_or("等待 Unity 采集"),
                            )
                            .monospace()
                            .small()
                            .color(MUTED),
                        )
                        .selectable(true),
                    );
                    ui.separator();
                }
            });
    });
}

fn artifacts_card(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("产物浏览").strong().size(16.0));
            hint(
                ui,
                "只读浏览构建根目录与节点自定义目录，点击文件仅打开所在文件夹",
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("刷新").clicked() {
                    app.load_artifacts("", "");
                }
            });
        });
        if app.monitor_artifact_loading {
            ui.horizontal(|ui| {
                ui.spinner();
                hint(ui, "正在读取目录…");
            });
            return;
        }
        if app.monitor_artifacts.is_none() {
            app.load_artifacts("", "");
            hint(ui, "正在准备产物目录读取…");
            return;
        }
        let snapshot = app.monitor_artifacts.clone().unwrap_or(Value::Null);
        let roots = snapshot["roots"].as_array().cloned().unwrap_or_default();
        if roots.is_empty() {
            hint(ui, "尚未配置工程或产物目录；构建后刷新即可查看。");
            return;
        }
        let current_root = snapshot["root"].as_str().unwrap_or("");
        let current_path = snapshot["path"].as_str().unwrap_or("");
        let root_name = roots
            .iter()
            .find(|root| root["id"].as_str() == Some(current_root))
            .and_then(|root| root["name"].as_str())
            .unwrap_or("构建目录");
        let mut chosen_root = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("目录根").small().color(MUTED));
            egui::ComboBox::from_id_salt("monitor-artifact-root")
                .selected_text(root_name)
                .width(330.0)
                .show_ui(ui, |ui| {
                    for root in &roots {
                        let root_id = root["id"].as_str().unwrap_or("");
                        if ui
                            .selectable_label(
                                current_root == root_id,
                                root["name"].as_str().unwrap_or(root_id),
                            )
                            .clicked()
                        {
                            chosen_root = Some(root_id.to_owned());
                        }
                    }
                });
            if !current_path.is_empty() && ui.small_button("↑ 上级").clicked() {
                app.load_artifacts(current_root, parent_path(current_path));
            }
            if ui.small_button("打开当前目录").clicked() {
                app.open_artifact_folder(current_root.to_owned(), current_path.to_owned());
            }
            hint(
                ui,
                if current_path.is_empty() {
                    "根目录".to_owned()
                } else {
                    format!("/{current_path}")
                },
            );
        });
        if let Some(root) = chosen_root {
            if root != current_root {
                app.load_artifacts(&root, "");
            }
        }
        ui.add_space(4.0);
        let entries = snapshot["entries"].as_array().cloned().unwrap_or_default();
        egui::Frame::new()
            .fill(BG)
            .stroke(Stroke::new(1.0, Color32::from_rgb(30, 41, 59)))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::same(7))
            .show(ui, |ui| {
                if entries.is_empty() {
                    hint(ui, "目录为空或尚未生成。");
                } else {
                    egui::Grid::new("monitor-artifact-grid")
                        .num_columns(4)
                        .spacing([12.0, 5.0])
                        .striped(true)
                        .show(ui, |ui| {
                            for label in ["名称", "类型", "大小", "修改时间"] {
                                ui.label(RichText::new(label).small().color(MUTED));
                            }
                            ui.end_row();
                            for entry in &entries {
                                let is_directory = entry["type"].as_str() == Some("directory");
                                let name = entry["name"].as_str().unwrap_or("");
                                let path = entry["path"].as_str().unwrap_or("");
                                if ui
                                    .selectable_label(
                                        false,
                                        RichText::new(format!(
                                            "{} {}",
                                            if is_directory { "▸" } else { "·" },
                                            name
                                        ))
                                        .monospace(),
                                    )
                                    .on_hover_text("单击目录进入；文件请用“打开”")
                                    .clicked()
                                    && is_directory
                                {
                                    app.load_artifacts(current_root, path);
                                }
                                ui.label(
                                    RichText::new(if is_directory {
                                        "文件夹"
                                    } else {
                                        "文件"
                                    })
                                    .small()
                                    .color(MUTED),
                                );
                                ui.label(
                                    RichText::new(if is_directory {
                                        "—".to_owned()
                                    } else {
                                        format_bytes(entry["size"].as_u64().unwrap_or(0))
                                    })
                                    .small()
                                    .color(MUTED),
                                );
                                ui.horizontal(|ui| {
                                    hint(
                                        ui,
                                        entry["modified"]
                                            .as_str()
                                            .map(display_time)
                                            .unwrap_or_else(|| "—".into()),
                                    );
                                    if ui.small_button("打开").clicked() {
                                        app.open_artifact_folder(
                                            current_root.to_owned(),
                                            path.to_owned(),
                                        );
                                    }
                                });
                                ui.end_row();
                            }
                        });
                }
            });
    });
}

fn logs_card(ui: &mut egui::Ui, app: &mut AdminApp, project: &Value) {
    let logs = project["logs"].as_array().cloned().unwrap_or_default();
    let channel = app.monitor_log_channel.clone();
    let query = app.monitor_log_query.to_lowercase();
    let filtered = logs
        .iter()
        .filter(|entry| channel == "all" || log_channel(entry) == channel)
        .filter(|entry| {
            query.is_empty()
                || entry["text"]
                    .as_str()
                    .map(|text| text.to_lowercase().contains(&query))
                    .unwrap_or(false)
        })
        .cloned()
        .collect::<Vec<_>>();
    let document = filtered
        .iter()
        .map(|entry| {
            format!(
                "[{}] {}",
                entry["time"]
                    .as_str()
                    .map(display_time)
                    .unwrap_or_else(|| "--:--:--".into()),
                entry["text"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("会话日志").strong().size(16.0));
            hint(ui, "后台仅保留每个项目最近 300 条结构化事件");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("复制筛选结果").clicked() {
                    ui.ctx().copy_text(document.clone());
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("monitor-log-channel")
                .selected_text(match channel.as_str() {
                    "build" => "构建频道",
                    "check" => "检测频道",
                    "vcs" => "版本频道",
                    _ => "全部频道",
                })
                .width(150.0)
                .show_ui(ui, |ui| {
                    for (value, label) in [
                        ("all", "全部频道"),
                        ("build", "构建频道"),
                        ("check", "检测频道"),
                        ("vcs", "版本频道"),
                    ] {
                        if ui.selectable_label(channel == value, label).clicked() {
                            app.monitor_log_channel = value.into();
                        }
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut app.monitor_log_query)
                    .hint_text("搜索日志")
                    .desired_width(240.0),
            );
            ui.checkbox(&mut app.monitor_log_follow, "自动滚动");
            pill(ui, &format!("{} 条", filtered.len()), ACCENT);
        });
        egui::Frame::new()
            .fill(BG)
            .stroke(Stroke::new(1.0, Color32::from_rgb(30, 41, 59)))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("monitor-logs-scroll")
                    .stick_to_bottom(app.monitor_log_follow)
                    .max_height(260.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if filtered.is_empty() {
                            hint(ui, "（当前筛选条件下暂无日志）");
                        }
                        for entry in &filtered {
                            let color = match log_channel(entry) {
                                "check" => WARN,
                                "vcs" => ACCENT_ALT,
                                _ => Color32::from_rgb(203, 213, 225),
                            };
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(
                                        entry["time"]
                                            .as_str()
                                            .map(display_time)
                                            .unwrap_or_else(|| "--:--:--".into()),
                                    )
                                    .monospace()
                                    .small()
                                    .color(MUTED),
                                );
                                ui.label(
                                    RichText::new(entry["type"].as_str().unwrap_or("event"))
                                        .monospace()
                                        .small()
                                        .color(color),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(entry["text"].as_str().unwrap_or(""))
                                            .monospace()
                                            .small()
                                            .color(color),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Extend),
                                );
                            });
                        }
                    });
            });
    });
}

fn selected_monitor_project(app: &AdminApp) -> Option<&Value> {
    app.monitor_snapshot
        .as_ref()?
        .get("projects")?
        .as_array()?
        .iter()
        .find(|project| project["id"].as_str() == Some(app.monitor_selected.as_str()))
}

fn resource_pill(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.11))
        .corner_radius(5.0)
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).small().color(MUTED));
                ui.label(RichText::new(value).small().strong().color(color));
            });
        });
}

fn task_pill(ui: &mut egui::Ui, label: &str, state: &str) {
    pill(
        ui,
        &format!("{label} · {}", status_label(state)),
        status_color(state),
    );
}

fn log_channel(entry: &Value) -> &'static str {
    match entry["type"].as_str().unwrap_or("") {
        event if event.starts_with("check") => "check",
        event if event.starts_with("vcs") => "vcs",
        _ => "build",
    }
}

fn parent_path(path: &str) -> &str {
    path.rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("")
}

fn display_time(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|_| value.chars().take(19).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_log_channels() {
        assert_eq!(log_channel(&json!({"type":"line"})), "build");
        assert_eq!(log_channel(&json!({"type":"output"})), "build");
        assert_eq!(log_channel(&json!({"type":"check-line"})), "check");
        assert_eq!(log_channel(&json!({"type":"vcs-line"})), "vcs");
    }

    #[test]
    fn parent_path_handles_roots_and_nested_paths() {
        assert_eq!(parent_path(""), "");
        assert_eq!(parent_path("one"), "");
        assert_eq!(parent_path("one/two"), "one");
        assert_eq!(parent_path("one/two/three"), "one/two");
    }
}
