//! 批量队列:小批量进程内并发(spawn_blocking + Semaphore);
//! 大批量自重生 worker 进程分治。
//!
//! 引擎的线程池是进程级"先到先得",单进程内并发 run 只能重叠串行段
//! (decode/pre/post);多进程各自持有引擎实例才能真正吃满核。

use crate::dto::{ItemStatusDto, OcrOutcomeDto};
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager};

pub struct BatchState {
    pub running: AtomicBool,
    pub cancel: Arc<AtomicBool>,
    pub semaphore: RwLock<Arc<tokio::sync::Semaphore>>,
}

impl BatchState {
    pub fn new(concurrency: usize) -> Self {
        Self {
            running: AtomicBool::new(false),
            cancel: Arc::new(AtomicBool::new(false)),
            semaphore: RwLock::new(Arc::new(tokio::sync::Semaphore::new(
                concurrency.max(1),
            ))),
        }
    }
}

/// 识别完成后的收尾:写历史 + 广播 item-done(可携带搭车生成的缩略图令牌)。
/// `pdf_page`:PDF 页任务的页号(图片 None)——前端据此丢弃晚到的非当前页结果。
pub fn finalize(
    app: &AppHandle,
    id: &str,
    outcome: &OcrOutcomeDto,
    thumb_token: Option<String>,
    pdf_page: Option<u32>,
) {
    let state = app.state::<crate::AppCtx>();
    if outcome.ok {
        let item = state.items.read().unwrap().get(id).cloned();
        if let Some(item) = item {
            state.history.append(crate::history::entry_from(&item, outcome));
        }
    }
    let _ = app.emit(
        "ocr://item-done",
        crate::dto::ItemDoneDto {
            id: id.to_string(),
            outcome: outcome.clone(),
            thumb_token,
            pdf_page,
        },
    );
}

/// 从已解码像素搭车生成缩略图(已存在则跳过),返回可用的新令牌。
pub(crate) fn ensure_thumb(app: &AppHandle, id: &str, img: &qppocr::Image) -> Option<String> {
    let state = app.state::<crate::AppCtx>();
    let path = state.dirs.thumbs.join(format!("{id}.jpg"));
    if path.exists() {
        return None; // 已有,前端令牌仍有效
    }
    match crate::thumb::make_thumb_from_rgb(img.w as u32, img.h as u32, &img.data, id, &state.dirs.thumbs) {
        Ok(_) => Some(state.media.register(path, "image/jpeg")),
        Err(_) => None,
    }
}

/// 单图识别(阻塞;在 spawn_blocking 里跑)。解码一次两用:缩略图 + 引擎输入。
/// 单张交互与批量共用此入口,保证行为一致。
pub fn run_item_blocking(app: &AppHandle, id: &str) -> OcrOutcomeDto {
    let state = app.state::<crate::AppCtx>();
    let item = {
        let items = state.items.read().unwrap();
        items.get(id).cloned()
    };
    let Some(item) = item else {
        // 条目缺失也必须广播完成事件——否则前端永远停在「排队中」
        let out = crate::dto::outcome_err(format!("图片不存在: {id}"));
        finalize(app, id, &out, None, None);
        return out;
    };
    let _ = app.emit(
        "ocr://item-status",
        ItemStatusDto {
            id: id.to_string(),
            phase: "running".into(),
        },
    );
    let outcome = match crate::image_util::decode_file_oriented(&item.path) {
        Ok(img) => {
            let thumb = ensure_thumb(app, id, &img);
            // catch_unwind:引擎内核在特殊形状上可能 panic,转为错误而不是挂死
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                state.engine.run_image(img)
            }));
            let out = crate::dto::outcome_from(match r {
                Ok(v) => v,
                Err(_) => Err(qppocr::Error::Image(
                    "引擎内核 panic(特殊图像形状?),请把图片反馈给开发".into(),
                )),
            });
            (out, thumb)
        }
        Err(e) => (crate::dto::outcome_from(Err(e)), None),
    };
    finalize(app, id, &outcome.0, outcome.1, None);
    outcome.0
}

