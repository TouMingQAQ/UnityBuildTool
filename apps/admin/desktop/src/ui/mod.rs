use crate::app::{
    AdminApp, Page, ToastKind, ACCENT, ACCENT_ALT, BG, BG_PANEL, CARD, CARD_ALT, ERR, MUTED, OK,
    TEXT, WARN,
};
use eframe::egui;
use egui::{Align, Layout, RichText};
use std::time::Duration;

pub mod build;
pub mod check;
pub mod help;
pub mod monitor;
pub mod profiles;
pub mod settings;
pub mod vcs;

pub fn draw_shell(app: &mut AdminApp, root: &mut egui::Ui) {
    egui::Panel::top("admin-top")
        .exact_size(64.0)
        .frame(
            egui::Frame::new()
                .fill(BG_PANEL)
                .stroke(egui::Stroke::NONE)
                .inner_margin(egui::Margin::symmetric(18, 10)),
        )
        .show(root, |ui| {
            let panel_width = ui.available_width();
            let compact = panel_width < 860.0;
            let narrow = panel_width < 700.0;
            ui.horizontal(|ui| {
                if let Some(icon) = app.brand_icon.as_ref() {
                    ui.add(
                        egui::Image::from_texture(icon).fit_to_exact_size(egui::vec2(
                            if narrow { 32.0 } else { 38.0 },
                            if narrow { 32.0 } else { 38.0 },
                        )),
                    );
                } else {
                    ui.label(RichText::new("T").strong().size(18.0).color(ACCENT));
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new("UnityBuildTool").strong().size(15.0));
                    if !compact {
                        ui.label(RichText::new("管理端").small().color(MUTED));
                    }
                });
                ui.add_space(if compact { 8.0 } else { 22.0 });
                let projects = app.projects.clone();
                let selected_name = projects
                    .iter()
                    .find(|p| p["id"].as_str() == Some(app.selected_project_id.as_str()))
                    .and_then(|p| p["name"].as_str())
                    .unwrap_or("选择项目");
                let mut switch_to = None;
                egui::ComboBox::from_id_salt("top-project-select")
                    .selected_text(selected_name)
                    .width(if narrow {
                        128.0
                    } else if compact {
                        168.0
                    } else {
                        220.0
                    })
                    .show_ui(ui, |ui| {
                        for project in &projects {
                            let id = project["id"].as_str().unwrap_or("");
                            let label = format!(
                                "{} · /p/{}",
                                project["name"].as_str().unwrap_or("未命名"),
                                project["slug"].as_str().unwrap_or("")
                            );
                            if ui
                                .selectable_label(app.selected_project_id == id, label)
                                .clicked()
                            {
                                switch_to = Some(id.to_owned());
                            }
                        }
                        if projects.is_empty() {
                            ui.label(RichText::new("尚无项目，请到设置中创建").color(MUTED));
                        }
                    });
                if let Some(id) = switch_to {
                    app.select_project(&id);
                }
                ui.add_space(8.0);
                if ui
                    .add(secondary_button(if narrow {
                        "网页"
                    } else {
                        "用户网页"
                    }))
                    .on_hover_text("在默认浏览器打开当前项目的用户 Web")
                    .clicked()
                {
                    app.open_user_website();
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    save_indicator(app, ui);
                    ui.separator();
                    let address = format!("{} · {}", app.base_url, task_label(app));
                    if panel_width >= 930.0 {
                        let mut address_copy = app.base_url.clone();
                        ui.add(
                            egui::TextEdit::singleline(&mut address_copy)
                                .desired_width(260.0)
                                .interactive(false),
                        )
                        .on_hover_text(address);
                    } else {
                        let short = app
                            .base_url
                            .trim_start_matches("http://")
                            .trim_start_matches("https://");
                        ui.add_sized(
                            [if narrow { 106.0 } else { 150.0 }, 30.0],
                            egui::Label::new(RichText::new(short).small().color(MUTED).monospace())
                                .truncate(),
                        )
                        .on_hover_text(address);
                    }
                    status_dot(
                        ui,
                        if app.busy() { WARN } else { OK },
                        if app.busy() { "忙碌" } else { "在线" },
                    );
                });
            });
        });

    egui::Panel::bottom("admin-status")
        .exact_size(32.0)
        .frame(
            egui::Frame::new()
                .fill(BG_PANEL)
                .stroke(egui::Stroke::NONE)
                .inner_margin(egui::Margin::symmetric(14, 5)),
        )
        .show(root, |ui| {
            ui.horizontal(|ui| {
                status_dot(
                    ui,
                    if app.busy() { WARN } else { OK },
                    if app.busy() {
                        "任务执行中"
                    } else {
                        "服务在线"
                    },
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let text = if app.save_error.is_some() {
                        "配置保存失败"
                    } else if app.config_loaded && app.config != app.saved_config {
                        "配置有未保存修改"
                    } else {
                        "配置已同步"
                    };
                    ui.label(
                        RichText::new(text)
                            .small()
                            .color(if app.save_error.is_some() {
                                ERR
                            } else if app.config_loaded && app.config != app.saved_config {
                                WARN
                            } else {
                                MUTED
                            }),
                    );
                    ui.label(
                        RichText::new(format!("{} 个后台请求", app.pending.len()))
                            .small()
                            .color(MUTED),
                    );
                });
            });
        });

    egui::Panel::left("admin-nav")
        .exact_size(196.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(BG_PANEL)
                .stroke(egui::Stroke::NONE)
                .inner_margin(egui::Margin::symmetric(14, 14)),
        )
        .show(root, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for page in Page::ALL {
                if nav_item(ui, page, app.page == page).clicked() {
                    app.page = page;
                }
            }
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);
            let count = app.selected_build_nodes().len();
            ui.label(RichText::new("当前构建选择").small().color(MUTED));
            ui.label(
                RichText::new(format!("{count} 个节点"))
                    .strong()
                    .color(if count == 0 { WARN } else { ACCENT }),
            );
            if app.busy() {
                ui.add_space(8.0);
                ui.label(RichText::new(task_label(app)).color(WARN).small());
            }
            ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                if nav_quit_button(ui).clicked() {
                    app.quit(ui.ctx());
                }
            });
        });

    let now = root.ctx().input(|input| input.time);
    if app.page_enter_page != app.page {
        app.page_enter_page = app.page;
        app.page_enter_time = now;
    }
    let page_age = (now - app.page_enter_time) as f32;
    let page_reveal = (page_age / 0.16).clamp(0.0, 1.0);
    let page_reveal = 1.0 - (1.0 - page_reveal).powi(3);
    if page_age < 0.16 {
        root.ctx().request_repaint();
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(BG)
                .stroke(egui::Stroke::NONE)
                .inner_margin(egui::Margin::symmetric(20, 16)),
        )
        .show(root, |ui| {
            ui.set_opacity(page_reveal);
            ui.add_space((1.0 - page_reveal) * 6.0);
            if app.root_loading && !app.root_loaded {
                centered_loading(ui, "正在读取管理配置…");
                return;
            }
            if app.projects.is_empty() {
                empty_projects(ui, app);
                return;
            }
            if app.project_loading && !app.config_loaded {
                centered_loading(ui, "正在读取项目配置…");
                return;
            }
            match app.page {
                Page::Build => build::draw(ui, app),
                Page::Monitor => monitor::draw(ui, app),
                Page::Profiles => profiles::draw(ui, app),
                Page::Check => check::draw(ui, app),
                Page::Vcs => vcs::draw(ui, app),
                Page::Settings => settings::draw(ui, app),
                Page::Help => help::draw(ui, app),
            }
        });
}

