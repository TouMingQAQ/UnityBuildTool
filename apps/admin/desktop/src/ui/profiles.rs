use super::{
    card, cfg_text, hint, pill, primary_button, secondary_button, section_title, show_password,
    ACCENT, ACCENT_ALT, CARD, CARD_ALT, ERR, MUTED, OK, WARN,
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
                        "Profile 与节点",
                        "扫描 Build Profile，配置平台、归档、签名与构建前版本步骤",
                    );
                    hint(ui, "节点可拖拽归组和排序；修改会自动保存。");
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if app.scan_summary.is_empty() {
                        pill(ui, "未扫描", MUTED);
                    } else {
                        pill(ui, &format!("{} 个 Profile", app.profiles.len()), ACCENT);
                    }
                });
            });
            ui.add_space(10.0);
            project_scan(ui, app);
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui.add(primary_button("添加打包节点")).clicked() {
                    app.open_build_editor(None, None);
                }
                if ui.add(secondary_button("新建构建组")).clicked() {
                    app.add_build_group();
                }
            });
            ui.add_space(6.0);
            let ungrouped = app.ungrouped_build_nodes();
            build_drop_card(ui, app, None, "未分组节点", ungrouped);
            let groups = app.build_groups();
            for group in groups {
                let id = group["id"].as_str().unwrap_or("").to_owned();
                let name = group["name"].as_str().unwrap_or("构建组").to_owned();
                let nodes = group["nodeIds"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|id| app.build_nodes().into_iter().find(|node| node["id"] == *id))
                    .collect::<Vec<_>>();
                build_drop_card(ui, app, Some(&id), &name, nodes);
            }
            if app.build_nodes().is_empty() {
                card(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(20.0);
                        ui.label(RichText::new("尚无打包节点").size(20.0).strong());
                        hint(ui, "先填写工程路径并扫描 Profile，然后添加节点。");
                        if ui.add(primary_button("扫描 Profile")).clicked() {
                            app.start_scan();
                        }
                        ui.add_space(20.0);
                    });
                });
            }
        });
}

fn project_scan(ui: &mut egui::Ui, app: &mut AdminApp) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("工程与扫描").strong().size(16.0));
            if app.project_loading {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(secondary_button("扫描 Profile")).clicked() {
                    app.start_scan();
                }
                if ui.button("CLI 打开项目").clicked() {
                    app.request_cli_open();
                }
                if ui.button("探测 Unity").clicked() {
                    app.request_unity_detect();
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Unity 工程").small().color(MUTED));
            cfg_text(
                ui,
                app,
                &["projectPath"],
                "例如 D:\\UnityProject\\Game 或 ..\\..",
                500.0,
            );
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Profile 目录").small().color(MUTED));
            cfg_text(
                ui,
                app,
                &["profileDir"],
                "留空使用 Assets/Settings/Build Profiles",
                420.0,
            );
            hint(ui, "附加目录会与默认目录一同递归扫描 .asset");
        });
        if !app.scan_summary.is_empty() {
            ui.add_space(5.0);
            egui::Frame::new()
                .fill(CARD_ALT)
                .corner_radius(6.0)
                .inner_margin(egui::Margin::same(9))
                .show(ui, |ui| {
                    ui.label(RichText::new(&app.scan_summary).small().color(MUTED));
                    if !app.ignored_profiles.is_empty() {
                        ui.label(
                            RichText::new(format!(
                                "已忽略不支持目标的 Profile：{}",
                                app.ignored_profiles
                                    .iter()
                                    .map(|profile| format!(
                                        "{} ({})",
                                        profile["name"].as_str().unwrap_or("未命名"),
                                        profile["targetName"].as_str().unwrap_or("未知")
                                    ))
                                    .collect::<Vec<_>>()
                                    .join("、")
                            ))
                            .small()
                            .color(WARN),
                        );
                    }
                });
        }
        if !app.profiles.is_empty() {
            ui.add_space(6.0);
            egui::CollapsingHeader::new(format!("已扫描 Profile（{}）", app.profiles.len()))
                .default_open(false)
                .show(ui, |ui| {
                    let profiles = app.profiles.clone();
                    egui::Grid::new("profile-scan-table")
                        .num_columns(4)
                        .striped(true)
                        .spacing([14.0, 6.0])
                        .show(ui, |ui| {
                            for title in ["名称", "平台", "场景", "路径"] {
                                ui.label(RichText::new(title).small().color(MUTED));
                            }
                            ui.end_row();
                            for profile in profiles {
                                ui.label(
                                    RichText::new(profile["name"].as_str().unwrap_or("未命名"))
                                        .strong(),
                                );
                                ui.label(
                                    RichText::new(profile["targetName"].as_str().unwrap_or(""))
                                        .small()
                                        .color(MUTED),
                                );
                                ui.label(
                                    RichText::new(
                                        profile["sceneCount"].as_i64().unwrap_or(0).to_string(),
                                    )
                                    .small()
                                    .color(MUTED),
                                );
                                ui.label(
                                    RichText::new(profile["assetPath"].as_str().unwrap_or(""))
                                        .small()
                                        .color(MUTED),
                                );
                                ui.end_row();
                            }
                        });
                });
        }
    });
}

