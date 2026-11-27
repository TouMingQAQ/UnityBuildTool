use super::{
    card, danger_button, draw_log_panel, hint, pill, primary_button, secondary_button,
    section_title, status_color, status_label, ACCENT, ACCENT_ALT, CARD, CARD_ALT, ERR, MUTED, OK,
    WARN,
};
use crate::app::{AdminApp, ConfirmAction};
use eframe::egui;
use egui::RichText;
use serde_json::Value;

pub fn draw(ui: &mut egui::Ui, app: &mut AdminApp) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    section_title(
                        ui,
                        "版本管理",
                        "Git / SVN 节点分组 · 还原未提交 → 切换分支 → 远端在线才更新",
                    );
                    hint(
                        ui,
                        "拖动手柄可排序或归组；构建节点可绑定这些分组作为构建前步骤。",
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let state = app.vcs_job["state"].as_str().unwrap_or("idle");
                    pill(ui, status_label(state), status_color(state));
                });
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if ui.add(primary_button("添加节点")).clicked() {
                    app.open_vcs_editor(None);
                }
                if ui.add(secondary_button("新建分组")).clicked() {
                    app.add_vcs_group();
                }
                if ui.add(secondary_button("全部更新")).clicked() {
                    let ids = app
                        .vcs_nodes()
                        .iter()
                        .filter_map(|node| node["id"].as_str().map(str::to_owned))
                        .collect();
                    app.start_vcs_update(ids);
                }
                if ui.button("重新探测全部").clicked() {
                    app.probe_vcs_all();
                }
                if ui.add(danger_button("停止")).clicked() {
                    app.stop_vcs();
                }
                if let Some(end) = app.vcs_job.get("end").filter(|end| end.is_object()) {
                    let ok = end["ok"].as_bool() == Some(true);
                    pill(
                        ui,
                        if ok { "更新完成" } else { "更新失败" },
                        if ok { OK } else { ERR },
                    );
                }
            });
            ui.add_space(10.0);

            let groups = app.vcs_groups();
            let ungrouped = app.ungrouped_vcs_nodes();
            vcs_drop_card(ui, app, None, "未分组节点", ungrouped);
            for group in groups {
                let id = group["id"].as_str().unwrap_or("").to_owned();
                let name = group["name"].as_str().unwrap_or("版本分组").to_owned();
                let nodes = group["nodeIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|id| app.vcs_nodes().into_iter().find(|node| node["id"] == *id))
                    .collect::<Vec<_>>();
                vcs_drop_card(ui, app, Some(&id), &name, nodes);
            }
            ui.add_space(8.0);
            let ctx = ui.ctx().clone();
            card(ui, |ui| {
                draw_log_panel(
                    ui,
                    &ctx,
                    &mut app.vcs_log,
                    "版本更新日志",
                    |log| log.clear(),
                    300.0,
                );
            });
        });
}

fn vcs_drop_card(
    ui: &mut egui::Ui,
    app: &mut AdminApp,
    group_id: Option<&str>,
    group_name: &str,
    nodes: Vec<Value>,
) {
    let mut hovered_node: Option<String> = None;
    let frame = egui::Frame::new()
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(31, 41, 55)))
        .corner_radius(9.0)
        .inner_margin(egui::Margin::same(13));
    let (response, payload) = ui.dnd_drop_zone::<String, _>(frame, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(group_name)
                    .strong()
                    .size(16.0)
                    .color(egui::Color32::WHITE),
            );
            pill(ui, &format!("{} 个节点", nodes.len()), ACCENT_ALT);
            if let Some(group_id) = group_id {
                let mut name = group_name.to_owned();
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .desired_width(180.0)
                        .hint_text("分组名称"),
                );
                if edit.changed() {
                    set_vcs_group_name(app, group_id, name.trim());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("删除分组").clicked() {
                        app.confirm = Some(crate::app::ConfirmDialog {
                            title: "删除版本分组".into(),
                            text: format!("删除“{group_name}”？节点将回到未分组。"),
                            action: ConfirmAction::DeleteVcsGroup(group_id.to_owned()),
                        });
                    }
                    if ui.small_button("更新分组").clicked() {
                        let ids = nodes
                            .iter()
                            .filter_map(|node| node["id"].as_str().map(str::to_owned))
                            .collect();
                        app.start_vcs_update(ids);
                    }
                });
            }
        });
        ui.add_space(6.0);
        if nodes.is_empty() {
            let rect = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 46.0), egui::Sense::hover())
                .0;
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "将节点拖到这里归组",
                egui::FontId::proportional(13.0),
                MUTED,
            );
        }
        for node in &nodes {
            let response = vcs_node_card(ui, app, node, group_id);
            if response.contains_pointer() {
                hovered_node = node["id"].as_str().map(str::to_owned);
            }
        }
    });
    if let Some(payload) = payload {
        app.move_vcs_node_before(&payload, group_id, hovered_node.as_deref());
    }
    response.response.on_hover_text("可将其他节点拖入此分组");
    ui.add_space(8.0);
}