/// 单图识别,但先把显示旋转(r=90/180/270,顺时针)烘焙进像素——
/// 「转正后重新识别」:结果框坐标逆变换回原图系,画布叠加依然精准。
pub fn run_item_rotated_blocking(app: &AppHandle, id: &str, rotation: u32) -> OcrOutcomeDto {
    let state = app.state::<crate::AppCtx>();
    let item = {
        let items = state.items.read().unwrap();
        items.get(id).cloned()
    };
    let Some(item) = item else {
        let out = crate::dto::outcome_err(format!("图片不存在: {id}"));
        finalize(app, id, &out, None, None);
        return out;
    };
    let _ = app.emit(
        "ocr://item-status",
        ItemStatusDto {
            id: id.to_string(),
            phase: "running".into(),
        },
    );

    let (outcome, thumb) = match crate::image_util::decode_file_oriented(&item.path) {
        Ok(img) => {
            let thumb = ensure_thumb(app, id, &img);
            let rot = match rotation % 360 {
                90 | 180 | 270 => rotation % 360,
                _ => 0,
            };
            let r = if rot == 0 {
                state.engine.run_image(img)
            } else {
                match state.engine.run_image(rotate_rgb_cw(&img, rot)) {
                    Ok(mut r) => {
                        // 框坐标:旋转帧 → 原图帧(逆映射)
                        let (iw, ih) = (img.w, img.h);
                        for line in r.lines.iter_mut() {
                            for p in line.pts.iter_mut() {
                                let (dx, dy) = (p[0], p[1]);
                                let (x, y): (f32, f32) = match rot {
                                    90 => (dy, ih as f32 - 1.0 - dx),
                                    180 => (iw as f32 - 1.0 - dx, ih as f32 - 1.0 - dy),
                                    _ => (iw as f32 - 1.0 - dy, dx), // 270
                                };
                                *p = [x, y];
                            }
                        }
                        Ok(r)
                    }
                    Err(e) => Err(e),
                }
            };
            (crate::dto::outcome_from(r), thumb)
        }
        Err(e) => (crate::dto::outcome_from(Err(e)), None),
    };
    finalize(app, id, &outcome, thumb, None);
    outcome
}

/// RGB 缓冲顺时针旋转(r=90/180/270)。
fn rotate_rgb_cw(img: &qppocr::Image, r: u32) -> qppocr::Image {
    let (iw, ih) = (img.w as usize, img.h as usize);
    let src = &img.data;
    let (ow, oh) = match r {
        90 | 270 => (ih, iw),
        _ => (iw, ih),
    };
    let mut out = vec![0u8; ow * oh * 3];
    for y in 0..ih {
        for x in 0..iw {
            let (dx, dy) = match r {
                90 => (ih - 1 - y, x),
                180 => (iw - 1 - x, ih - 1 - y),
                _ => (y, iw - 1 - x), // 270
            };
            let s = (y * iw + x) * 3;
            let d = (dy * ow + dx) * 3;
            out[d] = src[s];
            out[d + 1] = src[s + 1];
            out[d + 2] = src[s + 2];
        }
    }
    qppocr::rgb_from_bytes(ow as u32, oh as u32, out).expect("长度自洽")
}

// ---- 进程分治(大批量图片 / PDF 整册,通用舰队) ----

/// 进程分治的规模门槛:低于此走进程内(worker 启动+模型加载不划算)。
pub const PROC_BATCH_MIN: usize = 8;

/// worker 单行结果解析后的公共形状(图片/PDF 页通用)。
pub(crate) struct WorkerEvent {
    pub id: String,
    /// PDF 页任务的页码(图片任务为 None)
    pub page: Option<u32>,
    pub outcome: OcrOutcomeDto,
    /// 图片模式:worker 已把缩略图写进共享缓存目录(令牌注册留给主进程 sink)
    pub thumb_written: bool,
}

/// 舰队任务项:图片(path)或 PDF 页(page)。
#[derive(Clone)]
pub(crate) struct FleetItem {
    pub id: String,
    pub page: Option<u32>,
    pub path: Option<PathBuf>,
    /// 像素权重(LPT 均衡分块用)
    pub weight: u64,
}

