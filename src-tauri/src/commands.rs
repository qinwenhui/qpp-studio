//! 全部 Tauri 命令。自定义命令不受 ACL 约束（插件/core 命令才走 capabilities）。

use crate::dto::{
    EngineStatusDto, ImageItemDto, InitInfoDto, ItemOutcomeDto, OcrOutcomeDto, ShotMonitorDto,
};
use crate::dto;
use crate::settings::{parse_device, parse_preset, parse_tier, Settings};
use crate::AppCtx;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub fn app_init(app: AppHandle, state: State<AppCtx>) -> InitInfoDto {
    InitInfoDto {
        settings: state.settings.read().unwrap().clone(),
        engine: state.engine.status(),
        version: app.package_info().version.to_string(),
        platform: std::env::consts::OS.into(),
    }
}

/// 窗口标题显示当前文档名(图片/PDF 文件名),由前端在当前条目变化时调用;
/// None 恢复默认标题。mac 上标题进 Cmd+Tab/程序坞;Windows 无边框窗口
/// 标题不可见,但任务栏 tooltip 同样受益。
#[tauri::command]
pub fn set_window_title(app: AppHandle, title: Option<String>) {
    if let Some(w) = app.get_webview_window("main") {
        let text = title.unwrap_or_else(|| "QPP Studio".into());
        let _ = w.set_title(&text);
    }
}

#[tauri::command]
pub fn engine_status(state: State<AppCtx>) -> EngineStatusDto {
    state.engine.status()
}

/// 硬件检测信息 + 当前并行策略(按设置里的 tier/device/workers_override 实时计算)。
#[tauri::command]
pub fn hw_info(state: State<AppCtx>) -> dto::HwInfoDto {
    let settings = state.settings.read().unwrap();
    let tier = parse_tier(&settings.tier).unwrap_or(qppocr::Tier::Tiny);
    let device = parse_device(&settings.device);
    let p = crate::hw::plan_for(&state.hw, tier, settings.workers_override, &device);
    dto::HwInfoDto {
        cpu_brand: state.hw.cpu_brand.clone(),
        physical_cores: state.hw.physical_cores as u32,
        logical_cores: state.hw.logical_cores as u32,
        total_mem_gb: (state.hw.total_mem as f64 / 1_073_741_824.0 * 10.0).round() / 10.0,
        gpus: state
            .hw
            .gpus
            .iter()
            .map(|g| dto::GpuDto {
                name: g.name.clone(),
                api: g.api.clone(),
            })
            .collect(),
        plan: dto::PlanDto {
            workers: p.workers,
            threads_each: p.threads_each,
            inproc_concurrency: p.inproc_concurrency,
            mem_cap: p.mem_cap.min(u32::MAX as usize) as u32,
            clamped_by_mem: p.clamped_by_mem,
        },
    }
}

/// rfd 异步对话框:macOS 上 AppKit 面板必须由主线程创建,rfd 的 AsyncFileDialog
/// 内部自行分发主线程(sheet 模态挂主窗口);Windows 上则自开线程,双平台通用,
/// 任何线程都能 await(rfd 的 async future 标了 Send)。
#[tauri::command]
pub async fn pick_images(app: AppHandle) -> Result<Vec<ImageItemDto>, String> {
    let handles = rfd::AsyncFileDialog::new()
        .add_filter("图片和 PDF", &["png", "jpg", "jpeg", "bmp", "pdf"])
        .add_filter("PDF 文档", &["pdf"])
        .pick_files()
        .await
        .unwrap_or_default();
    let paths: Vec<PathBuf> = handles.iter().map(|h| h.path().to_path_buf()).collect();
    ingest_paths(&app, paths).await
}

#[tauri::command]
pub async fn add_files(app: AppHandle, paths: Vec<String>) -> Result<Vec<ImageItemDto>, String> {
    let paths = paths.into_iter().map(PathBuf::from).collect();
    ingest_paths(&app, paths).await
}

