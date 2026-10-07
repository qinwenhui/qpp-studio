//! QPP Studio — 组装层。插件顺序：single-instance 必须最先注册。

pub mod batch;
pub mod clipboard;
pub mod commands;
pub mod dto;
pub mod engine;
pub mod history;
pub mod hw;
pub mod image_util;
pub mod hotkey;
pub mod ingest;
#[cfg(target_os = "macos")]
pub mod menu;
pub mod media;
pub mod pdf;
pub mod screenshot;
pub mod selftest;
pub mod settings;
pub mod thumb;
pub mod windowfx;
pub mod worker;

use settings::Settings;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager};

/// 各类目录（app_data / app_cache）。
pub struct Dirs {
    pub settings_file: PathBuf,
    pub cache: PathBuf,
    pub inbox: PathBuf,
    pub thumbs: PathBuf,
    pub shots: PathBuf,
}

/// 全局状态（Send+Sync）。
pub struct AppCtx {
    pub dirs: Dirs,
    pub settings: RwLock<Settings>,
    pub engine: engine::EngineManager,
    /// 启动时一次性检测的硬件信息(并行策略优化表的输入)
    pub hw: hw::HwInfo,
    pub media: media::MediaRegistry,
    pub items: RwLock<HashMap<String, ingest::ImageItem>>,
    /// 插入顺序（前端列表顺序）
    pub order: RwLock<Vec<String>>,
    pub batch: batch::BatchState,
    /// PDF 后台识别队列(逐本处理,与图片批量互斥)
    pub pdf: ingest::PdfOcrState,
    pub history: history::HistoryStore,
    pub shot: screenshot::SessionSlot,
    pub last_shot: Mutex<Option<dto::ItemOutcomeDto>>,
    /// 导入代数:新一次选择会作废上一次还在跑的导入(100 张全量解码耗时较长)
    pub ingest_gen: std::sync::atomic::AtomicU64,
}

impl AppCtx {
    pub fn register_item(&self, item: ingest::ImageItem) {
        let id = item.id.clone();
        self.items.write().unwrap().insert(id.clone(), item);
        self.order.write().unwrap().push(id);
    }

    pub fn item_dto(&self, id: &str) -> Option<dto::ImageItemDto> {
        let item = self.items.read().unwrap().get(id).cloned()?;
        Some(dto::ImageItemDto {
            id: item.id,
            name: item.name,
            path: item.path.to_string_lossy().into_owned(),
            w: item.w,
            h: item.h,
            origin: item.origin,
            added_at: item.added_at,
            media_token: item.media_token,
            thumb_token: item.thumb_token,
            can_extract: item.can_extract,
        })
    }

    pub fn remove_item(&self, id: &str) {
        // 只有 PDF 父条目才做连带清理(inbox 页缓存 / 兜底 PDF)。
        // thumbs/<id>.jpg 是历史记录的唯一缩略图来源(历史条目按 id 引用它),
        // 从「当前识别」移除一张图不能删盘上的文件,否则那条历史只能显示占位图标。
        let is_pdf = self
            .items
            .read()
            .unwrap()
            .get(id)
            .map(|i| i.origin == "pdf")
            .unwrap_or(false);
        self.items.write().unwrap().remove(id);
        self.order.write().unwrap().retain(|i| i != id);
        if is_pdf {
            ingest::cleanup_pdf(self, id);
        }
    }

    pub fn clear_items(&self) {
        // 先记下 PDF 条目(清空后无从判断 origin),再清 + 逐个回收资源
        let pdf_ids: Vec<String> = {
            let items = self.items.read().unwrap();
            self.order
                .read()
                .unwrap()
                .iter()
                .filter_map(|id| {
                    items
                        .get(id)
                        .filter(|i| i.origin == "pdf")
                        .map(|_| id.clone())
                })
                .collect()
        };
        self.items.write().unwrap().clear();
        self.order.write().unwrap().clear();
        for id in pdf_ids {
            ingest::cleanup_pdf(self, &id);
        }
    }
}

/// media:// 令牌 → 前端可用 URL。Windows/WebView2 是 http://media.localhost，
/// macOS/Linux 是 media://localhost（wry 已知行为，CSP 两种都放行）。
pub fn media_url(token: &str) -> String {
    if cfg!(windows) {
        format!("http://media.localhost/{token}")
    } else {
        format!("media://localhost/{token}")
    }
}

