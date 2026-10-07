//! 统一入库：图片 / PDF / 剪贴板 / 截图 → ImageItem。
//!
//! PDF 策略(内存模型优先):入库只做 第0页渲染+页尺寸探测,**源字节不常驻**
//! (PDF_STORE 只存路径,用时 fs::read 走 OS page cache)。≤PDF_AUTO_OCR_MAX 页
//! 自动入队全册识别;更大文档默认按需模式(翻到哪页识别哪页,可点「识别全部」)。
//! 长驻队列线程逐本处理(多本排队,与图片批量互斥),每本按页数和硬件优化表选
//! 模式:≥8 页且档位允许 → K 个 worker 进程按页分治(渲染+OCR 都在 worker,
//! pdfium 全局锁决定了进程内并行无意义);否则主进程串行。job 携带页子集
//! (暂停后继续只跑缺失页)。逐页推 pdf://page-done(done=结果表长度,暂停/
//! 继续/乱序下自洽),结束推 pdf://ocr-done(带 completed,completed<total 即暂停)。

use crate::dto::ImageItemDto;
use crate::dto::OcrOutcomeDto;
use crate::media::mime_for;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use tauri::{AppHandle, Emitter, Manager};

lazy_static::lazy_static! {
    /// PDF 源路径(不存字节——几百 MB 的 PDF 常驻内存不可接受;用时读文件)
    pub static ref PDF_STORE: Mutex<HashMap<String, PathBuf>> =
        Mutex::new(HashMap::new());
    /// PDF 元信息:id → (页数, 按需模式, 每页像素权重)
    pub static ref PDF_PAGES: Mutex<HashMap<String, PdfMeta>> =
        Mutex::new(HashMap::new());
    /// PDF 每页识别结果(BTreeMap:舰队乱序插入 + 单页快查 + 天然有序;
    /// 按需模式下只积累翻过的页——识别内存随使用量增长,不随文档大小)
    pub static ref PDF_RESULTS: Mutex<HashMap<String, BTreeMap<u32, OcrOutcomeDto>>> =
        Mutex::new(HashMap::new());
}

/// 舰队分治的页数门槛:低于此 worker 启动+模型加载不划算。
pub const PDF_FLEET_MIN: usize = 8;

/// 自动全册识别的页数上限:超过此值的 PDF 默认按需模式(识别全部需手动点)。
/// 上限同时封顶了自动模式的识别结果内存(50 页 ≈ 数 MB)。
pub const PDF_AUTO_OCR_MAX: u32 = 50;

#[derive(Clone)]
pub struct PdfMeta {
    pub count: u32,
    /// 按需模式:翻到哪页识别哪页,不自动全册(文本层 PDF 恒 false——直提秒完)
    pub on_demand: bool,
    /// 文本层直提模式(resume/单页识别也要走直提而非 OCR)
    pub extract: bool,
    /// 每页像素权重(ingest 时探测,resume 任务的 LPT 分块用)
    pub weights: Vec<u64>,
}

// ---- PDF 识别队列(逐本处理,与图片批量互斥) ----

pub struct PdfOcrJob {
    pub id: String,
    pub name: String,
    /// 源文件路径(worker/串行 都读它)
    pub src_path: PathBuf,
    /// 本次要识别的页号(升序子集;暂停后继续 = 只含缺失页)
    pub pages: Vec<u32>,
    /// 全册页数(进度分母)
    pub total: u32,
    /// 每页像素权重(全册,下标=页号)
    pub weights: Vec<u64>,
    /// 文本层直提模式(数字原生 PDF,跳过 OCR,毫秒级/页)
    pub extract: bool,
}

