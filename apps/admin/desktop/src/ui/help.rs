use super::{card, hint, pill, section_title, ACCENT, ACCENT_ALT, CARD_ALT, MUTED};
use crate::app::AdminApp;
use eframe::egui;
use egui::RichText;

pub fn draw(ui: &mut egui::Ui, app: &AdminApp) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    section_title(
                        ui,
                        "帮助",
                        "UnityBuildTool 原生管理端完整使用说明与升级说明",
                    );
                    hint(
                        ui,
                        "管理端通过进程内 Rust Backend 执行构建、检测与版本操作；用户 Web 仍由内嵌 HTTP 服务提供。",
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    pill(ui, "Windows 原生 egui", ACCENT);
                });
            });
            ui.add_space(10.0);

            quick_start(ui, app);
            ui.add_space(10.0);
            build_guide(ui);
            ui.add_space(10.0);
            profiles_and_nodes_guide(ui);
            ui.add_space(10.0);
            check_and_vcs_guide(ui);
            ui.add_space(10.0);
            settings_guide(ui);
            ui.add_space(10.0);
            tray_lifecycle_guide(ui);
            ui.add_space(10.0);
            files_and_upgrade_guide(ui);
            ui.add_space(10.0);
            troubleshooting_guide(ui);
        });
}

fn quick_start(ui: &mut egui::Ui, app: &AdminApp) {
    card(ui, |ui| {
        ui.label(RichText::new("快速开始").strong().size(16.0));
        ui.add_space(4.0);
        guide(
            ui,
            "1",
            "在“设置”中创建或选择项目，填写项目名称与链接标识。",
        );
        guide(
            ui,
            "2",
            "在“设置 → Unity 引擎”填写 Unity.exe，或点击“自动探测”选择已安装版本。",
        );
        guide(
            ui,
            "3",
            "进入“Profile 与节点”，填写 Unity 工程路径，点击“扫描 Profile”。",
        );
        guide(
            ui,
            "4",
            "创建打包节点，选择 Profile，按需配置 DEV、Addressables、Android 签名与构建前版本更新。",
        );
        guide(
            ui,
            "5",
            "回到“构建”，勾选节点，建议先执行“命令预览”，确认无误后开始构建。",
        );
        ui.add_space(5.0);
        egui::Frame::new()
            .fill(CARD_ALT)
            .corner_radius(6.0)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                hint(
                    ui,
                    format!(
                        "当前服务地址：{} · 当前项目：{} · 输出目录：{}",
                        app.base_url,
                        app.current_project()
                            .and_then(|project| project["name"].as_str())
                            .unwrap_or("未选择"),
                        if app.cfg_str(&["outputBase"]).trim().is_empty() {
                            "Builds".to_owned()
                        } else {
                            app.cfg_str(&["outputBase"])
                        }
                    ),
                );
                hint(
                    ui,
                    "顶部“用户网页”打开当前项目 /p/{slug}；用户 Web 的接口、SSE 与下载行为与旧版一致。",
                );
            });
    });
}

fn build_guide(ui: &mut egui::Ui) {
    section(
        ui,
        "构建",
        "构建选择、命令预览、进度、产物与日志",
        |ui| {
            item(
                ui,
                "构建组",
                "构建组只用于筛选节点。选择组后可全选当前组或手动勾选；开始构建只执行勾选项。",
            );
            item(
                ui,
                "构建号",
                "-1 表示自动读取 Unity 工程的 AndroidBundleVersionCode，执行后递增并写回；正数是所有节点共用的固定值。",
            );
            item(
                ui,
                "失败停止",
                "启用“失败后停止后续节点”时，首个失败节点会终止该项目的后续队列；全局队列中其他项目仍按调度规则执行。",
            );
            item(
                ui,
                "命令预览",
                "仅生成命令和预计输出，不启动 Unity。建议首次配置或修改模板后先预览。",
            );
            item(
                ui,
                "运行进度",
                "每个节点会经历启动、切平台、构建 Addressables、Unity 编译、归档、压缩等阶段；真实日志会过滤 Unity 进度条噪声。",
            );
            item(
                ui,
                "AI 失败分析",
                "仅在设置中启用 AI 后可用。请求发送最近构建日志和必要上下文；密钥保存在配置文件中，建议使用限额 Key。",
            );
            item(
                ui,
                "产物与日志",
                "产物表可打开构建目录；UI 构建日志只保留最近 5000 行，完整日志仍写入各节点输出目录的 build.log。",
            );
        },
    );
}

