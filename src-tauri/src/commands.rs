//! 全部 Tauri 命令。自定义命令不受 ACL 约束（插件/core 命令才走 capabilities）。

use crate::dto::{
    EngineStatusDto, ImageItemDto, InitInfoDto, ItemOutcomeDto, OcrOutcomeDto, ShotMonitorDto,
};
use crate::settings::{parse_preset, parse_tier, Settings};
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

#[tauri::command]
pub fn engine_status(state: State<AppCtx>) -> EngineStatusDto {
    state.engine.status()
}

#[tauri::command]
pub async fn pick_images(app: AppHandle) -> Result<Vec<ImageItemDto>, String> {
    let paths = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .add_filter("图片和 PDF", &["png", "jpg", "jpeg", "bmp", "pdf"])
            .add_filter("PDF 文档", &["pdf"])
            .pick_files()
    })
    .await
    .map_err(|e| e.to_string())?
    .unwrap_or_default();
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

    // 大批量 + 档位允许 → 进程分治(每 worker 独立引擎,真正吃满核)
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let procs = crate::batch::worker_count(state.engine.spec().tier, cores);
    if tasks.len() >= crate::batch::PROC_BATCH_MIN && procs > 0 {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::batch::spawn_proc_batch(app, tasks);
        })
        .await
        .map_err(|e| e.to_string())?;
        return Ok(());
    }

    // 小批量:进程内共享引擎并发(并发 run 在算子层部分重叠)
    let user_conc = state.settings.read().unwrap().batch_concurrency;
    let conc = if user_conc == 0 {
        // 自动:tiny 引擎轻,4 并发收益明显;small/medium 引擎重,2 为甜点
        match state.engine.spec().tier {
            qppocr::Tier::Tiny => 4,
            _ => 2,
        }
    } else {
        user_conc
    };
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
    // 缩略图令牌不持久化,列表时按需注册(命中缓存则零成本)
    for e in &mut list {
        let thumb = state.dirs.thumbs.join(format!("{}.jpg", e.id));
        if thumb.exists() {
            e.thumb_token = Some(state.media.register(thumb, "image/jpeg"));
        }
    }
    list
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
    let ext = if fmt == "json" { "json" } else { "txt" };
    let path = tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .add_filter("文件", &[ext])
            .set_file_name(&default_name)
            .save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = path else {
        return Err("已取消".into());
    };
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

/// 获取 PDF 条目的页数与当前页。
#[tauri::command]
pub fn pdf_page_info(id: String) -> Option<(u32, u32)> {
    crate::ingest::PDF_PAGES.lock().unwrap().get(&id).copied()
}