fn vcs_node_card(
    ui: &mut egui::Ui,
    app: &mut AdminApp,
    node: &Value,
    group_id: Option<&str>,
) -> egui::Response {
    let id = node["id"].as_str().unwrap_or("").to_owned();
    let state = app.vcs_states.get(&id).cloned().unwrap_or(Value::Null);
    let status = state["status"]
        .as_str()
        .unwrap_or(if state["ok"].as_bool() == Some(false) {
            "fail"
        } else if state["ok"].as_bool() == Some(true) {
            "ok"
        } else {
            "idle"
        });
    let vcs_type = state["type"].as_str().unwrap_or(
        node["type"]
            .as_str()
            .filter(|kind| *kind != "auto")
            .unwrap_or("none"),
    );
    let frame = egui::Frame::new()
        .fill(CARD_ALT)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(30, 41, 59)))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 6));
    let response = frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.dnd_drag_source(
                    egui::Id::unique(("vcs-drag", id.as_str())),
                    id.clone(),
                    |ui| {
                        ui.label(RichText::new("⠿").color(MUTED))
                            .on_hover_text("拖动排序或归组");
                    },
                );
                pill(ui, status_label(status), status_color(status));
                pill(
                    ui,
                    match vcs_type {
                        "git" => "Git",
                        "svn" => "SVN",
                        _ => "未检测",
                    },
                    match vcs_type {
                        "git" => OK,
                        "svn" => ACCENT_ALT,
                        _ => MUTED,
                    },
                );
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(node["name"].as_str().unwrap_or("未命名节点"))
                                .strong()
                                .color(egui::Color32::WHITE),
                        );
                        if !node["branch"].as_str().unwrap_or("").is_empty() {
                            pill(ui, node["branch"].as_str().unwrap_or(""), ACCENT);
                        }
                        if !node["revert"].as_str().unwrap_or("").is_empty() {
                            pill(ui, "自定义还原", WARN);
                        }
                    });
                    ui.label(
                        RichText::new(
                            node["path"]
                                .as_str()
                                .filter(|path| !path.is_empty())
                                .unwrap_or("（路径为空，不执行更新）"),
                        )
                        .small()
                        .color(MUTED),
                    );
                    let mut meta = Vec::new();
                    if let Some(branch) =
                        state["branch"].as_str().filter(|branch| !branch.is_empty())
                    {
                        meta.push(format!("当前分支 {branch}"));
                    }
                    if let Some(revision) = state["revision"]
                        .as_str()
                        .filter(|revision| !revision.is_empty())
                    {
                        meta.push(format!("{revision}"));
                    }
                    if state["hasRemote"].as_bool() == Some(false) {
                        meta.push("无远端".into());
                    } else if state["remoteOnline"].as_bool() == Some(true) {
                        meta.push("远端在线".into());
                    } else if state["hasRemote"].as_bool() == Some(true) {
                        meta.push("远端离线".into());
                    }
                    if state["branchMatched"].as_bool() == Some(false) {
                        meta.push("待切换指定分支".into());
                    }
                    if state["dirty"].as_bool() == Some(true) {
                        meta.push(format!(
                            "{} 处未提交",
                            state["dirtyCount"].as_u64().unwrap_or(0)
                        ));
                    }
                    if let Some(message) = state["message"]
                        .as_str()
                        .filter(|message| !message.is_empty())
                    {
                        meta.push(message.to_owned());
                    }
                    if !meta.is_empty() {
                        ui.label(RichText::new(meta.join(" · ")).small().color(MUTED));
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("删除").clicked() {
                        app.confirm = Some(crate::app::ConfirmDialog {
                            title: "删除版本节点".into(),
                            text: format!(
                                "删除“{}”？该操作不会改动磁盘仓库。",
                                node["name"].as_str().unwrap_or("未命名节点")
                            ),
                            action: ConfirmAction::DeleteVcsNode(id.clone()),
                        });
                    }
                    if ui.small_button("编辑").clicked() {
                        app.open_vcs_editor(Some(&id));
                    }
                    if ui.small_button("刷新").clicked() {
                        app.probe_vcs_ids(vec![id.clone()]);
                    }
                    if !app.busy() && ui.small_button("更新").clicked() {
                        app.start_vcs_update(vec![id.clone()]);
                    }
                });
            });
        })
        .response;
    let _ = group_id;
    response
}