fn nav_item(ui: &mut egui::Ui, page: Page, active: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
    let hovered = response.hovered();
    let active_t = ui.ctx().animate_bool_with_time(
        egui::Id::unique(("nav-active", page.label())),
        active,
        0.18,
    );
    let hover_t = ui.ctx().animate_bool_with_time(
        egui::Id::unique(("nav-hover", page.label())),
        hovered,
        0.12,
    );
    let hover_fill =
        egui::Color32::from_rgba_unmultiplied(56, 189, 248, (16.0 * hover_t).round() as u8);
    let active_fill =
        egui::Color32::from_rgba_unmultiplied(56, 189, 248, (52.0 * active_t).round() as u8);
    let fill = hover_fill.blend(active_fill);
    if fill != egui::Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 8.0, fill);
    }
    if active_t > 0.001 {
        let indicator = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 4.0, rect.center().y - 9.0 * active_t),
            egui::pos2(rect.left() + 6.0, rect.center().y + 9.0 * active_t),
        );
        ui.painter().rect_filled(
            indicator,
            1.0,
            egui::Color32::from_rgba_unmultiplied(
                ACCENT.r(),
                ACCENT.g(),
                ACCENT.b(),
                (255.0 * active_t).round() as u8,
            ),
        );
    }
    let icon_color = MUTED
        .lerp_to_gamma(TEXT, hover_t)
        .lerp_to_gamma(ACCENT, active_t);
    let label_color = MUTED
        .lerp_to_gamma(TEXT, hover_t)
        .lerp_to_gamma(egui::Color32::WHITE, active_t);
    paint_nav_icon(
        ui.painter(),
        page,
        egui::pos2(rect.left() + 22.0, rect.center().y),
        icon_color,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 44.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        page.label(),
        egui::FontId::proportional(14.5),
        label_color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), active, page.label())
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint_nav_icon(painter: &egui::Painter, page: Page, center: egui::Pos2, color: egui::Color32) {
    let bounds = egui::Rect::from_center_size(center, egui::vec2(16.0, 16.0));
    let p = |x: f32, y: f32| bounds.left_top() + egui::vec2(x, y);
    let stroke = egui::Stroke::new(1.6, color);
    match page {
        Page::Build => {
            painter.rect_stroke(
                egui::Rect::from_min_max(p(2.0, 2.4), p(14.0, 13.6)),
                3.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.add(egui::Shape::closed_line(
                vec![p(6.3, 5.3), p(11.2, 8.0), p(6.3, 10.7)],
                stroke,
            ));
        }
        Page::Monitor => {
            painter.add(egui::Shape::line(
                vec![
                    p(1.7, 8.6),
                    p(5.2, 8.6),
                    p(7.0, 4.7),
                    p(9.9, 12.0),
                    p(11.4, 8.6),
                    p(14.3, 8.6),
                ],
                stroke,
            ));
        }
        Page::Profiles => {
            painter.add(egui::Shape::closed_line(
                vec![p(8.0, 1.9), p(13.8, 5.0), p(8.0, 8.1), p(2.2, 5.0)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(2.2, 8.0), p(8.0, 11.1), p(13.8, 8.0)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(2.2, 11.0), p(8.0, 14.1), p(13.8, 11.0)],
                stroke,
            ));
        }
        Page::Check => {
            painter.circle_stroke(center, 6.1, stroke);
            painter.add(egui::Shape::line(
                vec![p(4.9, 8.1), p(7.2, 10.2), p(11.1, 5.8)],
                stroke,
            ));
        }
        Page::Vcs => {
            painter.circle_stroke(p(4.8, 3.4), 1.5, stroke);
            painter.circle_stroke(p(4.8, 12.6), 1.5, stroke);
            painter.circle_stroke(p(11.2, 6.7), 1.5, stroke);
            painter.line_segment([p(4.8, 4.9), p(4.8, 11.1)], stroke);
            painter.add(egui::Shape::line(
                vec![p(4.8, 7.9), p(8.2, 7.9), p(9.7, 6.7)],
                stroke,
            ));
        }
        Page::Settings => {
            for (y, knob_x) in [(4.2, 10.3), (8.0, 5.8), (11.8, 11.0)] {
                painter.line_segment([p(1.8, y), p(knob_x - 2.0, y)], stroke);
                painter.circle_stroke(p(knob_x, y), 1.8, stroke);
                painter.line_segment([p(knob_x + 2.0, y), p(14.2, y)], stroke);
            }
        }
        Page::Help => {
            painter.circle_stroke(center, 6.2, stroke);
            painter.add(egui::Shape::line(
                vec![
                    p(6.0, 5.8),
                    p(6.5, 4.7),
                    p(8.0, 4.3),
                    p(9.5, 5.2),
                    p(9.4, 6.6),
                    p(8.0, 7.5),
                    p(8.0, 8.9),
                ],
                stroke,
            ));
            painter.circle_filled(p(8.0, 11.2), 0.9, color);
        }
    }
}

fn nav_quit_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::click());
    let fill = if response.hovered() {
        ERR.gamma_multiply(0.12)
    } else {
        CARD
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    ui.painter().text(
        egui::pos2(rect.left() + 14.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "退出管理端",
        egui::FontId::proportional(14.0),
        ERR,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "退出管理端")
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn save_indicator(app: &mut AdminApp, ui: &mut egui::Ui) {
    if let Some(error) = app.save_error.clone() {
        ui.horizontal(|ui| {
            ui.label(RichText::new("保存失败").small().color(ERR))
                .on_hover_text(error);
            if ui.small_button("重试").clicked() {
                app.retry_save();
            }
        });
    } else if app.saving {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new("保存中").small().color(WARN));
        });
    } else if app.config_loaded && app.config != app.saved_config {
        ui.label(RichText::new("待保存").small().color(WARN));
    } else {
        ui.label(RichText::new("已保存").small().color(MUTED));
    }
}