/// 渲染 PDF 指定页为图像(通过 media:// 协议返回令牌)。
#[tauri::command]
pub async fn pdf_render_page(
    app: AppHandle,
    id: String,
    page: u32,
    _dpi: Option<u16>,
) -> Result<crate::dto::PdfPageDto, String> {
    // 翻页:从 PDF 字节渲染指定页 → 落盘 PNG → 更新条目 path/媒体令牌 → 前端画布刷新
    let app2 = app.clone();
    let id2 = id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = crate::ingest::PDF_STORE
            .lock().unwrap()
            .get(&id2).cloned()
            .ok_or("PDF 条目不存在")?;
        let state = app2.state::<AppCtx>();
        let (w, h, rgb) = crate::pdf::render_page(&bytes, page, crate::pdf::DPI_VIEW)?;
        let png_path = state.dirs.inbox.join(format!("{id2}_p{page}.png"));
        let img = image::RgbImage::from_raw(w, h, rgb)
            .ok_or("渲染数据无效")?;
        img.save_with_format(&png_path, image::ImageFormat::Png)
            .map_err(|e| format!("保存失败: {e}"))?;
        let token = state.media.register(png_path.clone(), "image/png");
        let token2 = token.clone();
        {
            let mut items = state.items.write().unwrap();
            if let Some(item) = items.get_mut(&id2) {
                item.path = png_path;
                item.media_token = token;
                item.w = w;
                item.h = h;
            }
        }
        // 同步该页的识别结果(后台 OCR 可能已完成)
        let page_outcome = {
            let results = crate::ingest::PDF_RESULTS.lock().unwrap();
            results.get(&id2)
                .and_then(|v| v.iter().find(|(p, _)| *p == page))
                .map(|(_, o)| o.clone())
        };
        if let Some(outcome) = page_outcome {
            let st = app2.state::<AppCtx>();
            let mut items = st.items.write().unwrap();
            if let Some(item) = items.get_mut(&id2) {
                // 更新条目的 outcome(不通过 finalize,不走历史)
                // 直接修改条目上的结果
            }
            drop(items);
            // 发事件让前端更新结果面板
            let _ = app2.emit("ocr://item-done", crate::dto::ItemDoneDto {
                id: id2.clone(),
                outcome,
                thumb_token: None,
            });
        }
        Ok(crate::dto::PdfPageDto { media_token: token2, w, h, page })
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
    let bytes = crate::ingest::PDF_STORE
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("不是 PDF 条目或已释放")?;
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

/// 合并 PDF 识别结果导出(按页序拼接,带页码分隔)。
#[tauri::command]
pub fn pdf_export_merged(
    state: State<AppCtx>,
    parent_id: String,
    fmt: String,
) -> Result<String, String> {
    let items = state.items.read().unwrap();
    let mut pages: Vec<(&String, &crate::ingest::ImageItem)> = items
        .iter()
        .filter(|(_, it)| it.origin == "pdf-page" && it.path.to_str() == Some(parent_id.as_str()))
        .map(|(k, v)| (k, v))
        .collect();
    if pages.is_empty() {
        return Err("没有已识别的页面".into());
    }
    let order = state.order.read().unwrap();
    pages.sort_by_key(|(id, _)| order.iter().position(|x| x == *id).unwrap_or(usize::MAX));

    let parent_name = items
        .get(&parent_id)
        .map(|i| i.name.clone())
        .unwrap_or_else(|| "PDF".into());

    match fmt.as_str() {
        "txt" => {
            let mut out = String::new();
            out.push_str(&format!("# {} — OCR 全文

", parent_name));
            for (pid, it) in &pages {
                if let Some(entry) = state.history.get(pid) {
                    let page_label = it.name.rsplit('·').next().unwrap_or("").trim();
                    out.push_str(&format!("--- {} ---
", page_label));
                    if let Some(r) = &entry.outcome.result {
                        for line in &r.lines {
                            if !line.text.is_empty() {
                                out.push_str(&line.text);
                                out.push('\n');
                            }
                        }
                    }
                    out.push('\n');
                }
            }
            Ok(out)
        }
        "json" => {
            let mut page_results: Vec<serde_json::Value> = Vec::new();
            for (pid, it) in &pages {
                if let Some(entry) = state.history.get(pid) {
                    if let Some(r) = &entry.outcome.result {
                        let page_label = it.name.rsplit('·').next().unwrap_or("").trim();
                        page_results.push(serde_json::json!({
                            "page": page_label,
                            "text": r.lines.iter()
                                .filter(|l| !l.text.is_empty())
                                .map(|l| l.text.as_str())
                                .collect::<Vec<_>>()
                                .join("
"),
                            "lineCount": r.lines.len(),
                            "totalMs": r.timings.total_ms,
                        }));
                    }
                }
            }
            let json = serde_json::json!({
                "source": parent_name,
                "pages": page_results,
            });
            serde_json::to_string_pretty(&json).map_err(|e| e.to_string())
        }
        _ => Err("不支持的格式".into()),
    }
}

/// PDF 文本直提(跳过 OCR):从文本层提取全部页,毫秒级返回。
/// 仅对数字原生 PDF 有效(有文本层)。
#[tauri::command]
pub async fn pdf_extract_all(
    app: AppHandle,
    id: String,
) -> Result<Vec<crate::dto::PdfExtractedPageDto>, String> {
    let bytes = crate::ingest::PDF_STORE
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or("不是 PDF 条目或已释放")?;

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
            crate::batch::finalize(&app2, &pid, &outcome, None);
        }

        let _ = app2.emit("batch://done", ());
        Ok(results)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e: String| e)
}