/// 把失败批量合成为错误事件回执。
fn emit_err_items(
    items: &[(String, Option<u32>)],
    msg: &str,
    on_event: &mut impl FnMut(WorkerEvent),
) {
    for (id, page) in items {
        on_event(WorkerEvent {
            id: id.clone(),
            page: *page,
            outcome: crate::dto::outcome_err(msg.to_string()),
            thumb_written: false,
        });
    }
}

/// 单 worker 进程生命周期:spawn 自身 exe → 写一行请求 → 逐行回调 → 收尸。
/// 失败路径(exe 定位/spawn/写 stdin)对整块合成错误回执;读循环中断(崩溃/
/// 取消/暂停)只回执已收到的行,缺口由 run_fleet 收尾补齐(暂停除外,静默)。
fn run_worker_proc(
    req_base: &serde_json::Value,
    chunk: &[FleetItem],
    cancel: &AtomicBool,
    pause: &AtomicBool,
    mut on_event: impl FnMut(WorkerEvent),
) {
    let items: Vec<(String, Option<u32>)> = chunk
        .iter()
        .map(|t| (t.id.clone(), t.page))
        .collect();

    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            emit_err_items(&items, &format!("无法定位自身可执行文件: {e}"), &mut on_event);
            return;
        }
    };
    let mut child = match Command::new(exe)
        .arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            emit_err_items(&items, &format!("启动 worker 失败: {e}"), &mut on_event);
            return;
        }
    };

    let tasks: Vec<_> = chunk
        .iter()
        .map(|t| match (&t.path, t.page) {
            (Some(p), _) => serde_json::json!({ "id": t.id, "path": p.to_string_lossy() }),
            (None, Some(page)) => serde_json::json!({ "id": t.id, "page": page }),
            (None, None) => serde_json::json!({ "id": t.id }),
        })
        .collect();
    let mut request = req_base.clone();
    request["tasks"] = serde_json::Value::Array(tasks);

    let mut stdin = child.stdin.take().expect("worker stdin");
    if writeln!(stdin, "{request}").and_then(|_| stdin.flush()).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        emit_err_items(&items, "写入 worker 失败", &mut on_event);
        return;
    }
    drop(stdin); // 关闭 stdin:worker 处理完全部任务后自然退出

    if let Some(stdout) = child.stdout.take() {
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines() {
            if cancel.load(Ordering::SeqCst) || pause.load(Ordering::SeqCst) {
                let _ = child.kill();
                break;
            }
            let Ok(line) = line else { break };
            let Ok(resp) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let Some(id) = resp["id"].as_str() else { continue };
            if id.is_empty() {
                continue;
            }
            let outcome: OcrOutcomeDto = serde_json::from_value(resp["outcome"].clone())
                .unwrap_or_else(|_| crate::dto::outcome_err("worker 结果解析失败"));
            on_event(WorkerEvent {
                id: id.to_string(),
                page: resp["page"].as_u64().map(|p| p as u32),
                outcome,
                thumb_written: resp["thumb"].as_bool().unwrap_or(false),
            });
        }
    }
    let _ = child.wait();
}

/// LPT 贪心分块:重项优先塞给当前最轻的 worker(贪心,长尾更短)。
fn lpt_partition<T: Clone>(items: Vec<(T, u64)>, k: usize) -> Vec<Vec<T>> {
    let k = k.max(1);
    let mut sorted = items;
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    let mut chunks: Vec<Vec<T>> = vec![Vec::new(); k];
    let mut loads = vec![0u64; k];
    for (item, px) in sorted {
        let (lightest, _) = loads
            .iter()
            .enumerate()
            .min_by_key(|(_, l)| **l)
            .expect("k >= 1");
        chunks[lightest].push(item);
        loads[lightest] += px;
    }
    chunks
}