fn status_dot(ui: &mut egui::Ui, color: egui::Color32, text: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    ui.label(RichText::new(text).small().color(color));
}

pub fn task_label(app: &AdminApp) -> String {
    match app.job["state"].as_str() {
        Some("queued") => return "构建排队中".into(),
        Some("running" | "starting") => {
            let index = app.job["index"].as_u64().unwrap_or(0) as usize + 1;
            return format!("构建中 · 第 {index} 项");
        }
        _ => {}
    }
    match app.check_job["state"].as_str() {
        Some("queued") => return "环境检测排队中".into(),
        Some("running" | "starting") => return "环境检测中".into(),
        _ => {}
    }
    match app.vcs_job["state"].as_str() {
        Some("queued") => return "版本更新排队中".into(),
        Some("running" | "starting") => return "版本更新中".into(),
        _ => {}
    }
    "空闲".into()
}

fn centered_loading(ui: &mut egui::Ui, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.32);
        ui.spinner();
        ui.add_space(8.0);
        ui.label(RichText::new(text).color(MUTED));
    });
}

fn empty_projects(ui: &mut egui::Ui, app: &mut AdminApp) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.25);
        ui.label(RichText::new("尚无项目配置").size(24.0).strong());
        ui.label(
            RichText::new("创建第一个 Unity 项目后，即可扫描 Build Profile 并开始构建。")
                .color(MUTED),
        );
        ui.add_space(12.0);
        if ui.add(primary_button("创建项目")).clicked() {
            app.add_project();
        }
    });
}