fn profiles_and_nodes_guide(ui: &mut egui::Ui) {
    section(
        ui,
        "Profile 与节点",
        "扫描 Build Profile、配置节点与 Player Settings",
        |ui| {
            item(
                ui,
                "扫描",
                "Unity 6 Build Profile 从 Assets/Settings/Build Profiles 扫描；可在设置页增加附加 Profile 目录，目录会递归查找 .asset。",
            );
            item(
                ui,
                "旧版工程",
                "没有 Build Profile 的旧工程会生成“项目默认（跟随工程）”节点，读取 ProjectSettings。",
            );
            item(
                ui,
                "拖拽排序与分组",
                "拖动节点到其他构建组会立即归组；拖到节点前方可调整顺序。分组名可直接编辑。",
            );
            item(
                ui,
                "节点配置",
                "支持自定义构建目录、成品命名模板、自动压缩、DEV 包、Addressables、Android APK/AAB/Android Studio 工程、Keystore 与构建前 VCS 分组。",
            );
            item(
                ui,
                "Player Settings",
                "节点编辑器可读取并写入 Product Name、Company Name、Version。修改基于 SHA-256 revision；外部修改后保存会提示冲突，避免覆盖。任务运行期间拒绝写入。",
            );
            item(
                ui,
                "原子保存",
                "写入先落临时文件再替换原文件，尽量保留无关 YAML 字段；仍建议使用版本控制并审阅配置变化。",
            );
            item(
                ui,
                "密码",
                "Keystore / Key Alias 密码可显隐；只在“记住密码”时保存到配置文件。Android 构建会通过临时配置注入 Unity。",
            );
        },
    );
}

fn check_and_vcs_guide(ui: &mut egui::Ui) {
    section(
        ui,
        "环境检测与版本管理",
        "切换构建目标做编译检测；Git / SVN 还原、分支与更新",
        |ui| {
            item(
                ui,
                "环境检测",
                "可独立指定检测工程、Unity.exe、超时与 -nographics。检测只编译目标平台，不生成最终安装包。",
            );
            item(
                ui,
                "环境线",
                "可多选 BuildTarget。结果区按环境线显示状态、耗时与错误信息，日志可搜索、复制、自动滚动与清空当前视图。",
            );
            item(
                ui,
                "检测目录",
                "检测结果位于检测工程 .buildtool/check-results；每轮有独立目录，可一键打开。",
            );
            item(
                ui,
                "VCS 节点",
                "每个节点独立配置路径、类型、目标分支和还原命令。探测可识别 Git / SVN、当前分支、revision、脏文件与远端状态。",
            );
            item(
                ui,
                "更新顺序",
                "先执行还原命令，再切换分支，只有远端在线时才 pull / update；无远端或离线不会伪造成功。",
            );
            item(
                ui,
                "构建前更新",
                "节点编辑器可绑定一个或多个版本分组。构建该节点前依次更新分组；失败是否终止由“失败即停”控制。",
            );
            item(
                ui,
                "拖拽",
                "版本节点同样支持拖拽排序与归组；全部更新按当前顺序执行，可随时停止。",
            );
        },
    );
}

fn settings_guide(ui: &mut egui::Ui) {
    section(
        ui,
        "设置",
        "项目、服务、引擎、命名、Android、代理与 AI",
        |ui| {
            item(
                ui,
                "项目管理",
                "项目名称、slug 与启用状态保存在根配置。删除项目只删配置，不删除 Unity 工程和已生成产物。",
            );
            item(
                ui,
                "Web 服务",
                "监听地址与固定端口属于全局配置。端口 0 为随机端口；修改后需重启管理端。默认仅建议 127.0.0.1。",
            );
            item(
                ui,
                "Unity 引擎",
                "批处理模式直接调用 Unity.exe；自定义模板模式可用 {cli}、{project}、{profilePath} 等占位符接管完整命令。",
            );
            item(
                ui,
                "命名与归档",
                "全局成品模板、输出根目录、成功归档目录和 autoZip/dev/Addressables/Android 默认值可被节点覆盖。",
            );
            item(
                ui,
                "Android 工具",
                "SDK/NDK/JDK 可自动扫描或手动填写，构建时作为环境变量注入。打开目录用于确认路径有效。",
            );
            item(
                ui,
                "代理",
                "启用后为 Android Gradle 写入 HTTP/HTTPS 代理系统属性。",
            );
            item(
                ui,
                "Application 路径",
                "从 Unity Editor 采集 dataPath、streamingAssetsPath、persistentDataPath、temporaryCachePath、consoleLogPath；未采集项会标明来源。",
            );
            item(
                ui,
                "项目快捷目录",
                "工程、Assets、ProjectSettings、Packages、Profile、构建输出、成功归档、检测结果与仓库路径可一键打开。",
            );
            item(
                ui,
                "自动保存",
                "设置与节点编辑采用 400ms 防抖保存和单飞写队列。保存失败会保留草稿；运行/排队期间写操作锁定。",
            );
        },
    );
}