pub struct PdfOcrState {
    queue: Mutex<VecDeque<PdfOcrJob>>,
    cv: Condvar,
    /// 舰队运行中 → 图片批量 batch_start 返回 busy(互斥共用 worker 预算)
    pub fleet_active: AtomicBool,
    /// 暂停旗标:与「取消」语义分离——暂停静默中断(kill worker/串行 break),
    /// 不对未完成页合成错误回执,继续时这些页会重跑
    pub pause: Arc<AtomicBool>,
    /// 按需单页识别队列(LIFO:最后翻到的页优先),唯一执行线程消费。
    /// 元素 (id, page, force_ocr):force_ocr 让文本层 PDF 的单页也走 OCR
    ondemand: Mutex<Vec<(String, u32, bool)>>,
    ondemand_cv: Condvar,
}

impl Default for PdfOcrState {
    fn default() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            fleet_active: AtomicBool::new(false),
            pause: Arc::new(AtomicBool::new(false)),
            ondemand: Mutex::new(Vec::new()),
            ondemand_cv: Condvar::new(),
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
    // 缩略图刻意保留:历史条目按 id 引用它,而 PDF 缩略图无法像图片那样
    // 从源文件重新生成(源是 PDF,不是图片)。删了 = 该历史行永久占位图标。
}

/// 入队一本 PDF 的后台识别。
pub fn enqueue_pdf_ocr(app: &AppHandle, job: PdfOcrJob) {
    let state = app.state::<crate::AppCtx>();
    let mut q = state.pdf.queue.lock().unwrap();
    q.push_back(job);
    drop(q);
    state.pdf.cv.notify_one();
}

/// 暂停该 PDF 的识别:静默中断当前 run(kill worker / 串行 break,不产生
/// 错误回执)+ 清除队列中同 id 的待跑任务。返回当前已完成页数(前端同步用)。
pub fn pause_pdf(app: &AppHandle, id: &str) -> Option<u32> {
    let state = app.state::<crate::AppCtx>();
    state.pdf.pause.store(true, Ordering::SeqCst);
    state.pdf.queue.lock().unwrap().retain(|j| j.id != id);
    let done = PDF_RESULTS
        .lock()
        .unwrap()
        .get(id)
        .map(|m| m.len() as u32);
    let _ = app;
    done
}

/// 计算 (全册页数, 缺失页列表)。条目不存在返回 None。
pub fn remaining_pages(id: &str) -> Option<(u32, Vec<u32>)> {
    let meta = PDF_PAGES.lock().unwrap().get(id).cloned()?;
    let results = PDF_RESULTS.lock().unwrap();
    let done = results.get(id);
    let pages = (0..meta.count)
        .filter(|p| done.map_or(true, |m| !m.contains_key(p)))
        .collect();
    Some((meta.count, pages))
}

/// 按需识别单页:翻到未识别页时前端触发。只入 LIFO 队列(最新翻到的页
/// 优先),由唯一的常驻执行线程处理——**绝不能每页起一个线程**:每个线程
/// 都要 fs::read 整份 PDF + load_doc 全文解析(pdfium 全局锁串行化),
/// 快速翻完一本 = 几十个线程堆积,主进程 PDF 功能整体假死。
pub fn recognize_single_page(app: &AppHandle, id: &str, page: u32) {
    push_ondemand(app, id, page, false);
}

/// 强制单页 OCR(文本层 PDF 用户点「本页改用OCR」):绕过直提分支;
/// 已有结果也重跑(用户显式要求)。
pub fn recognize_page_force_ocr(app: &AppHandle, id: &str, page: u32) {
    push_ondemand(app, id, page, true);
}

fn push_ondemand(app: &AppHandle, id: &str, page: u32, force_ocr: bool) {
    if !force_ocr {
        let results = PDF_RESULTS.lock().unwrap();
        if results.get(id).map_or(false, |m| m.contains_key(&page)) {
            return; // 已有结果
        }
    }
    let state = app.state::<crate::AppCtx>();
    let mut q = state.pdf.ondemand.lock().unwrap();
    q.retain(|(i, p, _)| !(i == id && *p == page)); // 去重:同页旧请求出队
    q.push((id.to_string(), page, force_ocr));
    drop(q);
    state.pdf.ondemand_cv.notify_one();
}