pub fn primary_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).color(egui::Color32::WHITE))
        .fill(ACCENT.gamma_multiply(0.78))
        .stroke(egui::Stroke::NONE)
        .corner_radius(7.0)
}

pub fn secondary_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).color(ACCENT))
        .fill(ACCENT.gamma_multiply(0.11))
        .stroke(egui::Stroke::NONE)
        .corner_radius(7.0)
}

pub fn danger_button(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.into()).color(ERR))
        .fill(ERR.gamma_multiply(0.10))
        .stroke(egui::Stroke::NONE)
        .corner_radius(7.0)
}

pub fn card<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(10.0)
        .inner_margin(egui::Margin::same(14))
        .show(ui, add_contents)
}

pub fn section_title(ui: &mut egui::Ui, title: &str, hint: impl Into<String>) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 22.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 1.5, ACCENT);
        ui.label(
            RichText::new(title)
                .strong()
                .size(19.0)
                .color(egui::Color32::WHITE),
        );
        ui.add_space(3.0);
        ui.label(RichText::new(hint.into()).small().color(MUTED));
    });
}

pub fn hint(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).small().color(MUTED));
}

pub fn error_text(ui: &mut egui::Ui, text: &str) {
    if !text.is_empty() {
        ui.label(RichText::new(text).small().color(ERR));
    }
}

