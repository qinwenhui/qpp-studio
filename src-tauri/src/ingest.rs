//! 统一入库：图片 / PDF / 剪贴板 / 截图 → ImageItem。
//!
//! PDF 策略:入库只做 第0页渲染+页尺寸探测,识别任务入队;长驻队列线程
//! 逐本处理(多本排队),每本按页数和硬件优化表选 模式:≥8 页且档位允许
//! → K 个 worker 进程按页分治(渲染+OCR 都在 worker 里,pdfium 全局锁
//! 决定了进程内并行无意义);否则主进程串行逐页。逐页推 pdf://page-done
//! (舰队乱序到达,done 为完成计数),结束推 pdf://ocr-done。

use crate::dto::ImageItemDto;
use crate::dto::OcrOutcomeDto;
use crate::media::mime_for;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use tauri::{AppHandle, Emitter, Manager};

lazy_static::lazy_static! {
    /// PDF 原始字节(元信息查询用)
    pub static ref PDF_STORE: Mutex<HashMap<String, Vec<u8>>> =
        Mutex::new(HashMap::new());
    /// PDF 元信息:id → (总页数, 当前页)
    pub static ref PDF_PAGES: Mutex<HashMap<String, (u32, u32)>> =
        Mutex::new(HashMap::new());
    /// PDF 每页识别结果(BTreeMap:舰队乱序插入 + 单页快查 + 天然有序)
    pub static ref PDF_RESULTS: Mutex<HashMap<String, BTreeMap<u32, OcrOutcomeDto>>> =
        Mutex::new(HashMap::new());
}

/// 舰队分治的页数门槛:低于此 worker 启动+模型加载不划算。
pub const PDF_FLEET_MIN: usize = 8;

// ---- PDF 识别队列(逐本处理,与图片批量互斥) ----

pub struct PdfOcrJob {
    pub id: String,
    pub name: String,
    /// 原始文件路径(worker 读它;不可读时从 PDF_STORE 落盘兜底)
    pub src_path: PathBuf,
    pub pages: u32,
    /// 每页像素权重(LPT 均衡分块,page_sizes 探测)
    pub weights: Vec<u64>,
}

pub struct PdfOcrState {
    queue: Mutex<VecDeque<PdfOcrJob>>,
    cv: Condvar,
    /// 舰队运行中 → 图片批量 batch_start 返回 busy(互斥共用 worker 预算)
    pub fleet_active: AtomicBool,
}

impl Default for PdfOcrState {
    fn default() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            fleet_active: AtomicBool::new(false),
        }
    }
}