fn build_drop_card(
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
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut name)
                            .desired_width(170.0)
                            .hint_text("分组名称"),
                    )
                    .changed()
                {
                    set_build_group_name(app, group_id, name.trim());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("删除分组").clicked() {
                        app.confirm = Some(crate::app::ConfirmDialog {
                            title: "删除构建组".into(),
                            text: format!("删除“{group_name}”？组内节点会保留并移到未分组。"),
                            action: ConfirmAction::DeleteBuildGroup(group_id.to_owned()),
                        });
                    }
                    if ui.small_button("添加节点").clicked() {
                        app.open_build_editor(None, Some(group_id));
                    }
                });
            }
        });
        ui.add_space(6.0);
        if nodes.is_empty() {
            let rect = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 48.0), egui::Sense::hover())
                .0;
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "空构建组 · 拖入节点或点击添加节点",
                egui::FontId::proportional(13.0),
                MUTED,
            );
        }
        for node in &nodes {
            let response = build_node_card(ui, app, node);
            if response.contains_pointer() {
                hovered_node = node["id"].as_str().map(str::to_owned);
            }
        }
    });
    if let Some(payload) = payload {
        app.move_build_node_before(&payload, group_id, hovered_node.as_deref());
    }
    response.response.on_hover_text("可将其他节点拖入此分组");
    ui.add_space(8.0);
}

fn build_node_card(ui: &mut egui::Ui, app: &mut AdminApp, node: &Value) -> egui::Response {
    let id = node["id"].as_str().unwrap_or("").to_owned();
    let target = node["target"].as_i64().unwrap_or(19);
    let android_capable = target == 13 || target == 0;
    let mut tags = Vec::new();
    if node["dev"].as_bool().unwrap_or(false) {
        tags.push(("DEV", WARN));
    }
    if node["buildAddressables"].as_bool().unwrap_or(false) {
        tags.push(("Addr", ACCENT_ALT));
    }
    if android_capable {
        match node["androidBuildKind"].as_str().unwrap_or("apk") {
            "aab" => tags.push(("AAB", OK)),
            "gradleProject" => tags.push(("AS 工程", ACCENT_ALT)),
            _ => {}
        }
    }
    if node["autoZip"].as_bool() == Some(false) {
        tags.push(("不压缩", WARN));
    }
    if !node["customBuildDir"].as_str().unwrap_or("").is_empty() {
        tags.push(("共享缓存", WARN));
    }
    if android_capable
        && (!node["keystoreName"].as_str().unwrap_or("").is_empty()
            || !node["keystorePass"].as_str().unwrap_or("").is_empty())
    {
        tags.push(("签名", OK));
    }
    if node["vcsGroupIds"]
        .as_array()
        .is_some_and(|groups| !groups.is_empty())
    {
        tags.push(("构建前 VCS", ACCENT));
    }
    let frame = egui::Frame::new()
        .fill(CARD_ALT)
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(30, 41, 59)))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 6));
    frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.dnd_drag_source(
                    egui::Id::unique(("build-drag", id.as_str())),
                    id.clone(),
                    |ui| {
                        ui.label(RichText::new("⠿").color(MUTED))
                            .on_hover_text("拖动排序或归组");
                    },
                );
                pill(
                    ui,
                    node["targetName"].as_str().unwrap_or("未指定"),
                    if android_capable { OK } else { ACCENT_ALT },
                );
                let mut selected = app.build_selection.contains(&id);
                if ui.checkbox(&mut selected, "").changed() {
                    if selected {
                        app.build_selection.insert(id.clone());
                    } else {
                        app.build_selection.remove(&id);
                    }
                }
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(node["name"].as_str().unwrap_or("未命名节点"))
                            .strong()
                            .color(egui::Color32::WHITE),
                    );
                    ui.label(
                        RichText::new(
                            node["profileAssetPath"]
                                .as_str()
                                .unwrap_or("项目默认（跟随 Build Settings）"),
                        )
                        .small()
                        .color(MUTED),
                    );
                    ui.horizontal_wrapped(|ui| {
                        for (label, color) in &tags {
                            pill(ui, label, *color);
                        }
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("删除").clicked() {
                        app.confirm = Some(crate::app::ConfirmDialog {
                            title: "删除打包节点".into(),
                            text: format!(
                                "删除“{}”？工程文件和既有产物不会删除。",
                                node["name"].as_str().unwrap_or("未命名节点")
                            ),
                            action: ConfirmAction::DeleteBuildNode(id.clone()),
                        });
                    }
                    if ui.small_button("复制").clicked() {
                        app.clone_build_node(&id);
                    }
                    if ui.small_button("配置").clicked() {
                        app.open_build_editor(Some(&id), None);
                    }
                });
            });
        })
        .response
}