fn tray_lifecycle_guide(ui: &mut egui::Ui) {
    section(
        ui,
        "托盘、关闭与退出",
        "窗口隐藏后 HTTP 服务继续运行",
        |ui| {
            item(
                ui,
                "关闭窗口",
                "点击窗口关闭按钮只会隐藏管理端，不会停止 HTTP 服务或构建；用户仍可访问 Web。",
            );
            item(
                ui,
                "托盘左键",
                "显示或隐藏管理端窗口。右键菜单提供“显示/隐藏管理端”“打开用户网页”“退出”。",
            );
            item(
                ui,
                "退出",
                "退出会先设置停止标志并终止 Unity、检测、VCS 等子进程，等待任务释放后再关闭；随后固定端口会被释放。",
            );
            item(
                ui,
                "单实例",
                "当前版本不提供多实例锁；请避免同时运行两个管理端实例争用同一端口和配置。",
            );
        },
    );
}

fn files_and_upgrade_guide(ui: &mut egui::Ui) {
    section(
        ui,
        "配置、安装与升级",
        "固定路径和安装器契约",
        |ui| {
            item(
                ui,
                "配置路径",
                r"安装版：%APPDATA%\com.unitybuildtool.admin\config.json。调试版使用仓库 web/config.json 与 web 作为 runtime root。",
            );
            item(
                ui,
                "配置格式",
                "schemaVersion 为 2。项目字段按项目独立保存；未知字段会随 JSON 草稿保留，不会因 UI 未展示而丢失。",
            );
            item(
                ui,
                "安装包",
                "NSIS 安装包位于 target/release/bundle/nsis/，主程序名为 unitybuildtool-admin.exe。",
            );
            item(
                ui,
                "原位升级",
                "安装器会结束旧管理端、读取旧卸载注册项的 InstallLocation 或 UninstallString，静默卸载旧自定义目录，再安装到同一目录。",
            );
            item(
                ui,
                "保留配置",
                "升级不会删除 AppData 配置。新卸载器提供可选删除配置目录，默认保留，便于回装。",
            );
            item(
                ui,
                "用户 Web",
                "用户页面、项目工作台、HTTP API、SSE 字段与下载行为保持兼容；管理端不再暴露浏览器管理页面或管理 HTTP 路由。",
            );
        },
    );
}

fn troubleshooting_guide(ui: &mut egui::Ui) {
    section(
        ui,
        "常见问题",
        "启动、端口、Unity 与构建失败",
        |ui| {
            item(
                ui,
                "端口占用",
                "启动时若固定端口被占用会显示原生错误并退出。关闭旧管理端，或在设置中改用其他端口。",
            );
            item(
                ui,
                "扫描不到 Profile",
                "确认工程包含 Assets 与 ProjectSettings、Unity 版本至少支持 Build Profile，或使用旧版默认 Profile 回退。",
            );
            item(
                ui,
                "Unity 未启动",
                "检查 Unity.exe 路径、License、工程未锁定、Android 模块已安装；命令预览可用于定位参数问题。",
            );
            item(
                ui,
                "Android 构建失败",
                "优先检查 SDK/NDK/JDK 路径、网络代理、Keystore 密码、Gradle 模板与磁盘空间。",
            );
            item(
                ui,
                "构建号未递增",
                "自动模式要求 AndroidBundleVersionCode 可读取且工程可写；构建期间用户 Web 与配置修改会锁定。",
            );
            item(
                ui,
                "Player Settings 冲突",
                "若 Unity 或 Git 在读取后修改了 Profile / ProjectSettings，重新读取后再编辑。revision 不匹配会拒绝覆盖。",
            );
            item(
                ui,
                "日志缺失",
                "UI 只保留最近 5000 行，监控会话日志保留 300 条；完整文件日志位于节点输出目录 build.log。",
            );
        },
    );
}

fn section(ui: &mut egui::Ui, title: &str, subtitle: &str, contents: impl FnOnce(&mut egui::Ui)) {
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).strong().size(16.0).color(ACCENT));
            hint(ui, subtitle);
        });
        ui.add_space(5.0);
        contents(ui);
    });
}

fn guide(ui: &mut egui::Ui, step: &str, text: &str) {
    ui.horizontal_wrapped(|ui| {
        pill(ui, step, ACCENT_ALT);
        ui.label(RichText::new(text).color(egui::Color32::from_rgb(203, 213, 225)));
    });
}

fn item(ui: &mut egui::Ui, title: &str, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(title)
                .strong()
                .color(egui::Color32::from_rgb(226, 232, 240)),
        );
        ui.label(RichText::new(text).small().color(MUTED));
    });
    ui.add_space(2.0);
}