/// 入库含解码（缩略图要全量解码,大批量时串行极慢）:
/// 4 线程并行 + 逐张广播进度 + 代数守卫(新一次选择作废旧导入)。
async fn ingest_paths(app: &AppHandle, paths: Vec<PathBuf>) -> Result<Vec<ImageItemDto>, String> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tauri::Emitter;

    let gen_id = app
        .state::<AppCtx>()
        .ingest_gen
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    let total = paths.len();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // (原始下标, 路径):并行处理后按下标还原选择顺序
        let queue: std::sync::Mutex<Vec<(usize, PathBuf)>> =
            std::sync::Mutex::new(paths.into_iter().enumerate().collect());
        let results: std::sync::Mutex<Vec<(usize, ImageItemDto)>> =
            std::sync::Mutex::new(Vec::new());
        let first_err = std::sync::Mutex::new(None::<String>);
        let done = AtomicUsize::new(0);
        let is_stale = |g: u64| {
            app.state::<AppCtx>().ingest_gen.load(Ordering::SeqCst) != g
        };

        std::thread::scope(|scope| {
            for _ in 0..4 {
                let app = app.clone();
                scope.spawn(|| {
                    let app = app;
                    loop {
                        if is_stale(gen_id) {
                            return;
                        }
                        let next = { queue.lock().unwrap().pop() };
                        let Some((idx, p)) = next else { return };
                        match crate::ingest::ingest_file(&app, &p, "file") {
                            Ok(dto) => results.lock().unwrap().push((idx, dto)),
                            Err(e) => {
                                *first_err.lock().unwrap() = Some(e);
                            }
                        }
                        let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                        let _ = app.emit(
                            "ingest://progress",
                            serde_json::json!({ "done": d, "total": total }),
                        );
                    }
                });
            }
        });

        if is_stale(gen_id) {
            // 已被更新的选择取代:静默放弃
            return Ok(Vec::new());
        }
        let mut out = results.into_inner().unwrap();
        out.sort_by_key(|(idx, _)| *idx);
        let out: Vec<ImageItemDto> = out.into_iter().map(|(_, d)| d).collect();
        if out.is_empty() {
            if let Some(e) = first_err.into_inner().unwrap() {
                return Err(e);
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn read_clipboard_image(app: AppHandle) -> Result<ImageItemDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let img = crate::clipboard::read_image()?;
        crate::ingest::ingest_bitmap(&app, "剪贴板.png", img, "clipboard")
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 单张交互识别：直接 await 返回，同时也有事件广播。
/// `rotation`:画布显示旋转(顺时针 90/180/270),非 0 时烘焙进送引擎的像素,
/// 框坐标逆变换回原图系——「转正后重新识别」。
#[tauri::command]
pub async fn ocr_image(
    app: AppHandle,
    id: String,
    rotation: Option<u32>,
) -> Result<OcrOutcomeDto, String> {
    let rotation = rotation.unwrap_or(0);
    tauri::async_runtime::spawn_blocking(move || {
        if rotation % 360 == 0 {
            crate::batch::run_item_blocking(&app, &id)
        } else {
            crate::batch::run_item_rotated_blocking(&app, &id, rotation)
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn batch_start(
    app: AppHandle,
    state: State<'_, AppCtx>,
    ids: Vec<String>,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    if state.batch.running.swap(true, Ordering::SeqCst) {
        return Err("已有批量任务在进行中".into());
    }
    // PDF 舰队与图片批量共用 worker 预算,互斥执行。
    // 先 swap(running) 再查 fleet_active,与 PDF 侧的先立旗再查 running 对称,无竞态窗口
    if state.pdf.fleet_active.load(Ordering::SeqCst) {
        state.batch.running.store(false, Ordering::SeqCst);
        return Err("PDF 并行识别进行中,请稍后再试".into());
    }
    state.batch.cancel.store(false, Ordering::SeqCst);

    // 收集 (id, path, 像素权重):批量期间条目不会被动(当前识别语义保证)
    let mut tasks = Vec::with_capacity(ids.len());
    {
        let items = state.items.read().unwrap();
        for id in &ids {
            if let Some(item) = items.get(id) {
                tasks.push((id.clone(), item.path.clone(), (item.w as u64) * (item.h as u64)));
            }
        }
    }

    // 大批量 + 档位允许 → 进程分治(每 worker 独立引擎,真正吃满核)。
    // GPU 同样走舰队:实测(QPP_BENCH,GPU tiny,无错口径)4 进程 13.7 张/s
    // > 进程内并发×2 的 11.1——共享引擎受「同形状流水深度≤2」约束,
    // 多进程各持引擎反而绕开限流。小批量(<8)仍走下方进程内路径
    // (GPU inproc=2,同样受流水深度约束)
    let workers_override = state.settings.read().unwrap().workers_override;
    let spec = state.engine.spec();
    let plan = crate::hw::plan_for(&state.hw, spec.tier, workers_override, &spec.device);
    if tasks.len() >= crate::batch::PROC_BATCH_MIN && plan.workers > 0 {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::batch::spawn_proc_batch(app, tasks, plan);
        })
        .await
        .map_err(|e| e.to_string())?;
        return Ok(());
    }

    // 小批量:进程内共享引擎并发(并发 run 在算子层部分重叠)
    let user_conc = state.settings.read().unwrap().batch_concurrency;
    let conc = if user_conc == 0 { plan.inproc_concurrency } else { user_conc };
    let sem = Arc::new(tokio::sync::Semaphore::new(conc.max(1)));
    *state.batch.semaphore.write().unwrap() = sem.clone();
    let cancel = state.batch.cancel.clone();
    let mut handles = Vec::new();
    for id in ids {
        let app = app.clone();
        let sem = sem.clone();
        let cancel = cancel.clone();
        handles.push(tauri::async_runtime::spawn(async move {
            let _permit = sem.acquire().await;
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let app = app.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                crate::batch::run_item_blocking(&app, &id)
            })
            .await;
        }));
    }
    // 等本批全部落地再收尾:否则前端进度条会停在最后一张
    tauri::async_runtime::spawn(async move {
        for h in handles {
            let _ = h.await;
        }
        let state = app.state::<AppCtx>();
        state.batch.running.store(false, Ordering::SeqCst);
        let _ = app.emit("batch://done", ());
    });
    Ok(())
}

#[tauri::command]
pub fn batch_cancel(state: State<AppCtx>) {
    state.batch.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
}

// ---- 截图 ----

#[tauri::command]
pub fn screenshot_begin(app: AppHandle) {
    crate::screenshot::begin(app);
}

#[tauri::command]
pub fn screenshot_finish(
    app: AppHandle,
    mon: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Result<(), String> {
    crate::screenshot::finish(app, mon, x, y, w, h)
}

#[tauri::command]
pub fn screenshot_cancel(app: AppHandle) {
    crate::screenshot::cancel(&app);
}

/// 覆盖层页面按自己的窗口标签取所属屏幕信息。
#[tauri::command]
pub fn shot_window_monitor(
    _app: AppHandle,
    state: State<AppCtx>,
    label: String,
) -> Result<ShotMonitorDto, String> {
    let index: u32 = label
        .strip_prefix("shot-")
        .and_then(|s| s.parse().ok())
        .ok_or("非截图覆盖窗口")?;
    let guard = state.shot.lock().unwrap();
    let session = guard.as_ref().ok_or("截图会话已结束")?;
    let m = session
        .shots
        .get(index as usize)
        .ok_or("无效的显示器索引")?;
    Ok(ShotMonitorDto {
        url: crate::media_url(&m.token),
        w: m.w,
        h: m.h,
        dpr: m.scale as f64,
    })
}

/// 结果弹窗数据。
#[tauri::command]
pub fn shot_result_data(state: State<AppCtx>) -> Option<ItemOutcomeDto> {
    state.last_shot.lock().unwrap().clone()
}

#[tauri::command]
pub fn close_shot_result(app: AppHandle) {
    if let Some(w) = app.get_webview_window("shot-result") {
        let _ = w.close();
    }
}

/// 弹窗里「查看详情」：聚焦主窗口并关闭弹窗。
#[tauri::command]
pub fn focus_main(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    if let Some(w) = app.get_webview_window("shot-result") {
        let _ = w.close();
    }
}

// ---- 设置 ----

#[tauri::command]
pub fn settings_get(state: State<AppCtx>) -> Settings {
    state.settings.read().unwrap().clone()
}

/// 保存设置并触发副作用（热键/主题/引擎重建/批量并发）。
/// 热键注册失败直接 Err（不持久化），前端标红提示。
#[tauri::command]
pub async fn settings_set(
    app: AppHandle,
    state: State<'_, AppCtx>,
    settings: Settings,
) -> Result<Settings, String> {
    let old = state.settings.read().unwrap().clone();
    if settings.hotkey != old.hotkey {
        crate::hotkey::set(&app, &old.hotkey, &settings.hotkey)?;
    }
    let theme_changed = settings.theme != old.theme;
    let engine_changed = settings.tier != old.tier
        || settings.preset != old.preset
        || settings.device != old.device
        || settings.orientation != old.orientation
        || settings.enhance_contrast != old.enhance_contrast
        || settings.upscale != old.upscale;
    let conc_changed = settings.batch_concurrency != old.batch_concurrency;
    {
        let mut s = state.settings.write().unwrap();
        *s = settings.clone();
        s.save(&state.dirs.settings_file)?;
    }
    if theme_changed {
        crate::windowfx::apply_theme_effect(&app, &settings.theme);
    }
    if engine_changed {
        let tier = parse_tier(&settings.tier).unwrap_or(qppocr::Tier::Tiny);
        // 切档前体检:文件不齐就直接拒绝,不让应用进入报错态
        crate::engine::EngineManager::tier_preflight(&state.engine.models_dir(), tier)?;
        let spec = crate::engine::EngineSpec {
            tier,
            preset: if crate::settings::is_special_preset(&settings.preset) {
                qppocr::Preset::Accuracy
            } else {
                parse_preset(&settings.preset).unwrap_or(qppocr::Preset::Balanced)
            },
            special: crate::settings::is_special_preset(&settings.preset),
            orientation: settings.orientation,
            enhance_contrast: settings.enhance_contrast,
            upscale: settings.upscale,
            device: crate::settings::parse_device(&settings.device),
        };
        state.engine.reconfigure(app.clone(), spec);
    }
    if conc_changed && !state.batch.running.load(std::sync::atomic::Ordering::SeqCst) {
        *state.batch.semaphore.write().unwrap() = std::sync::Arc::new(
            tokio::sync::Semaphore::new(settings.batch_concurrency.max(1)),
        );
    }
    let _ = app.emit("settings://changed", &settings);
    Ok(settings)
}

// ---- 历史 ----

#[tauri::command]
pub fn history_list(
    state: State<AppCtx>,
    offset: usize,
    limit: usize,
) -> Vec<crate::dto::HistoryEntryDto> {
    let mut list = state.history.list(offset, limit);
    // 令牌不持久化,列表时按条目 id 现注册 thumbs/<id>.jpg。
    // 必须用 e.id 查路径:e.thumb_token 是进程内令牌,落盘后重启即失效。
    for e in &mut list {
        let thumb = state.dirs.thumbs.join(format!("{}.jpg", e.id));
        e.thumb_token = if thumb.is_file() {
            Some(state.media.register(thumb, "image/jpeg"))
        } else {
            None // 缺失的由前端按需调 history_thumb 补生成
        };
    }
    list
}

/// 历史条目缺缩略图时按需补生成(源文件仍在的前提下),返回新令牌。
/// 列表接口保持轻量:只有真正缺图的条目才会付解码代价。
#[tauri::command]
pub async fn history_thumb(app: AppHandle, id: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppCtx>();
        let thumb = state.dirs.thumbs.join(format!("{id}.jpg"));
        if thumb.is_file() {
            return Ok(Some(state.media.register(thumb, "image/jpeg")));
        }
        let entry = state.history.get(&id).ok_or("历史条目不存在")?;
        let src = PathBuf::from(&entry.path);
        if !src.is_file() {
            return Ok(None); // 源已删除,保持占位图标
        }
        let img = qppocr::decode_file(&src).map_err(|e| e.to_string())?;
        crate::thumb::make_thumb_from_rgb(
            img.w as u32,
            img.h as u32,
            &img.data,
            &id,
            &state.dirs.thumbs,
        )?;
        Ok(Some(state.media.register(thumb, "image/jpeg")))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn history_delete(state: State<AppCtx>, id: String) {
    state.history.delete(&id);
}

#[tauri::command]
pub fn history_clear(state: State<AppCtx>) {
    state.history.clear();
}

/// 历史重开：重建 media 令牌（令牌不持久化），返回条目与结果。
#[tauri::command]
pub async fn history_reopen(app: AppHandle, id: String) -> Result<ItemOutcomeDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppCtx>();
        let entry = state.history.get(&id).ok_or("历史条目不存在")?;
        let path = PathBuf::from(&entry.path);
        if !path.is_file() {
            return Err(format!("源文件已不存在：{}", entry.path));
        }
        let thumb = state.dirs.thumbs.join(format!("{id}.jpg"));
        if !thumb.exists() {
            // 入库已不解码,重开时按需补缩略图
            let img = qppocr::decode_file(&path).map_err(|e| e.to_string())?;
            crate::thumb::make_thumb_from_rgb(
                img.w as u32,
                img.h as u32,
                &img.data,
                &id,
                &state.dirs.thumbs,
            )?;
        }
        let media_token = state.media.register(path.clone(), crate::media::mime_for(&path));
        let thumb_token = state.media.register(thumb, "image/jpeg");
        let item = crate::ingest::ImageItem {
            id: entry.id.clone(),
            name: entry.name.clone(),
            path,
            w: entry.w,
            h: entry.h,
            origin: entry.origin.clone(),
            added_at: entry.at,
            media_token,
            thumb_token,
            can_extract: false,
        };
        let dto = ImageItemDto {
            id: item.id.clone(),
            name: item.name.clone(),
            path: item.path.to_string_lossy().into_owned(),
            w: item.w,
            h: item.h,
            origin: item.origin.clone(),
            added_at: item.added_at,
            media_token: item.media_token.clone(),
            thumb_token: item.thumb_token.clone(),
            can_extract: item.can_extract,
        };
        state.register_item(item);
        Ok(ItemOutcomeDto {
            item: dto,
            outcome: entry.outcome.clone(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- 图片条目管理 ----

#[tauri::command]
pub fn remove_item(state: State<AppCtx>, id: String) {
    state.remove_item(&id);
}

/// 批量移除:新批次替换「当前识别」时精确清掉旧条目(不碰刚 ingest 的新批次)。
#[tauri::command]
pub fn remove_items(state: State<AppCtx>, ids: Vec<String>) {
    for id in ids {
        state.remove_item(&id);
    }
}

#[tauri::command]
pub fn clear_items(state: State<AppCtx>) {
    state.clear_items();
}

// ---- 导出 / 剪贴板 / 打开位置 ----

/// 前端已持有文本内容，这里只负责选目标路径 + 落盘。
#[tauri::command]
pub async fn export_content(
    content: String,
    fmt: String,
    default_name: String,
) -> Result<String, String> {
    let ext = match fmt.as_str() {
        "json" => "json",
        "md" => "md",
        _ => "txt",
    };
    let Some(handle) = rfd::AsyncFileDialog::new()
        .add_filter("文件", &[ext])
        .set_file_name(&default_name)
        .save_file()
        .await
    else {
        return Err("已取消".into());
    };
    let path = handle.path().to_path_buf();
    std::fs::write(&path, content).map_err(|e| format!("写入失败: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn copy_text(text: String) -> Result<(), String> {
    crate::clipboard::set_text(&text)
}

#[tauri::command]
pub fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| e.to_string())
}

/// 在系统默认浏览器打开外部链接(关于页的 GitHub/作者主页)。
#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

// ---- PDF ----

/// 获取 PDF 条目状态:(总页数, 按需标记, 已完成页数, 直提标记)。
/// completed 供前端在「事件早于条目入 store 被丢」的竞态下自愈(直提 0 秒完成)。
#[tauri::command]
pub fn pdf_page_info(id: String) -> Option<(u32, u32, u32, u32)> {
    let meta = crate::ingest::PDF_PAGES.lock().unwrap().get(&id).cloned()?;
    let completed = crate::ingest::PDF_RESULTS
        .lock()
        .unwrap()
        .get(&id)
        .map(|m| m.len() as u32)
        .unwrap_or(0);
    Some((
        meta.count,
        if meta.on_demand { 1 } else { 0 },
        completed,
        if meta.extract { 1 } else { 0 },
    ))
}

/// 拉取某页识别结果(事件竞态丢失后的自愈路径)。
#[tauri::command]
pub fn pdf_page_outcome(id: String, page: u32) -> Option<crate::dto::OcrOutcomeDto> {
    crate::ingest::PDF_RESULTS
        .lock()
        .unwrap()
        .get(&id)
        .and_then(|m| m.get(&page))
        .cloned()
}

/// 整册切换识别方式:extract=true 直提(需文本层),false=OCR。
/// 返回是否真的切换了(模式相同时 false)。
#[tauri::command]
pub fn pdf_set_mode(app: AppHandle, id: String, extract: bool) -> Result<bool, String> {
    crate::ingest::set_pdf_mode(&app, &id, extract)
}

/// 本页强制 OCR(文本层 PDF 上重识别这一页,覆盖直提结果)。
#[tauri::command]
pub fn pdf_ocr_page(app: AppHandle, id: String, page: u32) {
    crate::ingest::recognize_page_force_ocr(&app, &id, page);
}

/// 暂停该 PDF 的识别(静默中断,不产生错误结果)。返回当前已完成页数。
#[tauri::command]
pub fn pdf_pause(app: AppHandle, id: String) -> Option<u32> {
    crate::ingest::pause_pdf(&app, &id)
}

/// 继续识别缺失页(按需模式的「识别全部」也是它):入队只含未完成页的任务。
/// 返回是否真的有活干——false 时前端应直接落定「已完成」,别乐观等待事件。
#[tauri::command]
pub fn pdf_resume(app: AppHandle, state: State<AppCtx>, id: String) -> Result<bool, String> {
    let (total, pages) =
        crate::ingest::remaining_pages(&id).ok_or("PDF 条目不存在")?;
    if pages.is_empty() {
        return Ok(false);
    }
    let meta = crate::ingest::PDF_PAGES
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("PDF 条目不存在")?;
    let src = crate::ingest::PDF_STORE
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("PDF 条目不存在")?;
    let name = state
        .items
        .read()
        .unwrap()
        .get(&id)
        .map(|i| i.name.clone())
        .unwrap_or_default();
    // 直提 PDF 的继续也走直提(毫秒级),不能落回 OCR
    let extract = meta.extract;
    crate::ingest::enqueue_pdf_ocr(
        &app,
        crate::ingest::PdfOcrJob {
            id,
            name,
            src_path: src,
            pages,
            total,
            weights: meta.weights,
            extract,
        },
    );
    Ok(true)
}

/// 按需识别单页(翻到未识别页时前端触发;已识别/在途则静默跳过)。
#[tauri::command]
pub fn pdf_recognize_page(app: AppHandle, id: String, page: u32) {
    crate::ingest::recognize_single_page(&app, &id, page);
}

/// 渲染一页到 inbox 缓存文件(JPEG q90——编码比 PNG 快数倍,显示与重识别都够用)。
fn render_view_page_to_file(
    app: &AppHandle,
    id: &str,
    page: u32,
) -> Result<(PathBuf, u32, u32), String> {
    let state = app.state::<AppCtx>();
    let src = crate::ingest::PDF_STORE
        .lock()
        .unwrap()
        .get(id)
        .cloned()
        .ok_or("PDF 条目不存在")?;
    let (w, h, rgb) = crate::pdf::render_page_from_file(&src, page, crate::pdf::DPI_VIEW)?;
    let path = state.dirs.inbox.join(format!("{id}_p{page}.jpg"));
    let img = image::RgbImage::from_raw(w, h, rgb).ok_or("渲染数据无效")?;
    let f = std::fs::File::create(&path).map_err(|e| format!("保存失败: {e}"))?;
    let mut wtr = std::io::BufWriter::new(f);
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut wtr, 90);
    img.write_with_encoder(enc)
        .map_err(|e| format!("编码失败: {e}"))?;
    Ok((path, w, h))
}

lazy_static::lazy_static! {
    /// 预取中的 (条目, 页),防重复起线程
    static ref PREFETCH_INFLIGHT: std::sync::Mutex<std::collections::HashSet<(String, u32)>> =
        std::sync::Mutex::new(std::collections::HashSet::new());
}

/// 后台预渲染下一页:只写缓存文件,不注册令牌/不发事件——翻到该页时
/// pdf_render_page 走缓存快路径,线性翻页零等待。
fn prefetch_view_page(app: &AppHandle, id: &str, page: u32) {
    let count = crate::ingest::PDF_PAGES
        .lock()
        .unwrap()
        .get(id)
        .map(|m| m.count)
        .unwrap_or(0);
    if page >= count {
        return;
    }
    let state = app.state::<AppCtx>();
    if state.dirs.inbox.join(format!("{id}_p{page}.jpg")).exists() {
        return; // 已有缓存
    }
    if !PREFETCH_INFLIGHT
        .lock()
        .unwrap()
        .insert((id.to_string(), page))
    {
        return; // 已在预取
    }
    let app = app.clone();
    let id = id.to_string();
    std::thread::spawn(move || {
        let _ = render_view_page_to_file(&app, &id, page);
        PREFETCH_INFLIGHT.lock().unwrap().remove(&(id, page));
    });
}

/// 渲染 PDF 指定页为图像(通过 media:// 协议返回令牌)。
/// 快路径:该页已渲染过(inbox 缓存文件在)→ 只读尺寸+注册令牌,毫秒级;
/// 慢路径:渲染 + JPEG 编码,随后后台预取下一页。
#[tauri::command]
pub async fn pdf_render_page(
    app: AppHandle,
    id: String,
    page: u32,
    _dpi: Option<u16>,
) -> Result<crate::dto::PdfPageDto, String> {
    let app2 = app.clone();
    let id2 = id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppCtx>();
        // 快路径:jpg(新)或 png(旧会话遗留)缓存命中
        let jpg = state.dirs.inbox.join(format!("{id2}_p{page}.jpg"));
        let png = state.dirs.inbox.join(format!("{id2}_p{page}.png"));
        let (path, w, h, mime) = if jpg.exists() {
            let (w, h) = image::image_dimensions(&jpg).map_err(|e| format!("缓存读取失败: {e}"))?;
            (jpg, w, h, "image/jpeg")
        } else if png.exists() {
            let (w, h) = image::image_dimensions(&png).map_err(|e| format!("缓存读取失败: {e}"))?;
            (png, w, h, "image/png")
        } else {
            // 慢路径:渲染 + 编码落盘
            render_view_page_to_file(&app2, &id2, page).map(|(p, w, h)| (p, w, h, "image/jpeg"))?
        };
        let token = state.media.register(path.clone(), mime);
        let token2 = token.clone();
        {
            let mut items = state.items.write().unwrap();
            if let Some(item) = items.get_mut(&id2) {
                item.path = path;
                item.media_token = token;
                item.w = w;
                item.h = h;
            }
        }
        // 同步该页的识别结果(识别可能已完成;带页号,前端丢弃晚到的非当前页)
        let page_outcome = {
            let results = crate::ingest::PDF_RESULTS.lock().unwrap();
            results.get(&id2).and_then(|m| m.get(&page)).cloned()
        };
        let recognized = page_outcome.is_some();
        if let Some(outcome) = page_outcome {
            let _ = app2.emit("ocr://item-done", crate::dto::ItemDoneDto {
                id: id2.clone(),
                outcome,
                thumb_token: None,
                pdf_page: Some(page),
            });
        }
        // 预取下一页:翻页方向上的下一击就是缓存命中
        prefetch_view_page(&app2, &id2, page + 1);
        Ok(crate::dto::PdfPageDto { media_token: token2, w, h, page, recognized })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 对 PDF 指定范围执行批量识别:每页渲染→入库→进批量队列。
/// 页与页之间通过事件通知前端(复用 ocr://item-done)。
#[tauri::command]
pub async fn pdf_ocr_range(
    app: AppHandle,
    id: String,
    start_page: u32,
    end_page: u32,
) -> Result<(), String> {
    let src = crate::ingest::PDF_STORE
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("不是 PDF 条目或已释放")?;
    let bytes = std::fs::read(&src).map_err(|e| format!("读取 PDF 失败: {e}"))?;
    let total = crate::pdf::page_count(&bytes)?;
    let start = start_page.min(total.saturating_sub(1));
    let end = end_page.min(total.saturating_sub(1));
    if start > end {
        return Err("页码范围无效".into());
    }

    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppCtx>();
        let mut ids = Vec::new();
        for page in start..=end {
            let (w, h, rgb) = match crate::pdf::render_page(&bytes, page, crate::pdf::DPI_OCR) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[pdf] 第 {} 页渲染失败: {e}", page + 1);
                    continue;
                }
            };
            let img = match qppocr::rgb_from_bytes(w, h, rgb) {
                Ok(i) => i,
                Err(e) => {
                    eprintln!("[pdf] 第 {} 页数据无效: {e}", page + 1);
                    continue;
                }
            };

            // 每页 PNG 落盘,path 指向 PNG(OCR 走正常解码管线)
            let pid = uuid::Uuid::new_v4().simple().to_string();
            let parent_name = {
                let items = state.items.read().unwrap();
                items
                    .get(&id)
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| "PDF".into())
            };
            let png_path = state.dirs.inbox.join(format!("{pid}_p{page}.png"));
            if let Some(png_img) = image::RgbImage::from_raw(w, h, img.data.clone()) {
                if let Err(e) = png_img.save_with_format(&png_path, image::ImageFormat::Png) {
                    eprintln!("[pdf] p{} save: {e}", page + 1);
                    continue;
                }
            } else {
                continue;
            }
            let media_token = state.media.register(png_path.clone(), "image/png");
            let display_name = format!("{} · 第{}页", parent_name, page + 1);

            // 缩略图
            let thumb_path = crate::thumb::make_thumb_from_rgb(w, h, &img.data, &pid, &state.dirs.thumbs)
                .unwrap_or_default();
            let thumb_token = if thumb_path.as_os_str().len() > 0 {
                state.media.register(thumb_path, "image/jpeg")
            } else {
                String::new()
            };

            let item = crate::ingest::ImageItem {
                id: pid.clone(),
                name: display_name,
                path: std::path::PathBuf::from(&id), // 指向父 PDF
                w,
                h,
                origin: "pdf-page".into(),
                added_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                media_token: thumb_token.clone(), // 用缩略图令牌做显示
                thumb_token,
                can_extract: false,
            };
            state.register_item(item);

            ids.push(pid);
        }

        // 进批量队列
        if !ids.is_empty() {
            let _ = tauri::async_runtime::block_on(async {
                // 直接调 batch 逻辑:设置状态,逐条识别
                for pid in &ids {
                    let _ = app2.emit("ocr://item-status", crate::dto::ItemStatusDto {
                        id: pid.clone(),
                        phase: "queued".into(),
                    });
                }
                // 复用 run_item_blocking
                for pid in &ids {
                    let _ = app2.emit("ocr://item-status", crate::dto::ItemStatusDto {
                        id: pid.clone(),
                        phase: "running".into(),
                    });
                    let _ = crate::batch::run_item_blocking(&app2, pid);
                }
                let _ = app2.emit("batch://done", ());
            });
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e)
}

/// 行集 → Markdown(版式感知):行高相对聚类判标题层级,垂直间隙突变分段落。
/// OCR 只有几何信息(无字体/加粗),这是诚实能做到的「保留格式」。
fn lines_to_markdown(lines: &[crate::dto::TextLineDto], base_level: usize) -> String {
    // 每行 (高度, 中心y)
    let metrics: Vec<(f32, f32)> = lines
        .iter()
        .map(|l| {
            let ys = l.pts.map(|p| p[1]);
            let hmax = ys.iter().cloned().fold(f32::MIN, f32::max);
            let hmin = ys.iter().cloned().fold(f32::MAX, f32::min);
            ((hmax - hmin).max(1.0), (hmax + hmin) / 2.0)
        })
        .collect();
    let mut hs: Vec<f32> = metrics.iter().map(|(h, _)| *h).collect();
    hs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med_h = hs.get(hs.len() / 2).copied().unwrap_or(1.0);

    let mut out = String::new();
    let mut prev: Option<(f32, f32)> = None; // (中心y, 行高)
    for (l, (h, cy)) in lines.iter().zip(metrics) {
        let text = l.text.trim();
        if text.is_empty() {
            continue;
        }
        // 段落:与上一行垂直间隙 > 1.8×行高 → 空行分隔
        if let Some((p_cy, p_h)) = prev {
            if cy - p_cy > p_h * 1.8 {
                out.push('\n');
            }
        }
        // 标题层级:行高 ≥1.5×中位 → 大标题,≥1.2× → 次级
        let level = if h >= med_h * 1.5 {
            base_level
        } else if h >= med_h * 1.2 {
            base_level + 1
        } else {
            0
        };
        if level > 0 {
            for _ in 0..level {
                out.push('#');
            }
            out.push(' ');
        }
        out.push_str(text);
        out.push('\n');
        prev = Some((cy, h));
    }
    out
}

/// 合并 PDF 识别结果导出(txt/json/md)。数据源是 PDF_RESULTS(按页有序)——
/// 不再依赖旧管线的逐页条目。
#[tauri::command]
pub fn pdf_export_merged(
    state: State<AppCtx>,
    parent_id: String,
    fmt: String,
) -> Result<String, String> {
    let results = crate::ingest::PDF_RESULTS
        .lock()
        .unwrap()
        .get(&parent_id)
        .cloned()
        .ok_or("PDF 条目不存在")?;
    if results.is_empty() {
        return Err("尚无识别结果(识别完成后可导出)".into());
    }
    let parent_name = state
        .items
        .read()
        .unwrap()
        .get(&parent_id)
        .map(|i| i.name.clone())
        .unwrap_or_else(|| "PDF".into());

    match fmt.as_str() {
        "txt" => {
            let mut out = format!("# {parent_name} — 全文\n\n");
            for (page, outcome) in &results {
                out.push_str(&format!("--- 第 {} 页 ---\n", page + 1));
                if let Some(r) = &outcome.result {
                    for line in &r.lines {
                        if !line.text.is_empty() {
                            out.push_str(line.text.trim_end());
                            out.push('\n');
                        }
                    }
                }
                out.push('\n');
            }
            Ok(out)
        }
        "md" => {
            let mut out = format!("# {parent_name}\n");
            for (page, outcome) in &results {
                out.push_str(&format!("\n---\n\n## 第 {} 页\n\n", page + 1));
                if let Some(r) = &outcome.result {
                    out.push_str(&lines_to_markdown(&r.lines, 3));
                }
            }
            Ok(out)
        }
        "json" => {
            let page_results: Vec<serde_json::Value> = results
                .iter()
                .map(|(page, outcome)| {
                    let r = outcome.result.as_ref();
                    serde_json::json!({
                        "page": page + 1,
                        "text": r.map(|r| r.lines.iter()
                            .filter(|l| !l.text.is_empty())
                            .map(|l| l.text.trim())
                            .collect::<Vec<_>>()
                            .join("\n")).unwrap_or_default(),
                        "lineCount": r.map(|r| r.lines.len()).unwrap_or(0),
                        "totalMs": r.map(|r| r.timings.total_ms).unwrap_or(0.0),
                    })
                })
                .collect();
            let json = serde_json::json!({
                "source": parent_name,
                "pages": page_results,
            });
            serde_json::to_string_pretty(&json).map_err(|e| e.to_string())
        }
        _ => Err("不支持的格式".into()),
    }
}

/// 设备实测对比:合成图在 CPU/GPU 各跑 热身1+5轮 取中位(当前档位/线程数)。
/// GPU 失败时 gpu_error 携带引擎报错(loader 缺失/驱动低于 Vulkan 1.4 等,
/// 引擎错误串自带设备清单)。
#[tauri::command]
pub async fn device_benchmark(app: AppHandle) -> Result<crate::dto::DeviceBenchDto, String> {
    let (spec, dir, threads) = {
        let state = app.state::<AppCtx>();
        (
            state.engine.spec(),
            state.engine.models_dir(),
            state.engine.threads,
        )
    };
    tauri::async_runtime::spawn_blocking(move || {
        // 合成测试图:白底三黑杠(与 selftest 同构,结果可复现)
        let (w, h) = (960u32, 540u32);
        let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([255u8, 255, 255]));
        for (y0, hh) in [(80u32, 28u32), (240, 24), (380, 20)] {
            for y in y0..y0 + hh {
                for x in 80..880 {
                    img.put_pixel(x, y, image::Rgb([10, 10, 10]));
                }
            }
        }
        let image =
            qppocr::rgb_from_bytes(w, h, img.into_raw()).map_err(|e| e.to_string())?;

        let run_side = |device: qppocr::DeviceChoice| -> Result<(f64, u32), String> {
            let mut s = spec.clone();
            s.device = device;
            let engine = crate::engine::build_engine(&s, threads, &dir)?;
            let _ = engine.run(&image); // 热身(含 GPU 首见形状的计划构建)
            let mut times = Vec::new();
            let mut last = 0u32;
            for _ in 0..5 {
                let t0 = std::time::Instant::now();
                let r = engine.run(&image).map_err(|e| e.to_string())?;
                times.push(t0.elapsed().as_secs_f64() * 1000.0);
                last = r.lines.len() as u32;
            }
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            Ok((times[times.len() / 2], last))
        };

        let (cpu_ms, cpu_lines) =
            run_side(qppocr::DeviceChoice::Cpu).map_err(|e| format!("CPU 侧失败:{e}"))?;
        match run_side(qppocr::DeviceChoice::gpu()) {
            Ok((gpu_ms, gpu_lines)) => Ok(crate::dto::DeviceBenchDto {
                cpu_ms,
                gpu_ms,
                cpu_lines,
                gpu_lines,
                gpu_error: None,
            }),
            Err(e) => Ok(crate::dto::DeviceBenchDto {
                cpu_ms,
                gpu_ms: 0.0,
                cpu_lines,
                gpu_lines: 0,
                gpu_error: Some(e),
            }),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
/// PDF 文本直提(跳过 OCR):从文本层提取全部页,毫秒级返回。
/// 仅对数字原生 PDF 有效(有文本层)。
#[tauri::command]
pub async fn pdf_extract_all(
    app: AppHandle,
    id: String,
) -> Result<Vec<crate::dto::PdfExtractedPageDto>, String> {
    let src = crate::ingest::PDF_STORE
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("不是 PDF 条目或已释放")?;
    let bytes = std::fs::read(&src).map_err(|e| format!("读取 PDF 失败: {e}"))?;

    let total = crate::pdf::page_count(&bytes)?;
    let has_text = crate::pdf::has_text_layer(&bytes);
    if !has_text {
        return Err("此 PDF 没有文本层(扫描件),请使用 OCR 识别".into());
    }

    let state = app.state::<AppCtx>();
    let parent_name = {
        let items = state.items.read().unwrap();
        items
            .get(&id)
            .map(|i| i.name.clone())
            .unwrap_or_else(|| "PDF".into())
    };

    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppCtx>();
        let mut results = Vec::new();

        for page in 0..total {
            let lines = crate::pdf::extract_text_lines(&bytes, page, crate::pdf::DPI_VIEW)
                .unwrap_or_default();

            let text: Vec<String> = lines
                .iter()
                .map(|(t, _, _)| t.clone())
                .collect();

            let full_text = text.join("
");
            results.push(crate::dto::PdfExtractedPageDto {
                page: page + 1,
                text: full_text,
                line_count: lines.len() as u32,
            });
        }

        // 同时写入识别记录(每页一个条目,和 OCR 批量结果一致)
        for (page_idx, result) in results.iter().enumerate() {
            let pid = uuid::Uuid::new_v4().simple().to_string();
            let display_name = format!("{} · 第{}页(直提)", parent_name, page_idx + 1);

            let thumb_path = {
                // 渲染缩略图
                match crate::pdf::render_page(&bytes, page_idx as u32, crate::pdf::DPI_THUMB) {
                    Ok((w, h, rgb)) => {
                        crate::thumb::make_thumb_from_rgb(w, h, &rgb, &pid, &state.dirs.thumbs)
                            .unwrap_or_default()
                    }
                    Err(_) => std::path::PathBuf::new(),
                }
            };
            let thumb_token = if thumb_path.as_os_str().len() > 0 {
                state.media.register(thumb_path, "image/jpeg")
            } else {
                String::new()
            };

            // 构造 OcrResultDto(置信度全部 1.0,无 timings)
            let text_lines: Vec<crate::dto::TextLineDto> = crate::pdf::extract_text_lines(
                &bytes,
                page_idx as u32,
                crate::pdf::DPI_VIEW,
            )
            .unwrap_or_default()
            .into_iter()
            .map(|(t, c, pts)| crate::dto::TextLineDto {
                text: t,
                confidence: c,
                rotation: 0,
                pts,
                chars: vec![],
                retried: false,
            })
            .collect();

            let outcome = crate::dto::OcrOutcomeDto {
                ok: true,
                error: None,
                result: Some(crate::dto::OcrResultDto {
                    lines: text_lines,
                    work_w: 0,
                    work_h: 0,
                    num_boxes: 0,
                    num_merged: 0,
                    num_decluttered: 0,
                    num_det_retried: 0,
                    num_flipped: 0,
                    num_unread: 0,
                    extracted: true,
                    timings: crate::dto::TimingsDto {
                        det_pre_ms: 0.0, det_infer_ms: 0.0, det_post_ms: 0.0,
                        crop_ms: 0.0, cls_ms: 0.0, rec_pre_ms: 0.0, rec_infer_ms: 0.0,
                        rec_post_ms: 0.0, total_ms: 0.0,
                    },
                }),
            };

            // 渲染全尺寸图供画布显示
            let (vw, vh, _vrgb) = crate::pdf::render_page(
                &bytes, page_idx as u32, crate::pdf::DPI_VIEW,
            )
            .unwrap_or((0, 0, vec![]));

            let item = crate::ingest::ImageItem {
                id: pid.clone(),
                name: display_name,
                path: std::path::PathBuf::from(&id),
                w: vw,
                h: vh,
                origin: "pdf-page".into(),
                added_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                media_token: thumb_token.clone(),
                thumb_token,
                can_extract: false,
            };
            state.register_item(item);
            crate::batch::finalize(&app2, &pid, &outcome, None, None);
        }

        let _ = app2.emit("batch://done", ());
        Ok(results)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e: String| e)
}
