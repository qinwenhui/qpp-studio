//! 自重生 worker 进程:大批量时多进程分治。
//! 引擎的进程级线程池"先到先得",单进程内并发 run 只能重叠串行段;多进程才能真正吃满核。
//!
//! 协议(一行 JSON,简单可控):
//!   stdin : {"modelsDir":..., "tier":..., "preset":..., "threads":N,
//!            "orientation":bool, "enhanceContrast":bool,
//!            "tasks":[{"id":..., "path":...}, ...]}
//!   stdout: 每完成一行 {"id":..., "outcome":{...}}(逐行 flush,父进程实时收)

use crate::dto;
use crate::engine::{build_engine, EngineSpec};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::Path;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerRequest {
    models_dir: String,
    /// 缩略图缓存目录(worker 搭识别解码的便车直接写入,主进程只注册令牌)
    #[serde(default)]
    thumb_dir: Option<String>,
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

fn yes() -> bool {
    true
}

fn one() -> i32 {
    1
}

#[derive(Deserialize, Clone)]
struct WorkerTask {
    id: String,
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkerResult {
    id: String,
    outcome: dto::OcrOutcomeDto,
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

    let stdout = std::io::stdout();
    let mut w = stdout.lock();
    for t in &req.tasks {
        // 解码一次两用:缩略图 + 引擎输入
        // EXIF 方向感知解码:引擎内置解析器对部分 APP1 结构漏读(Orientation=8 实测),
        // 统一走应用侧 image_util 保证与画布显示方向一致
        let (outcome, thumb_ok) = match crate::image_util::decode_file_oriented(Path::new(&t.path)) {
            Ok(img) => {
                let thumb_ok = req
                    .thumb_dir
                    .as_deref()
                    .map(|dir| {
                        crate::thumb::make_thumb_from_rgb(
                            img.w as u32,
                            img.h as u32,
                            &img.data,
                            &t.id,
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
            "id": t.id,
            "thumb": thumb_ok,
            "outcome": serde_json::to_value(&outcome).unwrap_or(serde_json::Value::Null),
        });
        let _ = writeln!(w, "{resp}");
        let _ = w.flush();
    }
}

fn respond_all(tasks: &[WorkerTask], outcome: dto::OcrOutcomeDto) {
    let stdout = std::io::stdout();
    let mut w = stdout.lock();
    for t in tasks {
        let resp = WorkerResult {
            id: t.id.clone(),
            outcome: outcome.clone(),
        };
        if let Ok(s) = serde_json::to_string(&resp) {
            let _ = writeln!(w, "{s}");
            let _ = w.flush();
        }
    }
}