pub fn pill(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.14))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().strong().color(color));
        });
}

pub fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str, enabled: bool) -> bool {
    ui.add_enabled_ui(enabled, |ui| ui.checkbox(value, label))
        .inner
        .changed()
}

pub fn draw_log_panel(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    log: &mut crate::app::LogPanel,
    title: &str,
    clear: impl FnOnce(&mut crate::app::LogPanel),
    max_height: f32,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong());
        ui.add(
            egui::TextEdit::singleline(&mut log.search)
                .hint_text("搜索日志")
                .desired_width(190.0),
        );
        if ui.button("复制").clicked() {
            ctx.copy_text(log.rendered());
        }
        if ui.button("清空").clicked() {
            clear(log);
        }
        ui.checkbox(&mut log.follow, "自动滚动");
        ui.label(
            RichText::new(format!("{} 行", log.lines.len()))
                .small()
                .color(MUTED),
        );
    });
    let body = log.rendered();
    egui::Frame::new()
        .fill(BG)
        .corner_radius(7.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .stick_to_bottom(log.follow)
                .max_height(max_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(if body.is_empty() {
                                "（暂无日志）"
                            } else {
                                &body
                            })
                            .monospace()
                            .color(egui::Color32::from_rgb(203, 213, 225)),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend)
                        .selectable(true),
                    );
                });
        });
}

pub fn draw_overlays(app: &mut AdminApp, ctx: &egui::Context) {
    draw_toasts(app, ctx);

    if let Some(confirm) = app.confirm.as_ref() {
        let mut close = false;
        let mut accept = false;
        egui::Window::new(confirm.title.clone())
            .id(egui::Id::unique("confirm-dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.label(&confirm.text);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        close = true;
                    }
                    if ui.add(danger_button("确认")).clicked() {
                        accept = true;
                    }
                });
            });
        if accept {
            if let Some(confirm) = app.confirm.take() {
                app.confirm_action(confirm.action);
            }
        } else if close {
            app.confirm = None;
        }
    }

    if app.preview.is_some() {
        let mut close = false;
        let title = app
            .preview
            .as_ref()
            .map(|dialog| dialog.title.clone())
            .unwrap_or_default();
        egui::Window::new(title)
            .id(egui::Id::unique("preview-dialog"))
            .collapsible(false)
            .resizable(true)
            .default_width(760.0)
            .default_height(480.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if let Some(dialog) = &app.preview {
                        for item in &dialog.items {
                            card(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(item["name"].as_str().unwrap_or(""))
                                            .strong()
                                            .color(ACCENT),
                                    );
                                    if let Some(target) = item["target"].as_str() {
                                        pill(ui, target, ACCENT_ALT);
                                    }
                                    if let Some(target) = item["target"].as_i64() {
                                        pill(ui, &format!("BuildTarget {target}"), ACCENT_ALT);
                                    }
                                });
                                if let Some(preview) = item["previewName"].as_str() {
                                    hint(ui, format!("预计产物：{preview}"));
                                }
                                if let Some(output) = item["output"].as_str() {
                                    hint(ui, format!("输出：{output}"));
                                }
                                if let Some(log) = item["logFile"].as_str() {
                                    hint(ui, format!("日志：{log}"));
                                }
                                let mut command = item["command"].as_str().unwrap_or("").to_owned();
                                ui.add(
                                    egui::TextEdit::multiline(&mut command)
                                        .font(egui::TextStyle::Monospace)
                                        .desired_rows(2)
                                        .desired_width(f32::INFINITY),
                                );
                            });
                            ui.add_space(6.0);
                        }
                    }
                });
                if ui.button("关闭").clicked() {
                    close = true;
                }
            });
        if close {
            app.preview = None;
        }
    }

    if app.ai.is_some() {
        let mut close = false;
        let loading = app.ai.as_ref().map(|ai| ai.loading).unwrap_or(false);
        egui::Window::new("AI 构建分析")
            .id(egui::Id::unique("ai-dialog"))
            .collapsible(false)
            .resizable(true)
            .default_width(720.0)
            .default_height(420.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("正在请求 AI 分析…");
                    });
                }
                if let Some(ai) = &mut app.ai {
                    egui::ScrollArea::vertical()
                        .max_height(330.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&ai.text)
                                        .color(egui::Color32::from_rgb(203, 213, 225)),
                                )
                                .wrap(),
                            );
                        });
                    if ui.button("关闭").clicked() {
                        close = true;
                    }
                }
            });
        if close {
            app.ai = None;
        }
    }

    if app.build_editor.is_some() {
        profiles::draw_build_editor(app, ctx);
    }
    if app.vcs_editor.is_some() {
        vcs::draw_vcs_editor(app, ctx);
    }

    if app.server_restart_note {
        egui::Window::new("监听地址已修改")
            .id(egui::Id::unique("server-restart-note"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("新的监听地址已保存。请重启管理端，使内置用户 Web 服务绑定到新地址。");
                if ui.button("知道了").clicked() {
                    app.server_restart_note = false;
                }
            });
    }
}