/// 整册切换识别方式(直提 ⇄ OCR):清空已有结果,按新模式重跑全部页。
/// 切直提前校验文本层;清结果让 done 计数从 0 重新爬。
pub fn set_pdf_mode(app: &AppHandle, id: &str, extract: bool) -> Result<bool, String> {
    let state = app.state::<crate::AppCtx>();
    let meta = PDF_PAGES
        .lock()
        .unwrap()
        .get(id)
        .cloned()
        .ok_or("PDF 条目不存在")?;
    if meta.extract == extract {
        return Ok(false);
    }
    if extract {
        // 切直提必须有文本层(ingest 时的 can_extract 可能是 false)
        let src = PDF_STORE
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or("PDF 条目不存在")?;
        let bytes = std::fs::read(&src).map_err(|e| format!("读取 PDF 失败: {e}"))?;
        if !crate::pdf::has_text_layer(&bytes) {
            return Err("此 PDF 没有文本层,只能 OCR".into());
        }
    }
    // 清结果 + 改模式 + 作废队列中同 id 旧任务 → 全册重跑
    PDF_RESULTS.lock().unwrap().insert(id.to_string(), BTreeMap::new());
    PDF_PAGES.lock().unwrap().get_mut(id).unwrap().extract = extract;
    state.pdf.queue.lock().unwrap().retain(|j| j.id != id);
    let name = state
        .items
        .read()
        .unwrap()
        .get(id)
        .map(|i| i.name.clone())
        .unwrap_or_default();
    let src = PDF_STORE
        .lock()
        .unwrap()
        .get(id)
        .cloned()
        .ok_or("PDF 条目不存在")?;
    enqueue_pdf_ocr(
        app,
        PdfOcrJob {
            id: id.to_string(),
            name,
            src_path: src,
            pages: (0..meta.count).collect(),
            total: meta.count,
            weights: meta.weights,
            extract,
        },
    );
    Ok(true)
}