/// 通用舰队:K 个自重生 worker 并行,**阻塞**至全部结束。
/// `req_base`:请求公共字段(不含 tasks,线程数等已含);`pdf_spec`:PDF 模式
/// 请求级 `pdf` 字段。结果实时回调 report(多线程共享,须 Sync)。
/// 中断语义:`cancel`(真取消,worker 崩溃/批量取消)对缺口统一合成错误回执;
/// `pause`(暂停,仅 PDF 队列用)静默中断——缺失页留给 resume 重跑。
pub(crate) fn run_fleet(
    mut req_base: serde_json::Value,
    pdf_spec: Option<serde_json::Value>,
    items: Vec<FleetItem>,
    k: usize,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    report: &(impl Fn(WorkerEvent) + Sync),
) {
    if items.is_empty() {
        return;
    }
    let k = k.clamp(1, items.len());
    let chunks = lpt_partition(
        items.into_iter().map(|t| { let w = t.weight; (t, w) }).collect(),
        k,
    );

    if let Some(pdf) = pdf_spec {
        req_base["pdf"] = pdf;
    }

    // (id, page) 唯一标识一项——PDF 各页共用条目 id,必须带页码区分
    let finished: Mutex<HashSet<(String, Option<u32>)>> = Mutex::new(HashSet::new());
    let all_keys: Vec<(String, Option<u32>)> = chunks
        .iter()
        .flatten()
        .map(|t| (t.id.clone(), t.page))
        .collect();

    // kill 条件 = 取消或暂停(读循环逐行检查,每行两次原子读,开销可忽略)
    std::thread::scope(|scope| {
        for chunk in chunks {
            let finished = &finished;
            let req = &req_base;
            let cancel = cancel.clone();
            let pause = pause.clone();
            scope.spawn(move || {
                run_worker_proc(req, &chunk, &cancel, &pause, |ev| {
                    let key = (ev.id.clone(), ev.page);
                    report(ev);
                    finished.lock().unwrap().insert(key);
                });
            });
        }
    });

    // 暂停:静默返回(缺失页由 resume 重跑);取消/崩溃:统一错误回执,UI 不悬死
    if pause.load(Ordering::SeqCst) {
        return;
    }
    let fin = finished.into_inner().unwrap();
    for key in all_keys {
        if !fin.contains(&key) {
            report(WorkerEvent {
                id: key.0,
                page: key.1,
                outcome: crate::dto::outcome_err("已取消或 worker 异常退出"),
                thumb_written: false,
            });
        }
    }
}

/// 大批量图片进程分治:K 个自重生 worker,逐行收结果,实时 finalize。
/// 调用方需已置 batch.running;完成后本函数负责收尾(running=false + batch://done)。
/// `weights`:每张图的像素数(ingest 时已探测)。
pub fn spawn_proc_batch(
    app: AppHandle,
    tasks: Vec<(String, PathBuf, u64)>,
    plan: crate::hw::ParallelPlan,
) {
    let state = app.state::<crate::AppCtx>();
    let spec = state.engine.spec();
    let models_dir = state.engine.models_dir();
    let k = plan.workers.max(1);
    let t_each = plan.threads_each.max(1);

    let items: Vec<FleetItem> = tasks
        .into_iter()
        .map(|(id, path, weight)| FleetItem {
            id,
            page: None,
            path: Some(path),
            weight,
        })
        .collect();

    let req_base = serde_json::json!({
        "modelsDir": models_dir.to_string_lossy(),
        "thumbDir": state.dirs.thumbs.to_string_lossy(),
        "tier": crate::settings::tier_str(spec.tier),
        "preset": crate::settings::preset_str(spec.preset),
        "threads": t_each,
        "orientation": spec.orientation,
        "enhanceContrast": spec.enhance_contrast,
        "upscale": spec.upscale,
    });

    let cancel = state.batch.cancel.clone();
    // 图片批量没有「暂停」语义,传一个永不置位的占位旗标
    let no_pause = Arc::new(AtomicBool::new(false));
    std::thread::spawn(move || {
        let sink = |ev: WorkerEvent| {
            // worker 已把缩略图写进共享缓存目录,主进程注册令牌即可
            let thumb = if ev.thumb_written {
                let state = app.state::<crate::AppCtx>();
                let path = state.dirs.thumbs.join(format!("{}.jpg", ev.id));
                if path.exists() {
                    Some(state.media.register(path, "image/jpeg"))
                } else {
                    None
                }
            } else {
                None
            };
            finalize(&app, &ev.id, &ev.outcome, thumb, None);
        };
        run_fleet(req_base, None, items, k, cancel, no_pause, &sink);
        let state = app.state::<crate::AppCtx>();
        state.batch.running.store(false, Ordering::SeqCst);
        let _ = app.emit("batch://done", ());
    });
}