fn draw_toasts(app: &mut AdminApp, ctx: &egui::Context) {
    app.toasts
        .retain(|toast| toast.created.elapsed() < Duration::from_secs(5));
    if app.toasts.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::unique("toast-area"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -42.0])
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            ui.set_max_width(480.0);
            for toast in app.toasts.iter().rev() {
                let (color, icon) = match toast.kind {
                    ToastKind::Info => (ACCENT, "i"),
                    ToastKind::Ok => (OK, "✓"),
                    ToastKind::Error => (ERR, "!"),
                };
                let age = toast.created.elapsed().as_secs_f32();
                let enter = (age / 0.18).clamp(0.0, 1.0);
                let eased = 1.0 - (1.0 - enter).powi(3);
                let exit = ((5.0 - age) / 0.30).clamp(0.0, 1.0);
                let alpha = eased.min(exit);
                let slide = (1.0 - eased) * 14.0;
                ui.scope(|ui| {
                    ui.set_opacity(alpha);
                    ui.horizontal(|ui| {
                        egui::Frame::new()
                            .fill(CARD)
                            .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.65)))
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .shadow(egui::epaint::Shadow {
                                offset: [0, 4],
                                blur: 16,
                                spread: 0,
                                color: egui::Color32::from_black_alpha(120),
                            })
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(icon).strong().color(color));
                                    ui.label(
                                        RichText::new(&toast.text).color(egui::Color32::WHITE),
                                    );
                                });
                            });
                        ui.add_space(slide);
                    });
                });
                ui.add_space(6.0);
            }
        });
}

fn self_close(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
}

pub fn small_stat(ui: &mut egui::Ui, label: &str, value: impl Into<String>, color: egui::Color32) {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).small().color(MUTED));
        ui.label(RichText::new(value.into()).size(18.0).strong().color(color));
    });
}

pub fn compatibility_note(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(CARD_ALT)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(MUTED));
        });
}

pub fn cfg_text(
    ui: &mut egui::Ui,
    app: &mut AdminApp,
    keys: &[&str],
    hint: &str,
    width: f32,
) -> bool {
    let mut text = app.cfg_str(keys);
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .hint_text(hint)
            .desired_width(width),
    );
    if response.changed() {
        app.set_cfg(keys, serde_json::json!(text));
        true
    } else {
        false
    }
}