/// 常驻按需执行线程:逐页 渲染+OCR。同一 PDF 的字节与文档解析结果缓存
/// 复用(整本翻完只读一次文件、解析一次);引擎有界等待;条目删除/页失效跳过。
/// 注意:此处不看 pause——按需单页是用户正在看的页,立即响应优先。
pub fn spawn_ondemand_worker(app: AppHandle) {
    std::thread::spawn(move || {
        let state = app.state::<crate::AppCtx>();
        // 文档缓存:load_doc_from_file 不借用调用方数据,换 PDF 时 drop 旧的即可
        let mut cur_id = String::new();
        let mut cur_doc: Option<pdfium_render::prelude::PdfDocument<'static>> = None;
        let mut cur_count = 0u32;
        let mut cur_extract = false;

        loop {
            // 等任务(LIFO:最后翻到的页最先处理)
            let (id, page, force_ocr) = {
                let mut q = state.pdf.ondemand.lock().unwrap();
                loop {
                    if let Some(job) = q.pop() {
                        break job;
                    }
                    q = state.pdf.ondemand_cv.wait(q).unwrap();
                }
            };
            // 已有结果(可能被舰队补齐)/条目已删 → 跳过;强制 OCR 例外
            if !force_ocr {
                let results = PDF_RESULTS.lock().unwrap();
                if results.get(&id).map_or(true, |m| m.contains_key(&page)) {
                    continue;
                }
            }
            if !PDF_PAGES.lock().unwrap().contains_key(&id) {
                continue;
            }
            // 换 PDF 时重建文档缓存
            if cur_id != id || cur_doc.is_none() {
                cur_doc = None;
                let Some(src) = PDF_STORE.lock().unwrap().get(&id).cloned() else {
                    continue;
                };
                let Some(meta) = PDF_PAGES.lock().unwrap().get(&id).cloned() else {
                    continue;
                };
                cur_count = meta.count;
                cur_extract = meta.extract;
                match crate::pdf::load_doc_from_file(&src) {
                    Ok(d) => {
                        cur_doc = Some(d);
                        cur_id = id.clone();
                    }
                    Err(e) => {
                        // 读不了/解析不了:该页给错误回执,不让前端悬等
                        cur_id.clear();
                        let out = crate::dto::outcome_err(format!("读取 PDF 失败: {e}"));
                        let d = insert_result(&id, page, out.clone());
                        emit_page(&app, &id, page, &out, d, cur_count.max(page + 1));
                        continue;
                    }
                }
            }
            if page >= cur_count {
                continue;
            }
            let doc = cur_doc.as_ref().expect("刚校验过");
            // 文本层 PDF 单页默认直提;force_ocr(用户点「本页改用OCR」)走 OCR
            let outcome = if cur_extract && !force_ocr {
                // 文本层 PDF:单页也走直提(毫秒级,无需引擎)
                crate::pdf::extract_doc_text_lines(doc, page, crate::pdf::DPI_OCR)
                    .map(extract_lines_to_outcome)
                    .unwrap_or_else(crate::dto::outcome_err)
            } else {
                // 引擎有界等待(启动头几秒)
                let mut engine = state.engine.current();
                for _ in 0..150 {
                    if engine.is_some() {
                        break;
                    }
                    if !PDF_PAGES.lock().unwrap().contains_key(&id) {
                        engine = None;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    engine = state.engine.current();
                }
                let Some(engine) = engine else { continue };
                match crate::pdf::render_doc_page(doc, page, crate::pdf::DPI_OCR)
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
                }
            };
            let d = insert_result(&id, page, outcome.clone());
            emit_page(&app, &id, page, &outcome, d, cur_count);
        }
    });
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
    // 复用图片批量的取消旗标 + 本队列的暂停旗标(此刻图片批量必不在跑,
    // 复位安全;上一 run 的暂停残留也在此清零)
    state.batch.cancel.store(false, Ordering::SeqCst);
    state.pdf.pause.store(false, Ordering::SeqCst);

    // 文本层直提:毫秒级/页,主进程串行即可(不受舰队/暂停阈值约束)
    if job.extract {
        serial_pdf_extract(app, &job);
        let completed = PDF_RESULTS
            .lock()
            .unwrap()
            .get(&job.id)
            .map(|m| m.len() as u32)
            .unwrap_or(0);
        let _ = app.emit(
            "pdf://ocr-done",
            serde_json::json!({
                "id": job.id,
                "total": job.total,
                "completed": completed,
                "name": job.name,
                "elapsedMs": started.elapsed().as_millis() as u64,
            }),
        );
        return;
    }

    let workers_override = state.settings.read().unwrap().workers_override;
    let spec = state.engine.spec();
    let plan = crate::hw::plan_for(&state.hw, spec.tier, workers_override, &spec.device);
    // 自测/基准:强制小文档也走舰队路径
    let force = std::env::var("QPP_PDF_FLEET_FORCE").is_ok();
    let use_fleet =
        plan.workers > 0 && (job.pages.len() >= PDF_FLEET_MIN || force);

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

    // completed < total = 被暂停(或个别页永久失败);前端据此显示「已暂停」
    let completed = PDF_RESULTS
        .lock()
        .unwrap()
        .get(&job.id)
        .map(|m| m.len() as u32)
        .unwrap_or(0);
    let _ = app.emit(
        "pdf://ocr-done",
        serde_json::json!({
            "id": job.id,
            "total": job.total,
            "completed": completed,
            "name": job.name,
            "elapsedMs": started.elapsed().as_millis() as u64,
        }),
    );
}

/// 逐页推送结果 + 页0 落历史(舰队 sink 与串行共用)。
/// done 由调用方在 PDF_RESULTS 锁内 insert 后取 len()——暂停/继续/乱序下自洽。
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

/// 入库一条结果并返回新的 done(结果表长度)。
fn insert_result(id: &str, page: u32, outcome: OcrOutcomeDto) -> usize {
    let mut results = PDF_RESULTS.lock().unwrap();
    let m = results.entry(id.to_string()).or_default();
    m.insert(page, outcome);
    m.len()
}