fn set_vcs_group_name(app: &mut AdminApp, id: &str, name: &str) {
    let mut groups = app.vcs_groups();
    if let Some(group) = groups
        .iter_mut()
        .find(|group| group["id"].as_str() == Some(id))
    {
        group["name"] = serde_json::json!(if name.is_empty() {
            "版本分组"
        } else {
            name
        });
    }
    app.set_cfg(&["vcs", "groups"], Value::Array(groups));
}

pub fn draw_vcs_editor(app: &mut AdminApp, ctx: &egui::Context) {
    let mut close = false;
    let mut save = false;
    let mut probe = false;
    if let Some(editor) = app.vcs_editor.as_mut() {
        egui::Window::new(if editor.editing_id.is_some() {
            "编辑版本节点"
        } else {
            "添加版本节点"
        })
        .id(egui::Id::unique("vcs-editor"))
        .collapsible(false)
        .resizable(true)
        .default_width(560.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(RichText::new("基本信息").strong().color(ACCENT));
            ui.horizontal(|ui| {
                ui.label(RichText::new("名称").small().color(MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut editor.name)
                        .hint_text("例如 GameClient / Art 资源库")
                        .desired_width(380.0),
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("路径").small().color(MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut editor.path)
                        .hint_text("绝对路径或相对 runtime root 的路径")
                        .desired_width(300.0),
                );
                if ui.add(secondary_button("检测类型")).clicked() {
                    probe = true;
                }
            });
            if !editor.probe_text.is_empty() {
                ui.label(
                    RichText::new(&editor.probe_text)
                        .small()
                        .color(if editor.probe_ok { OK } else { WARN }),
                );
            }
            ui.separator();
            ui.label(RichText::new("仓库类型与分支").strong().color(ACCENT));
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("类型").small().color(MUTED));
                for (value, label) in [("auto", "自动检测"), ("git", "Git"), ("svn", "SVN")] {
                    if ui
                        .selectable_label(editor.vcs_type == value, label)
                        .clicked()
                    {
                        editor.vcs_type = value.into();
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("目标分支").small().color(MUTED));
                ui.add(
                    egui::TextEdit::singleline(&mut editor.branch)
                        .hint_text("留空则不切换分支（SVN 可填 trunk 或 URL）")
                        .desired_width(320.0),
                );
                if !editor.branches.is_empty() {
                    egui::ComboBox::from_id_salt("vcs-branch-combo")
                        .selected_text("选择已探测分支")
                        .width(140.0)
                        .show_ui(ui, |ui| {
                            let branches = editor.branches.clone();
                            for branch in branches {
                                if ui.selectable_label(false, &branch).clicked() {
                                    editor.branch = branch;
                                }
                            }
                        });
                }
            });
            ui.separator();
            ui.label(RichText::new("更新前还原命令").strong().color(ACCENT));
            ui.add(
                egui::TextEdit::singleline(&mut editor.revert)
                    .hint_text("留空 = 默认普通还原")
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal_wrapped(|ui| {
                for (label, command, help) in [
                    (
                        "只还原未暂存",
                        "git checkout -- .",
                        "保留已暂存与未跟踪文件",
                    ),
                    (
                        "普通还原",
                        "git reset --hard",
                        "还原已暂存和未暂存，保留未跟踪",
                    ),
                    (
                        "还原并清理",
                        "git reset --hard && git clean -fd",
                        "同时删除未跟踪文件与目录",
                    ),
                    ("仅清理未跟踪", "git clean -fd", "不动已提交与已修改内容"),
                    ("清空（默认）", "", "恢复默认普通还原逻辑"),
                ] {
                    if ui.small_button(label).on_hover_text(help).clicked() {
                        editor.revert = command.into();
                    }
                }
            });
            if !editor.error.is_empty() {
                ui.label(RichText::new(&editor.error).small().color(ERR));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(primary_button("保存节点")).clicked() {
                    save = true;
                }
                if ui.button("取消").clicked() {
                    close = true;
                }
            });
        });
    }
    if probe {
        app.probe_vcs_editor();
    }
    if save {
        app.save_vcs_editor();
    }
    if close {
        app.vcs_editor = None;
    }
}