/// 条目删除后的 PDF 资源清理:三 store(修字节常驻泄漏)+ 磁盘渲染页/
/// 兜底 PDF/缩略图(best-effort)。队列中排着的同 id 任务跑起来时会被
/// 「条目已删」检查自然短路,舰队 sink 检测到会杀舰队止损。
pub fn cleanup_pdf(state: &crate::AppCtx, id: &str) {
    PDF_STORE.lock().unwrap().remove(id);
    PDF_PAGES.lock().unwrap().remove(id);
    PDF_RESULTS.lock().unwrap().remove(id);
    let prefix = format!("{id}_p");
    if let Ok(rd) = std::fs::read_dir(&state.dirs.inbox) {
        for e in rd.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let is_page_cache = name.starts_with(&prefix)
                && (name.ends_with(".png") || name.ends_with(".jpg"));
            if is_page_cache {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let _ = std::fs::remove_file(state.dirs.inbox.join(format!("{id}.pdf")));
    let _ = std::fs::remove_file(state.dirs.thumbs.join(format!("{id}.jpg")));
}

/// 入队一本 PDF 的后台识别。
pub fn enqueue_pdf_ocr(app: &AppHandle, job: PdfOcrJob) {
    let state = app.state::<crate::AppCtx>();
    let mut q = state.pdf.queue.lock().unwrap();
    q.push_back(job);
    drop(q);
    state.pdf.cv.notify_one();
}

/// 长驻队列线程:空闲挂起(Condvar 零空转),逐本处理;图片批量优先,让路等待。
pub fn spawn_pdf_queue(app: AppHandle) {
    std::thread::spawn(move || loop {
        // 等队列非空
        let job = {
            let state = app.state::<crate::AppCtx>();
            let mut q = state.pdf.queue.lock().unwrap();
            loop {
                if let Some(job) = q.pop_front() {
                    break job;
                }
                q = state.pdf.cv.wait(q).unwrap();
            }
        };
        // 图片批量优先:等它跑完再开舰队(共用 worker 预算)
        while app.state::<crate::AppCtx>().batch.running.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        run_pdf_job(&app, job);
    });
}

/// 处理一本 PDF:选模式(舰队/串行)→ 逐页推结果 → 收尾 ocr-done。
fn run_pdf_job(app: &AppHandle, job: PdfOcrJob) {
    let state = app.state::<crate::AppCtx>();
    let started = std::time::Instant::now();
    // 复用图片批量的取消旗标(此刻图片批量必不在跑,复位安全;
    // 条目删除/批量取消允许中断本任务)
    state.batch.cancel.store(false, Ordering::SeqCst);

    let workers_override = state.settings.read().unwrap().workers_override;
    let plan = crate::hw::plan(&state.hw, state.engine.spec().tier, workers_override);
    // 自测/基准:强制小文档也走舰队路径
    let force = std::env::var("QPP_PDF_FLEET_FORCE").is_ok();
    let use_fleet = plan.workers > 0 && (job.pages as usize >= PDF_FLEET_MIN || force);

    if use_fleet {
        // 先立旗再查 running,与 batch_start 的先 swap 再查 fleet_active 对称,无竞态窗口
        state.pdf.fleet_active.store(true, Ordering::SeqCst);
        if state.batch.running.load(Ordering::SeqCst) {
            // 图片批量恰好插进来了:退回队首稍后重试
            state.pdf.fleet_active.store(false, Ordering::SeqCst);
            let mut q = state.pdf.queue.lock().unwrap();
            q.push_front(job);
            drop(q);
            std::thread::sleep(std::time::Duration::from_millis(150));
            return;
        }
        run_pdf_fleet(app, &job, &plan);
        state.pdf.fleet_active.store(false, Ordering::SeqCst);
    } else {
        serial_pdf_ocr(app, &job);
    }

    let _ = app.emit(
        "pdf://ocr-done",
        serde_json::json!({
            "id": job.id,
            "total": job.pages,
            "name": job.name,
            "elapsedMs": started.elapsed().as_millis() as u64,
        }),
    );
}

/// 逐页推送结果 + 页0 落历史(舰队 sink 与串行共用)。
fn emit_page(
    app: &AppHandle,
    id: &str,
    page: u32,
    outcome: &OcrOutcomeDto,
    done: usize,
    total: u32,
) {
    let _ = app.emit(
        "pdf://page-done",
        serde_json::json!({
            "id": id,
            "page": page,
            "outcome": outcome,
            "done": done,
            "total": total,
        }),
    );
    if page == 0 {
        // 第 0 页结果也落到父条目上(列表里立即可见,与旧串行行为一致)
        crate::batch::finalize(app, id, outcome, None, Some(page));
    }
}

/// 舰队模式:K 个 worker 按页分治(渲染+OCR 都在 worker)。
/// pub(crate):selftest 基准直接调用(不走队列,精确计时)。
pub(crate) fn run_pdf_fleet(app: &AppHandle, job: &PdfOcrJob, plan: &crate::hw::ParallelPlan) {
    let state = app.state::<crate::AppCtx>();
    let spec = state.engine.spec();
    let models_dir = state.engine.models_dir();

    // worker 数还受页数约束:每 worker 至少 2 页,摊薄引擎构建成本
    let k = plan.workers.min((job.pages as usize).div_ceil(2)).max(1);

    // worker 读源文件:原路径优先,不可读则 PDF_STORE 字节落盘兜底
    let src_readable = std::fs::metadata(&job.src_path)
        .map(|m| m.is_file())
        .unwrap_or(false);
    let src = if src_readable {
        job.src_path.clone()
    } else {
        let bytes = PDF_STORE.lock().unwrap().get(&job.id).cloned();
        match bytes {
            Some(b) => {
                let p = state.dirs.inbox.join(format!("{}.pdf", job.id));
                match std::fs::write(&p, &b) {
                    Ok(()) => p,
                    Err(_) => {
                        serial_pdf_ocr(app, job);
                        return;
                    }
                }
            }
            None => {
                serial_pdf_ocr(app, job);
                return;
            }
        }
    };

    let done = Arc::new(AtomicUsize::new(0));
    let parent = job.id.clone();
    let cancel = state.batch.cancel.clone();

    // PDF sink:计数、入库、推事件;条目已删则杀舰队止损
    let sink = |ev: crate::batch::WorkerEvent| {
        let Some(page) = ev.page else { return };
        if !PDF_PAGES.lock().unwrap().contains_key(&parent) {
            cancel.store(true, Ordering::SeqCst);
            return;
        }
        PDF_RESULTS
            .lock()
            .unwrap()
            .entry(parent.clone())
            .or_default()
            .insert(page, ev.outcome.clone());
        let d = done.fetch_add(1, Ordering::SeqCst) + 1;
        emit_page(app, &parent, page, &ev.outcome, d, job.pages);
    };

    let req_base = serde_json::json!({
        "modelsDir": models_dir.to_string_lossy(),
        "tier": crate::settings::tier_str(spec.tier),
        "preset": crate::settings::preset_str(spec.preset),
        "threads": plan.threads_each.max(1),
        "orientation": spec.orientation,
        "enhanceContrast": spec.enhance_contrast,
        "upscale": spec.upscale,
    });
    let pdf_spec = serde_json::json!({
        "path": src.to_string_lossy(),
        "dpi": crate::pdf::DPI_OCR,
    });
    let items: Vec<crate::batch::FleetItem> = (0..job.pages)
        .map(|p| crate::batch::FleetItem {
            id: job.id.clone(),
            page: Some(p),
            path: None,
            weight: job.weights.get(p as usize).copied().unwrap_or(1),
        })
        .collect();

    crate::batch::run_fleet(req_base, Some(pdf_spec), items, k, cancel.clone(), &sink);
}

/// 串行模式(页数少 / medium / 舰队不可用):主进程逐页渲染 + 共享引擎。
/// 渲染/解码失败产出错误回执(不再静默跳过,否则前端进度永远悬死)。
fn serial_pdf_ocr(app: &AppHandle, job: &PdfOcrJob) {
    let state = app.state::<crate::AppCtx>();
    let err_all = |msg: &str| {
        let mut done = 0usize;
        for page in 0..job.pages {
            done += 1;
            let out = crate::dto::outcome_err(msg.to_string());
            PDF_RESULTS
                .lock()
                .unwrap()
                .entry(job.id.clone())
                .or_default()
                .insert(page, out.clone());
            emit_page(app, &job.id, page, &out, done, job.pages);
        }
    };

    let Some(bytes) = PDF_STORE.lock().unwrap().get(&job.id).cloned() else {
        err_all("PDF 数据已失效");
        return;
    };
    let doc = match crate::pdf::load_doc(&bytes) {
        Ok(d) => d,
        Err(e) => {
            err_all(&e);
            return;
        }
    };
    // 引擎可能还在启动构建(启动头几秒拖入的场景):有界等待而不是整本报错
    let mut engine = state.engine.current();
    for _ in 0..150 {
        if engine.is_some() {
            break;
        }
        if !PDF_PAGES.lock().unwrap().contains_key(&job.id) {
            return; // 等待期间条目已删
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
        engine = state.engine.current();
    }
    let Some(engine) = engine else {
        err_all("引擎未就绪");
        return;
    };

    let mut done = 0usize;
    for page in 0..job.pages {
        if state.batch.cancel.load(Ordering::Relaxed) {
            break;
        }
        if !PDF_PAGES.lock().unwrap().contains_key(&job.id) {
            break; // 条目已删
        }
        let outcome = match crate::pdf::render_doc_page(&doc, page, crate::pdf::DPI_OCR)
            .and_then(|(w, h, rgb)| {
                qppocr::rgb_from_bytes(w, h, rgb).map_err(|e| e.to_string())
            }) {
            Ok(img) => {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    engine.run(&img)
                }));
                crate::dto::outcome_from(match r {
                    Ok(Ok(v)) => Ok(v),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(qppocr::Error::Image("引擎内部错误".into())),
                })
            }
            Err(e) => crate::dto::outcome_err(e),
        };
        done += 1;
        PDF_RESULTS
            .lock()
            .unwrap()
            .entry(job.id.clone())
            .or_default()
            .insert(page, outcome.clone());
        emit_page(app, &job.id, page, &outcome, done, job.pages);
    }
}

