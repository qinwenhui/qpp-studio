//! 自重生 worker 进程:大批量图片 / PDF 整册按页分治。
//! 引擎的进程级线程池"先到先得",单进程内并发 run 只能重叠串行段;多进程才能真正吃满核。
//! PDF 渲染同理:pdfium 的 thread_safe 是全局互斥锁,进程内并行渲染=串行,
//! 每个进程独立 pdfium 实例才是真并行。
//!
//! 协议(一行 JSON,简单可控):
//!   图片: stdin  {"modelsDir","thumbDir","tier","preset","threads",
//!                 "orientation","enhanceContrast","upscale",
//!                 "tasks":[{"id","path"},...]}
//!          stdout 每完成一行 {"id","thumb":bool,"outcome"}(逐行 flush)
//!   PDF : stdin  请求级 "pdf":{"path","dpi"} + tasks [{"id","page"},...]
//!          stdout 每完成一行 {"id","page","outcome"}
//! 任何失败(文件读不了/引擎加载失败/渲染失败)都对剩余任务逐条回错误结果,
//! 父进程永不悬等。

use crate::dto;
use crate::engine::{build_engine, EngineSpec};
use serde::Deserialize;
use std::io::{BufRead, Write};
use std::path::Path;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerRequest {
    models_dir: String,
    /// 缩略图缓存目录(worker 搭识别解码的便车直接写入,主进程只注册令牌)
    #[serde(default)]
    thumb_dir: Option<String>,
    /// PDF 模式:本请求所有任务都是同一 PDF 的页任务
    #[serde(default)]
    pdf: Option<PdfSpec>,
    tier: String,
    preset: String,
    threads: usize,
    #[serde(default = "yes")]
    orientation: bool,
    #[serde(default)]
    enhance_contrast: bool,
    #[serde(default = "one")]
    upscale: i32,
    tasks: Vec<WorkerTask>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PdfSpec {
    path: String,
    dpi: u16,
}

fn yes() -> bool {
    true
}

fn one() -> i32 {
    1
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum WorkerTask {
    Image { id: String, path: String },
    Page { id: String, page: u32 },
}

impl WorkerTask {
    fn id(&self) -> &str {
        match self {
            WorkerTask::Image { id, .. } | WorkerTask::Page { id, .. } => id,
        }
    }
    fn page(&self) -> Option<u32> {
        match self {
            WorkerTask::Image { .. } => None,
            WorkerTask::Page { page, .. } => Some(*page),
        }
    }
}

/// `qpp-studio.exe --worker` 入口。
pub fn run_worker() {
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        eprintln!("[worker] 读请求失败");
        return;
    }
    let req: WorkerRequest = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[worker] 请求解析失败: {e}");
            return;
        }
    };

    let spec = EngineSpec {
        tier: crate::settings::parse_tier(&req.tier).unwrap_or(qppocr::Tier::Tiny),
        preset: if crate::settings::is_special_preset(&req.preset) {
            qppocr::Preset::Accuracy
        } else {
            crate::settings::parse_preset(&req.preset).unwrap_or(qppocr::Preset::Balanced)
        },
        special: crate::settings::is_special_preset(&req.preset),
        orientation: req.orientation,
        enhance_contrast: req.enhance_contrast,
        upscale: req.upscale,
    };
    let engine = match build_engine(&spec, req.threads, Path::new(&req.models_dir)) {
        Ok(e) => e,
        Err(e) => {
            // 引擎失败也要把每个任务回成错误结果,父进程不能悬等
            let out = dto::outcome_err(format!("worker 引擎加载失败: {e}"));
            respond_all(&req.tasks, out);
            return;
        }
    };

    if let Some(pdf) = &req.pdf {
        run_pdf_tasks(&req, pdf, &engine);
    } else {
        run_image_tasks(&req, &engine);
    }
}

/// PDF 页任务:文件读一次、文档解析一次,逐页渲染 + OCR。
fn run_pdf_tasks(req: &WorkerRequest, pdf: &PdfSpec, engine: &qppocr::Engine) {
    let stdout = std::io::stdout();
    let mut w = stdout.lock();

    let bytes = match std::fs::read(&pdf.path) {
        Ok(b) => b,
        Err(e) => {
            respond_all(&req.tasks, dto::outcome_err(format!("worker 读 PDF 失败: {e}")));
            return;
        }
    };
    let doc = match crate::pdf::load_doc(&bytes) {
        Ok(d) => d,
        Err(e) => {
            respond_all(&req.tasks, dto::outcome_err(format!("worker 加载 PDF 失败: {e}")));
            return;
        }
    };

    for t in &req.tasks {
        let Some(page) = t.page() else {
            continue; // 协议混用防御:PDF 请求里不该有图片任务
        };
        let outcome = match crate::pdf::render_doc_page(&doc, page, pdf.dpi) {
            Ok((w_px, h_px, rgb)) => match qppocr::rgb_from_bytes(w_px, h_px, rgb) {
                Ok(img) => dto::outcome_from(engine.run(&img)),
                Err(e) => dto::outcome_from(Err(e)),
            },
            Err(e) => dto::outcome_err(e),
        };
        let resp = serde_json::json!({
            "id": t.id(),
            "page": page,
            "outcome": serde_json::to_value(&outcome).unwrap_or(serde_json::Value::Null),
        });
        let _ = writeln!(w, "{resp}");
        let _ = w.flush();
    }
}

/// 图片任务:解码一次两用(缩略图 + 引擎输入)。
/// EXIF 方向感知解码:引擎内置解析器对部分 APP1 结构漏读(Orientation=8 实测),
/// 统一走应用侧 image_util 保证与画布显示方向一致。
fn run_image_tasks(req: &WorkerRequest, engine: &qppocr::Engine) {
    let stdout = std::io::stdout();
    let mut w = stdout.lock();
    for t in &req.tasks {
        let WorkerTask::Image { id, path } = t else {
            continue; // 协议混用防御
        };
        let (outcome, thumb_ok) = match crate::image_util::decode_file_oriented(Path::new(path)) {
            Ok(img) => {
                let thumb_ok = req
                    .thumb_dir
                    .as_deref()
                    .map(|dir| {
                        crate::thumb::make_thumb_from_rgb(
                            img.w as u32,
                            img.h as u32,
                            &img.data,
                            id,
                            Path::new(dir),
                        )
                        .is_ok()
                    })
                    .unwrap_or(false);
                let out = dto::outcome_from(engine.run(&img));
                (out, thumb_ok)
            }
            Err(e) => (dto::outcome_from(Err(e)), false),
        };
        let resp = serde_json::json!({
            "id": id,
            "thumb": thumb_ok,
            "outcome": serde_json::to_value(&outcome).unwrap_or(serde_json::Value::Null),
        });
        let _ = writeln!(w, "{resp}");
        let _ = w.flush();
    }
}

/// 兜底:把剩余任务全部回执同一条结果(页任务带 page 字段)。
fn respond_all(tasks: &[WorkerTask], outcome: dto::OcrOutcomeDto) {
    let stdout = std::io::stdout();
    let mut w = stdout.lock();
    for t in tasks {
        let mut resp = serde_json::json!({
            "id": t.id(),
            "outcome": serde_json::to_value(&outcome).unwrap_or(serde_json::Value::Null),
        });
        if let Some(page) = t.page() {
            resp["page"] = serde_json::json!(page);
        }
        let _ = writeln!(w, "{resp}");
        let _ = w.flush();
    }
}