/// 舰队模式:K 个 worker 按页分治(渲染+OCR 都在 worker)。
/// pub(crate):selftest 基准直接调用(不走队列,精确计时)。
pub(crate) fn run_pdf_fleet(app: &AppHandle, job: &PdfOcrJob, plan: &crate::hw::ParallelPlan) {
    let state = app.state::<crate::AppCtx>();
    let spec = state.engine.spec();
    let models_dir = state.engine.models_dir();

    // worker 数还受页数约束:每 worker 至少 2 页,摊薄引擎构建成本
    let k = plan.workers.min(job.pages.len().div_ceil(2)).max(1);

    // worker 读源文件;不可读(源被移走/网盘掉线)则退回串行(它会给出错误回执)
    let src_readable = std::fs::metadata(&job.src_path)
        .map(|m| m.is_file())
        .unwrap_or(false);
    if !src_readable {
        serial_pdf_ocr(app, job);
        return;
    }

    let parent = job.id.clone();
    let cancel = state.batch.cancel.clone();
    let pause = state.pdf.pause.clone();

    // PDF sink:入库、推事件;条目已删则杀舰队止损
    let sink = |ev: crate::batch::WorkerEvent| {
        let Some(page) = ev.page else { return };
        if !PDF_PAGES.lock().unwrap().contains_key(&parent) {
            cancel.store(true, Ordering::SeqCst);
            return;
        }
        let d = insert_result(&parent, page, ev.outcome.clone());
        emit_page(app, &parent, page, &ev.outcome, d, job.total);
    };

    let req_base = serde_json::json!({
        "modelsDir": models_dir.to_string_lossy(),
        "tier": crate::settings::tier_str(spec.tier),
        "preset": crate::settings::preset_str(spec.preset),
        "threads": plan.threads_each.max(1),
        "device": if matches!(spec.device, qppocr::DeviceChoice::Gpu { .. }) { "gpu" } else { "cpu" },
        "orientation": spec.orientation,
        "enhanceContrast": spec.enhance_contrast,
        "upscale": spec.upscale,
    });
    let pdf_spec = serde_json::json!({
        "path": job.src_path.to_string_lossy(),
        "dpi": crate::pdf::DPI_OCR,
    });
    let items: Vec<crate::batch::FleetItem> = job
        .pages
        .iter()
        .map(|&p| crate::batch::FleetItem {
            id: job.id.clone(),
            page: Some(p),
            path: None,
            weight: job.weights.get(p as usize).copied().unwrap_or(1),
        })
        .collect();

    crate::batch::run_fleet(req_base, Some(pdf_spec), items, k, cancel.clone(), pause, &sink);
}

/// 文本层行集 → OCR 同构结果(行框全宽近似,置信度 1.0)。
/// 串行直提与按需单页共用。
fn extract_lines_to_outcome(
    lines: Vec<(String, f32, [[f32; 2]; 4])>,
) -> OcrOutcomeDto {
    let line_dtos = lines
        .iter()
        .map(|(text, conf, pts)| crate::dto::TextLineDto {
            text: text.clone(),
            confidence: *conf,
            rotation: 0,
            pts: *pts,
            chars: Vec::new(),
            retried: false,
        })
        .collect::<Vec<_>>();
    // 结果框基于页面渲染坐标;从行框反推页面包围盒作 work 尺寸
    let (mut w, mut h) = (0f32, 0f32);
    for l in &line_dtos {
        for p in &l.pts {
            w = w.max(p[0]);
            h = h.max(p[1]);
        }
    }
    OcrOutcomeDto {
        ok: true,
        error: None,
        result: Some(crate::dto::OcrResultDto {
            lines: line_dtos,
            work_w: w as i32,
            work_h: h as i32,
            num_boxes: lines.len() as u32,
            num_merged: 0,
            num_decluttered: 0,
            num_det_retried: 0,
            num_flipped: 0,
            num_unread: 0,
            timings: Default::default(),
            extracted: true,
        }),
    }
}