#[derive(Clone)]
pub struct ImageItem {
    pub id: String,
    pub name: String,
    /// 可显示/可解码的图片路径
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
    pub origin: String,
    pub added_at: u64,
    pub media_token: String,
    pub thumb_token: String,
    pub can_extract: bool,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn valid_ext(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "bmp" | "pdf")
    )
}

/// 渲染 PDF 某页 → PNG 落盘 → 返回 (路径, 宽, 高, 媒体令牌, 缩略图令牌)
fn render_pdf_page_to_item(
    app: &AppHandle,
    bytes: &[u8],
    page: u32,
    item_id: &str,
) -> Result<(PathBuf, u32, u32, String, String), String> {
    let state = app.state::<crate::AppCtx>();
    let (w, h, rgb) = crate::pdf::render_page(bytes, page, crate::pdf::DPI_VIEW)?;
    let png_path = state.dirs.inbox.join(format!("{item_id}_p{page}.png"));
    let img = image::RgbImage::from_raw(w, h, rgb)
        .ok_or("渲染数据无效")?;
    img.save_with_format(&png_path, image::ImageFormat::Png)
        .map_err(|e| format!("保存页面失败: {e}"))?;
    let media_token = state.media.register(png_path.clone(), "image/png");

    // 缩略图
    let thumb_token = match crate::pdf::render_page(bytes, page, crate::pdf::DPI_THUMB) {
        Ok((tw, th, trgb)) => {
            let tp = state.dirs.thumbs.join(format!("{item_id}.jpg"));
            if let Some(ti) = image::RgbImage::from_raw(tw, th, trgb) {
                if ti.save_with_format(&tp, image::ImageFormat::Jpeg).is_ok() {
                    state.media.register(tp, "image/jpeg")
                } else { String::new() }
            } else { String::new() }
        }
        Err(_) => String::new(),
    };
    Ok((png_path, w, h, media_token, thumb_token))
}