fn set_build_group_name(app: &mut AdminApp, id: &str, name: &str) {
    let mut groups = app.build_groups();
    if let Some(group) = groups
        .iter_mut()
        .find(|group| group["id"].as_str() == Some(id))
    {
        group["name"] = serde_json::json!(if name.is_empty() { "构建组" } else { name });
    }
    app.set_cfg(&["buildGroups"], Value::Array(groups));
}

pub fn draw_build_editor(app: &mut AdminApp, ctx: &egui::Context) {
    let mut close = false;
    let mut save = false;
    let mut read_player = false;
    let mut write_player = false;
    let mut profile_changed = false;
    let profiles = app.profiles.clone();
    let vcs_groups = app.vcs_groups();
    if let Some(editor) = app.build_editor.as_mut() {
        let title = editor
            .editing_id
            .as_ref()
            .map(|_| "编辑打包节点".to_owned())
            .unwrap_or_else(|| "添加打包节点".into());
        egui::Window::new(title)
            .id(egui::Id::unique("build-editor"))
            .collapsible(false)
            .resizable(true)
            .default_width(760.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(650.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label(RichText::new("节点与 Profile").strong().color(ACCENT));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("节点名称").small().color(MUTED));
                            ui.add(
                                egui::TextEdit::singleline(&mut editor.name)
                                    .hint_text("例如 Wallpaper Windows x64")
                                    .desired_width(360.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("关联 Profile").small().color(MUTED));
                            let selected = editor.profile_asset_path.clone();
                            let selected_text = editor
                                .profile_asset_path
                                .is_empty()
                                .then_some("（请选择 Profile）".to_owned())
                                .unwrap_or_else(|| {
                                    profiles
                                        .iter()
                                        .cloned()
                                        .find(|profile| {
                                            profile["assetPath"].as_str()
                                                == Some(selected.as_str())
                                        })
                                        .map(|profile| {
                                            format!(
                                                "{} ({}) · {}",
                                                profile["name"].as_str().unwrap_or(""),
                                                profile["targetName"].as_str().unwrap_or(""),
                                                selected
                                            )
                                        })
                                        .unwrap_or_else(|| selected.clone())
                                });
                            egui::ComboBox::from_id_salt("editor-profile-select")
                                .selected_text(selected_text)
                                .width(470.0)
                                .show_ui(ui, |ui| {
                                    for profile in profiles.clone() {
                                        let asset_path =
                                            profile["assetPath"].as_str().unwrap_or("");
                                        let label = format!(
                                            "{} ({}) · {}",
                                            profile["name"].as_str().unwrap_or(""),
                                            profile["targetName"].as_str().unwrap_or(""),
                                            asset_path
                                        );
                                        if ui
                                            .selectable_label(
                                                editor.profile_asset_path == asset_path,
                                                label,
                                            )
                                            .clicked()
                                        {
                                            editor.profile_asset_path =
                                                asset_path.to_owned();
                                            editor.target = profile["target"]
                                                .as_i64()
                                                .unwrap_or(editor.target);
                                            if editor.name.trim().is_empty()
                                                || editor.name == "新建打包节点"
                                            {
                                                editor.name = profile["name"]
                                                    .as_str()
                                                    .unwrap_or("")
                                                    .to_owned();
                                            }
                                            if android_capable(editor.target) {
                                                if let Some(keystore) =
                                                    profile["keystoreName"].as_str()
                                                {
                                                    if editor.keystore_name.is_empty() {
                                                        editor.keystore_name =
                                                            keystore.to_owned();
                                                    }
                                                }
                                                if let Some(alias) =
                                                    profile["keyaliasName"].as_str()
                                                {
                                                    if editor.keyalias_name.is_empty()
                                                        || editor.keyalias_name == "key"
                                                    {
                                                        editor.keyalias_name =
                                                            alias.to_owned();
                                                    }
                                                }
                                            }
                                            profile_changed = true;
                                        }
                                    }
                                    if profiles.is_empty() {
                                        ui.label(
                                            RichText::new("未扫描到 Profile，请先扫描")
                                                .color(WARN),
                                        );
                                    }
                                });
                        });
                        if let Some(profile) = profiles.iter().find(|profile| {
                            profile["assetPath"].as_str()
                                == Some(editor.profile_asset_path.as_str())
                        }) {
                            ui.label(
                                RichText::new(format!(
                                    "平台 {} · {} 个场景 · 版本 {}{}",
                                    profile["targetName"].as_str().unwrap_or(""),
                                    profile["sceneCount"].as_i64().unwrap_or(0),
                                    profile["bundleVersion"].as_str().unwrap_or("未设置"),
                                    profile["productName"]
                                        .as_str()
                                        .map(|name| format!(" · {name}"))
                                        .unwrap_or_default()
                                ))
                                .small()
                                .color(MUTED),
                            );
                        }
                        ui.separator();
                        ui.label(RichText::new("缓存与命名").strong().color(ACCENT));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("自定义构建目录").small().color(MUTED));
                            ui.add(
                                egui::TextEdit::singleline(&mut editor.custom_build_dir)
                                    .hint_text("留空默认 Builds/{节点名}")
                                    .desired_width(400.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("产物命名模板").small().color(MUTED));
                            ui.add(
                                egui::TextEdit::singleline(&mut editor.name_template)
                                    .hint_text("留空继承全局命名模板")
                                    .desired_width(400.0),
                            );
                        });
                        ui.horizontal_wrapped(|ui| {
                            for token in [
                                "{Platform}",
                                "{Product}",
                                "{Profile}",
                                "{Version}",
                                "{VersionCode}",
                                "{Time}",
                                "{Date}",
                                "{Dev}",
                            ] {
                                if ui.small_button(token).clicked() {
                                    editor.name_template.push_str(token);
                                }
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(&mut editor.auto_zip, "自动压缩");
                            ui.checkbox(&mut editor.dev, "DEV 包");
                            ui.checkbox(&mut editor.build_addressables, "构建 Addressables");
                        });
                        if editor.build_addressables {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Addressables 方法")
                                        .small()
                                        .color(MUTED),
                                );
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut editor.addressables_method,
                                    )
                                    .hint_text("例如 MyTools.AddressableBuilder.BuildContent")
                                    .desired_width(420.0),
                                );
                            });
                        }
                        if android_capable(editor.target) {
                            ui.separator();
                            ui.label(
                                RichText::new("Android 构建与签名").strong().color(ACCENT),
                            );
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("产物类型").small().color(MUTED));
                                for (value, label) in [
                                    ("apk", "APK"),
                                    ("aab", "AAB"),
                                    ("gradleProject", "导出 Gradle 工程"),
                                ] {
                                    if ui
                                        .selectable_label(
                                            editor.android_build_kind == value,
                                            label,
                                        )
                                        .clicked()
                                    {
                                        editor.android_build_kind = value.into();
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Keystore").small().color(MUTED));
                                ui.add(
                                    egui::TextEdit::singleline(&mut editor.keystore_name)
                                        .hint_text("相对工程根目录，例如 KeyStore/user.keystore")
                                        .desired_width(400.0),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Alias").small().color(MUTED));
                                ui.add(
                                    egui::TextEdit::singleline(&mut editor.keyalias_name)
                                        .hint_text("key")
                                        .desired_width(200.0),
                                );
                                ui.checkbox(&mut editor.remember, "记住密码");
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Keystore 密码").small().color(MUTED));
                                let visible = show_password(ui, "build-keystore");
                                ui.add(
                                    egui::TextEdit::singleline(&mut editor.keystore_pass)
                                        .password(!visible)
                                        .hint_text("仅 Android 构建使用")
                                        .desired_width(300.0),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Alias 密码").small().color(MUTED));
                                let visible = show_password(ui, "build-keyalias");
                                ui.add(
                                    egui::TextEdit::singleline(&mut editor.keyalias_pass)
                                        .password(!visible)
                                        .hint_text("留空则同 Keystore 密码")
                                        .desired_width(300.0),
                                );
                            });
                            hint(
                                ui,
                                if editor.target == 0 {
                                    "旧版 Unity 项目默认节点：跟随当前激活平台；若为 Android 仍会注入签名。"
                                } else {
                                    "签名密码会随项目 config.json 保存；请限制配置目录权限。"
                                },
                            );
                        }
                        ui.separator();
                        ui.label(RichText::new("构建前版本更新").strong().color(ACCENT));
                        if vcs_groups.is_empty() {
                            hint(ui, "暂无版本分组；可在“版本管理”页创建。");
                        } else {
                            ui.horizontal_wrapped(|ui| {
                                for group in &vcs_groups {
                                    let id = group["id"].as_str().unwrap_or("").to_owned();
                                    let mut selected =
                                        editor.vcs_group_ids.contains(&id);
                                    let label = group["name"].as_str().unwrap_or("版本分组");
                                    if ui.checkbox(&mut selected, label).changed() {
                                        if selected {
                                            editor.vcs_group_ids.push(id);
                                        } else {
                                            editor.vcs_group_ids.retain(|value| value != group["id"].as_str().unwrap_or(""));
                                        }
                                    }
                                }
                            });
                        }
                        ui.separator();
                        ui.label(RichText::new("Player Settings").strong().color(ACCENT));
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Product Name / Company Name / Version")
                                    .small()
                                    .color(MUTED),
                            );
                            if ui.small_button("从文件读取").clicked() {
                                read_player = true;
                            }
                            if ui.small_button("写入 Player Settings").clicked() {
                                write_player = true;
                            }
                        });
                        if let Some(player) = editor.player.as_mut() {
                            if player.loading {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new("正在读取 Player Settings…").small().color(MUTED));
                                });
                            }
                            if !player.path.is_empty() {
                                ui.label(
                                    RichText::new(&player.path)
                                        .small()
                                        .monospace()
                                        .color(MUTED),
                                );
                            }
                            ui.horizontal_wrapped(|ui| {
                                if player.fallback {
                                    pill(ui, "旧版 ProjectSettings 回退", WARN);
                                }
                                if player.inherited {
                                    pill(ui, "部分字段继承项目设置", ACCENT_ALT);
                                }
                                if player.editable {
                                    pill(ui, "可安全写入", OK);
                                } else {
                                    pill(ui, "需在 Unity 中启用 Profile 覆盖", ERR);
                                }
                            });
                            egui::Grid::new("player-settings-fields")
                                .num_columns(2)
                                .spacing([12.0, 6.0])
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new("Product Name")
                                            .small()
                                            .color(MUTED),
                                    );
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut player.product_name,
                                        )
                                        .desired_width(420.0),
                                    );
                                    ui.end_row();
                                    ui.label(
                                        RichText::new("Company Name")
                                            .small()
                                            .color(MUTED),
                                    );
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut player.company_name,
                                        )
                                        .desired_width(420.0),
                                    );
                                    ui.end_row();
                                    ui.label(
                                        RichText::new("Version").small().color(MUTED),
                                    );
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut player.bundle_version,
                                        )
                                        .desired_width(240.0),
                                    );
                                    ui.end_row();
                                });
                            if !player.error.is_empty() {
                                ui.label(
                                    RichText::new(&player.error).small().color(ERR),
                                );
                            }
                            hint(
                                ui,
                                format!(
                                    "revision {}… · 外部修改会导致写入冲突",
                                    player.revision.chars().take(12).collect::<String>()
                                ),
                            );
                        } else {
                            hint(ui, "选择 Profile 后可读取并编辑其 Player Settings。");
                        }
                        if !editor.error.is_empty() {
                            ui.label(RichText::new(&editor.error).small().color(ERR));
                        }
                    });
                ui.add_space(6.0);
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
    if profile_changed && !write_player {
        read_player = true;
    }
    if read_player {
        app.read_player_settings();
    }
    if write_player {
        app.write_player_settings();
    }
    if save {
        app.save_build_editor();
    }
    if close {
        app.build_editor = None;
    }
}

fn android_capable(target: i64) -> bool {
    target == 0 || target == 13
}