/// 文本层直提(数字原生 PDF):主进程逐页提取文本层 → 组装成 OCR 同构结果
/// (行框为全宽近似,置信度 1.0)。毫秒级/页;失败页产出错误回执,不悬死。
fn serial_pdf_extract(app: &AppHandle, job: &PdfOcrJob) {
    let state = app.state::<crate::AppCtx>();
    let Some(src) = PDF_STORE.lock().unwrap().get(&job.id).cloned() else {
        return;
    };
    let bytes = match std::fs::read(&src) {
        Ok(b) => b,
        Err(_) => return,
    };
    let doc = match crate::pdf::load_doc(&bytes) {
        Ok(d) => d,
        Err(_) => return,
    };
    for &page in &job.pages {
        if state.batch.cancel.load(Ordering::Relaxed)
            || state.pdf.pause.load(Ordering::Relaxed)
        {
            break;
        }
        if !PDF_PAGES.lock().unwrap().contains_key(&job.id) {
            break; // 条目已删
        }
        let outcome = crate::pdf::extract_doc_text_lines(&doc, page, crate::pdf::DPI_OCR)
            .map(extract_lines_to_outcome)
            .unwrap_or_else(crate::dto::outcome_err);
        let d = insert_result(&job.id, page, outcome.clone());
        emit_page(app, &job.id, page, &outcome, d, job.total);
    }
}

/// 串行模式(页数少 / medium / 按需单页 / 舰队不可用):主进程逐页渲染 +
/// 共享引擎。渲染/解码失败产出错误回执(不静默跳过,否则前端进度悬死)。
fn serial_pdf_ocr(app: &AppHandle, job: &PdfOcrJob) {
    let state = app.state::<crate::AppCtx>();
    let err_all = |msg: &str| {
        for &page in &job.pages {
            let out = crate::dto::outcome_err(msg.to_string());
            let d = insert_result(&job.id, page, out.clone());
            emit_page(app, &job.id, page, &out, d, job.total);
        }
    };

    let Some(src) = PDF_STORE.lock().unwrap().get(&job.id).cloned() else {
        err_all("PDF 数据已失效");
        return;
    };
    let bytes = match std::fs::read(&src) {
        Ok(b) => b,
        Err(e) => {
            err_all(&format!("读取 PDF 失败: {e}"));
            return;
        }
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

    for &page in &job.pages {
        if state.batch.cancel.load(Ordering::Relaxed)
            || state.pdf.pause.load(Ordering::Relaxed)
        {
            break; // 取消/暂停:静默中断,缺失页留给 resume
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
        let d = insert_result(&job.id, page, outcome.clone());
        emit_page(app, &job.id, page, &outcome, d, job.total);
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

        // 存元信息(源只存路径——几百 MB 的 PDF 字节常驻内存不可接受)。
        // 文本层 PDF 恒为自动模式:直提毫秒级完成,按需无意义
        let on_demand = !can_extract && count > PDF_AUTO_OCR_MAX;
        PDF_STORE.lock().unwrap().insert(id.clone(), path.to_path_buf());
        PDF_PAGES.lock().unwrap().insert(
            id.clone(),
            PdfMeta {
                count,
                on_demand,
                extract: can_extract,
                weights: weights.clone(),
            },
        );
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

        // 有文本层(数字原生)→ 直提管线:毫秒级/页,不受按需阈值约束,
        // 拖入即秒出全部结果,完全跳过 OCR。
        // 无文本层 → ≤阈值自动入队全册 OCR;大文档默认按需模式
        // (前端翻页触发单页识别,「识别全部」可随时切整册)
        if can_extract {
            enqueue_pdf_ocr(
                app,
                PdfOcrJob {
                    id: id.clone(),
                    name: file_name,
                    src_path: path.to_path_buf(),
                    pages: (0..count).collect(),
                    total: count,
                    weights,
                    extract: true,
                },
            );
        } else if !on_demand {
            enqueue_pdf_ocr(
                app,
                PdfOcrJob {
                    id: id.clone(),
                    name: file_name,
                    src_path: path.to_path_buf(),
                    pages: (0..count).collect(),
                    total: count,
                    weights,
                    extract: false,
                },
            );
        }

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
