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
pub fn finalize(app: &AppHandle, id: &str, outcome: &OcrOutcomeDto, thumb_token: Option<String>) {
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
        finalize(app, id, &out, None);
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
    finalize(app, id, &outcome.0, outcome.1);
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
        finalize(app, id, &out, None);
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
    finalize(app, id, &outcome, thumb);
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

// ---- 进程分治(大批量) ----

/// 进程分治的规模门槛:低于此走进程内(worker 启动+模型加载不划算)。
pub const PROC_BATCH_MIN: usize = 8;

/// 按档位与核数定 worker 进程数。medium 内存太重,不分治。
/// 实测(18 核大小核,100 张):总线程 ≈ 核数时最优,且**更多小 worker 优于
/// 更少大 worker**(8×t2 = 3.46× > 4×t4 = 2.82×)——单图推理本身能吃满核,
/// 小 worker 只为重叠解码/前后处理,多进程还能摊平图片大小长尾。
pub fn worker_count(tier: qppocr::Tier, cores: usize) -> usize {
    let cap = match tier {
        qppocr::Tier::Tiny => 8,
        qppocr::Tier::Small => 4,
        qppocr::Tier::Medium => return 0,
    };
    (cores / 2).clamp(2, cap)
}

pub fn threads_each(workers: usize, cores: usize) -> usize {
    (cores / workers).clamp(2, 4)
}

/// 大批量进程分治:K 个自重生 worker,逐行收结果,实时广播 item-done。
/// 调用方需已置 batch.running;完成后本函数负责收尾(running=false + batch://done)。
/// `weights`:每张图的像素数(ingest 时已探测),按负载均衡分块——引擎 CLI
/// probe 调度的思想,避免"重图扎堆一个 worker"。
pub fn spawn_proc_batch(app: AppHandle, tasks: Vec<(String, PathBuf, u64)>) {
    let state = app.state::<crate::AppCtx>();
    let spec = state.engine.spec();
    let models_dir = state.engine.models_dir();
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let k = worker_count(spec.tier, cores).max(1);
    let t_each = threads_each(k, cores);

    // 按像素权重均衡分块:重图优先塞给当前最轻的 worker(贪心,长尾更短)
    let mut sorted = tasks;
    sorted.sort_by(|a, b| b.2.cmp(&a.2));
    let mut chunks: Vec<Vec<(String, PathBuf)>> = vec![Vec::new(); k];
    let mut loads = vec![0u64; k];
    for (id, path, px) in sorted {
        let (lightest, _) = loads
            .iter()
            .enumerate()
            .min_by_key(|(_, l)| **l)
            .expect("k >= 1");
        chunks[lightest].push((id, path));
        loads[lightest] += px;
    }

    let finished: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let cancel = state.batch.cancel.clone();
    let all_ids: Vec<String> = chunks
        .iter()
        .flatten()
        .map(|(id, _)| id.clone())
        .collect();

    std::thread::spawn(move || {
        std::thread::scope(|scope| {
            for chunk in chunks {
                let app = app.clone();
                let finished = finished.clone();
                let cancel = cancel.clone();
                let models_dir = models_dir.clone();
                let spec = spec;
                scope.spawn(move || {
                    run_worker_chunk(app, models_dir, spec, t_each, chunk, finished, cancel);
                });
            }
        });

        // 未完成的(取消/进程异常)统一收尾,UI 不悬死
        let fin = finished.lock().unwrap().clone();
        for id in &all_ids {
            if !fin.contains(id) {
                finalize(&app, id, &crate::dto::outcome_err("已取消或 worker 异常退出"), None);
            }
        }
        let state = app.state::<crate::AppCtx>();
        state.batch.running.store(false, Ordering::SeqCst);
        let _ = app.emit("batch://done", ());
    });
}

#[allow(clippy::too_many_arguments)]
fn run_worker_chunk(
    app: AppHandle,
    models_dir: PathBuf,
    spec: crate::engine::EngineSpec,
    threads: usize,
    chunk: Vec<(String, PathBuf)>,
    finished: Arc<Mutex<HashSet<String>>>,
    cancel: Arc<AtomicBool>,
) {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            for (id, _) in chunk {
                finalize(
                    &app,
                    &id,
                    &crate::dto::outcome_err(format!("无法定位自身可执行文件: {e}")),
                    None,
                );
            }
            return;
        }
    };
    let thumb_dir = app.state::<crate::AppCtx>().dirs.thumbs.clone();
    let mut child = match Command::new(exe)
        .arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            for (id, _) in chunk {
                finalize(&app, &id, &crate::dto::outcome_err(format!("启动 worker 失败: {e}")), None);
            }
            return;
        }
    };

    let request = serde_json::json!({
        "modelsDir": models_dir.to_string_lossy(),
        "thumbDir": thumb_dir.to_string_lossy(),
        "tier": crate::settings::tier_str(spec.tier),
        "preset": crate::settings::preset_str(spec.preset),
        "threads": threads,
        "orientation": spec.orientation,
        "enhanceContrast": spec.enhance_contrast,
        "upscale": spec.upscale,
        "tasks": chunk
            .iter()
            .map(|(id, p)| serde_json::json!({
                "id": id,
                "path": p.to_string_lossy(),
            }))
            .collect::<Vec<_>>(),
    });

    let mut stdin = child.stdin.take().expect("worker stdin");
    if writeln!(stdin, "{request}").and_then(|_| stdin.flush()).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        for (id, _) in chunk {
            finalize(&app, &id, &crate::dto::outcome_err("写入 worker 失败"), None);
        }
        return;
    }
    drop(stdin); // 关闭 stdin:worker 处理完全部任务后自然退出

    if let Some(stdout) = child.stdout.take() {
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines() {
            if cancel.load(Ordering::SeqCst) {
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
            // worker 已把缩略图写进共享缓存目录,主进程注册令牌即可
            let thumb = if resp["thumb"].as_bool().unwrap_or(false) {
                let state = app.state::<crate::AppCtx>();
                let path = state.dirs.thumbs.join(format!("{id}.jpg"));
                if path.exists() {
                    Some(state.media.register(path, "image/jpeg"))
                } else {
                    None
                }
            } else {
                None
            };
            finalize(&app, id, &outcome, thumb);
            finished.lock().unwrap().insert(id.to_string());
        }
    }
    let _ = child.wait();
}