pub fn ingest_file(app: &AppHandle, path: &Path, origin: &str) -> Result<ImageItemDto, String> {
    if !path.is_file() {
        return Err(format!("文件不存在: {}", path.display()));
    }
    if !valid_ext(path) {
        return Err(format!("不支持的格式: {}", path.display()));
    }

    let state = app.state::<crate::AppCtx>();
    let is_pdf = path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false);

    if is_pdf {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let bytes = std::fs::read(path).map_err(|e| format!("读取 PDF 失败: {e}"))?;
        // 一次探测:页数 + 每页点尺寸(不渲染像素,舰队分块权重用)
        let sizes = crate::pdf::page_sizes(&bytes)?;
        let count = sizes.len() as u32;
        if count == 0 {
            return Err("PDF 没有任何页面".into());
        }
        let can_extract = crate::pdf::has_text_layer(&bytes);
        let file_name = path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "PDF".into());
        let weights: Vec<u64> = sizes
            .iter()
            .map(|&pt| crate::pdf::page_weight(pt, crate::pdf::DPI_OCR))
            .collect();

        // 存元信息
        PDF_STORE.lock().unwrap().insert(id.clone(), bytes.clone());
        PDF_PAGES.lock().unwrap().insert(id.clone(), (count, 0));
        PDF_RESULTS.lock().unwrap().insert(id.clone(), BTreeMap::new());

        // 同步渲染第 1 页
        let (png_path, w, h, media_token, thumb_token) =
            render_pdf_page_to_item(app, &bytes, 0, &id)?;
        let item = ImageItem {
            id: id.clone(),
            name: file_name.clone(),
            path: png_path,
            w, h,
            origin: "pdf".into(),
            added_at: now_ms(),
            media_token,
            thumb_token,
            can_extract,
        };
        state.register_item(item);

        // 入队后台识别(队列线程逐本:舰队按页分治 / 串行)
        enqueue_pdf_ocr(
            app,
            PdfOcrJob {
                id: id.clone(),
                name: file_name,
                src_path: path.to_path_buf(),
                pages: count,
                weights,
            },
        );

        Ok(state.item_dto(&id).expect("刚插入的条目"))
    } else {
        // ---- 普通图片 ----
        let id = uuid::Uuid::new_v4().simple().to_string();
        let (w, h) = crate::image_util::probe_dims_oriented(path)
            .map_err(|e| format!("读取图片尺寸失败: {e}"))?;
        let media_token = state.media.register(path.to_path_buf(), mime_for(path));
        let item = ImageItem {
            id: id.clone(),
            name: path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "未命名".into()),
            path: path.to_path_buf(),
            w, h,
            origin: origin.into(),
            added_at: now_ms(),
            media_token,
            thumb_token: String::new(),
            can_extract: false,
        };
        state.register_item(item);
        Ok(state.item_dto(&id).expect("刚插入的条目"))
    }
}

/// 位图入库(剪贴板/截图)
pub fn ingest_bitmap(
    app: &AppHandle,
    name: &str,
    img: image::RgbaImage,
    origin: &str,
) -> Result<ImageItemDto, String> {
    let state = app.state::<crate::AppCtx>();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let png_path = state.dirs.inbox.join(format!("{id}.png"));
    img.save_with_format(&png_path, image::ImageFormat::Png)
        .map_err(|e| format!("保存图片失败: {e}"))?;
    let mut dto = ingest_file(app, &png_path, origin)?;
    dto.name = name.to_string();
    if let Some(item) = state.items.write().unwrap().get_mut(&dto.id) {
        item.name = name.to_string();
    }
    Ok(dto)
}