/// 模型目录解析：设置覆盖 → 资源目录（安装态）→ exe 同级（便携态）
/// → 开发态回退并排的 qppocr 仓库。
fn resolve_models_dir(app: &AppHandle, settings: &Settings) -> PathBuf {
    if let Some(dir) = &settings.models_dir {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return p;
        }
    }
    if let Ok(res) = app
        .path()
        .resolve("models", tauri::path::BaseDirectory::Resource)
    {
        if res.is_dir() {
            return res;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("models");
            if p.is_dir() {
                return p;
            }
        }
    }
    if cfg!(debug_assertions) {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../qppocr/models");
        if p.is_dir() {
            return p;
        }
    }
    PathBuf::from("models")
}

fn bootstrap(app: AppHandle) {
    let data_dir = app
        .path()
        .app_data_dir()
        .expect("app_data 目录不可用");
    let cache_dir = app.path().app_cache_dir().expect("app_cache 目录不可用");
    let (inbox, thumbs, shots) = (
        cache_dir.join("inbox"),
        cache_dir.join("thumbs"),
        cache_dir.join("shots"),
    );
    for d in [&data_dir, &cache_dir, &inbox, &thumbs, &shots] {
        let _ = std::fs::create_dir_all(d);
    }

    let settings = Settings::load(&data_dir.join("settings.json"));
    let models_dir = resolve_models_dir(&app, &settings);
    let hw_info = hw::detect();
    // 引擎线程数:用户显式设置优先;Apple Silicon 默认按 P 核数(避免 E 核
    // 拖慢单图延迟,进程级线程池启动即定型);其余平台 0 = 引擎自动
    let threads = if settings.threads > 0 {
        settings.threads
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        hw_info.physical_cores
    } else {
        0
    };
    let ctx = AppCtx {
        dirs: Dirs {
            settings_file: data_dir.join("settings.json"),
            cache: cache_dir,
            inbox,
            thumbs,
            shots,
        },
        engine: engine::EngineManager::new(threads, models_dir),
        hw: hw_info,
        batch: batch::BatchState::new(settings.batch_concurrency),
        pdf: ingest::PdfOcrState::default(),
        history: history::HistoryStore::load(data_dir.join("history.json")),
        media: media::MediaRegistry::default(),
        settings: RwLock::new(settings.clone()),
        items: RwLock::new(HashMap::new()),
        order: RwLock::new(Vec::new()),
        shot: Mutex::new(None),
        last_shot: Mutex::new(None),
        ingest_gen: std::sync::atomic::AtomicU64::new(0),
    };
    let tier = settings::parse_tier(&settings.tier).unwrap_or(qppocr::Tier::Tiny);
    let preset = settings::parse_preset(&settings.preset).unwrap_or(qppocr::Preset::Balanced);
    app.manage(ctx);

    // 硬件与并行策略一次性打日志(设置面板也有展示,这里给 dev 控制台/排障用)
    {
        let state = app.state::<AppCtx>();
        let device = settings::parse_device(&settings.device);
        let p = hw::plan_for(&state.hw, tier, settings.workers_override, &device);
        let gpu_list = if state.hw.gpus.is_empty() {
            "无 GPU".to_string()
        } else {
            state
                .hw
                .gpus
                .iter()
                .map(|g| format!("{} ({})", g.name, g.api))
                .collect::<Vec<_>>()
                .join(", ")
        };
        eprintln!(
            "[hw] {} · P{}/L{} · {:.1}GB · GPU: {} → {}: {} 进程 × {} 线程 · 小批量并发 {}{}",
            state.hw.cpu_brand,
            state.hw.physical_cores,
            state.hw.logical_cores,
            state.hw.total_mem as f64 / 1_073_741_824.0,
            gpu_list,
            settings::tier_str(tier),
            p.workers,
            p.threads_each,
            p.inproc_concurrency,
            if p.clamped_by_mem { "(内存闸压低)" } else { "" },
        );
    }

    // 启动即建引擎：锁死进程级线程池尺寸
    app.state::<AppCtx>().engine.spawn_init(
        app.clone(),
        engine::EngineSpec {
            tier,
            preset: if settings::is_special_preset(&settings.preset) {
                qppocr::Preset::Accuracy
            } else {
                preset
            },
            orientation: settings.orientation,
            enhance_contrast: settings.enhance_contrast,
            upscale: settings.upscale,
            special: settings::is_special_preset(&settings.preset),
            device: settings::parse_device(&settings.device),
        },
    );

    if let Err(e) = hotkey::register(&app, &settings.hotkey) {
        let _ = app.emit("app://toast", dto::ToastDto {
            level: "error".into(),
            message: e,
        });
    }
    windowfx::apply_theme_effect(&app, &settings.theme);
    ingest::spawn_pdf_queue(app.clone());
    ingest::spawn_ondemand_worker(app.clone());
    history::spawn_flush_thread(app.clone());
    let _ = app.emit("app://ready", ());
}