pub fn cfg_textarea(
    ui: &mut egui::Ui,
    app: &mut AdminApp,
    keys: &[&str],
    hint: &str,
    rows: usize,
) -> bool {
    let mut text = app.cfg_str(keys);
    let response = ui.add(
        egui::TextEdit::multiline(&mut text)
            .hint_text(hint)
            .desired_rows(rows)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace),
    );
    if response.changed() {
        app.set_cfg(keys, serde_json::json!(text));
        true
    } else {
        false
    }
}

pub fn cfg_checkbox(
    ui: &mut egui::Ui,
    app: &mut AdminApp,
    keys: &[&str],
    label: &str,
    default: bool,
) -> bool {
    let mut value = app.cfg_bool(keys, default);
    let changed = ui.checkbox(&mut value, label).changed();
    if changed {
        app.set_cfg(keys, serde_json::json!(value));
    }
    changed
}

pub fn cfg_i64(
    ui: &mut egui::Ui,
    app: &mut AdminApp,
    keys: &[&str],
    label: &str,
    default: i64,
    width: f32,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).small().color(MUTED));
        let mut text = app.cfg_i64(keys, default).to_string();
        let response = ui.add(
            egui::TextEdit::singleline(&mut text)
                .desired_width(width)
                .char_limit(12),
        );
        if response.changed() {
            if let Ok(value) = text.trim().parse::<i64>() {
                app.set_cfg(keys, serde_json::json!(value));
            }
        }
    });
    false
}

pub fn cfg_combo(
    ui: &mut egui::Ui,
    app: &mut AdminApp,
    keys: &[&str],
    id: &str,
    options: &[(&str, &str)],
    default: &str,
    width: f32,
) -> bool {
    let current = app.cfg_str(keys);
    let current = if current.is_empty() {
        default.to_owned()
    } else {
        current
    };
    let selected = options
        .iter()
        .find(|(value, _)| *value == current)
        .map(|(_, label)| *label)
        .unwrap_or(current.as_str());
    let mut chosen = None;
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected)
        .width(width)
        .show_ui(ui, |ui| {
            for (value, label) in options {
                if ui.selectable_label(current == *value, *label).clicked() {
                    chosen = Some((*value).to_owned());
                }
            }
        });
    if let Some(chosen) = chosen {
        app.set_cfg(keys, serde_json::json!(chosen));
        true
    } else {
        false
    }
}

pub fn show_password(ui: &mut egui::Ui, id: &str) -> bool {
    let id = egui::Id::unique(("password-visible", id));
    let mut visible = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(id))
        .unwrap_or(false);
    let response = ui
        .small_button(if visible { "隐藏" } else { "显示" })
        .on_hover_text("显示或隐藏密码");
    if response.clicked() {
        visible = !visible;
        ui.ctx().data_mut(|data| data.insert_temp(id, visible));
    }
    visible
}

pub fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(MUTED));
}

pub fn value_label(ui: &mut egui::Ui, text: impl Into<String>, mono: bool) {
    let mut text = RichText::new(text.into()).color(egui::Color32::from_rgb(203, 213, 225));
    if mono {
        text = text.monospace();
    }
    ui.add(egui::Label::new(text).wrap());
}

pub fn status_color(status: &str) -> egui::Color32 {
    match status {
        "ok" | "done" | "success" => OK,
        "running" | "starting" | "queued" | "updating" | "probing" => ACCENT,
        "partial" | "unsupported" | "skipped" | "cancelled" => WARN,
        "fail" | "error" | "timeout" => ERR,
        _ => MUTED,
    }
}

pub fn status_label(status: &str) -> &'static str {
    match status {
        "idle" => "空闲",
        "queued" => "排队中",
        "starting" => "启动中",
        "running" => "执行中",
        "updating" => "更新中",
        "probing" => "探测中",
        "ok" | "done" | "success" => "完成",
        "partial" => "仅还原",
        "skipped" => "跳过",
        "unsupported" => "不支持",
        "cancelled" => "已取消",
        "timeout" => "超时",
        "fail" | "error" => "失败",
        _ => "待执行",
    }
}