/// 主窗口:配置无法按平台分值,统一在 Rust 侧按平台创建。
/// - Windows/Linux:无边框自绘标题栏(前端 TitleBar 画三圆点)
/// - macOS:decorations + titleBarStyle Overlay——原生交通灯悬浮,
///   内容延伸到标题栏下(前端左区让位),窗口 transparent 走 macos-private-api
fn create_main_window(app: &tauri::App) -> tauri::Result<()> {
    let builder = tauri::WebviewWindowBuilder::new(
        app,
        "main",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("QPP Studio")
    .inner_size(1280.0, 800.0)
    .min_inner_size(960.0, 600.0)
    .center()
    .transparent(true);
    // drag-drop 处理器默认启用(与原 tauri.conf.json 的 dragDropEnabled 一致)

    #[cfg(target_os = "macos")]
    let builder = builder
        .decorations(true)
        .title_bar_style(tauri::TitleBarStyle::Overlay);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.decorations(false);

    builder.build()?;
    Ok(())
}

pub fn run() {
    // Apple Silicon 并行调优(引擎内核按 x86 标定):
    // - fork 门槛 4e6 在 M 系(小核多、单核快)过高,小算子切不出并行——
    //   qppocr-kernels 启动时读 QPPOCR_FORK_MACS,这里降为 2e6;
    //   必须在首次引擎调用前注入(OnceLock 锁定),worker 进程自动继承
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    // edition 2024 的 set_var 是 unsafe;此刻单线程、任何引擎代码未跑,安全
    unsafe {
        std::env::set_var("QPPOCR_FORK_MACS", "2000000");
    }

    // 工作线程 panic 默认无输出,静默吞掉故障 —— 开发期打出来
    if cfg!(debug_assertions) {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("[panic] {info}");
        }));
    }
    let selftest = std::env::var("QPP_SELFTEST").is_ok() || std::env::var("QPP_BENCH").is_ok();

    tauri::Builder::default()
        // 单实例必须最先注册：二次启动聚焦主窗口
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .register_asynchronous_uri_scheme_protocol("media", |ctx, request, responder| {
            media::handle(ctx, request, responder)
        })
        .setup(move |app| {
            create_main_window(app)?;
            #[cfg(target_os = "macos")]
            menu::setup(app)?;
            let handle = app.handle().clone();
            bootstrap(handle.clone());
            if selftest {
                selftest::spawn(handle);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_init,
            commands::engine_status,
            commands::hw_info,
            commands::device_benchmark,
            commands::set_window_title,
            commands::pick_images,
            commands::add_files,
            commands::read_clipboard_image,
            commands::ocr_image,
            commands::batch_start,
            commands::batch_cancel,
            commands::screenshot_begin,
            commands::screenshot_finish,
            commands::screenshot_cancel,
            commands::shot_window_monitor,
            commands::shot_result_data,
            commands::close_shot_result,
            commands::focus_main,
            commands::settings_get,
            commands::settings_set,
            commands::history_list,
            commands::history_thumb,
            commands::history_delete,
            commands::history_clear,
            commands::history_reopen,
            commands::remove_item,
            commands::remove_items,
            commands::clear_items,
            commands::export_content,
            commands::copy_text,
            commands::reveal_path,
            commands::open_url,
            commands::pdf_page_info,
            commands::pdf_render_page,
            commands::pdf_pause,
            commands::pdf_resume,
            commands::pdf_recognize_page,
            commands::pdf_page_outcome,
            commands::pdf_set_mode,
            commands::pdf_ocr_page,
            commands::pdf_ocr_range,
            commands::pdf_export_merged,
            commands::pdf_extract_all,
        ])
        .run(tauri::generate_context!())
        .expect("QPP Studio 启动失败");
}
