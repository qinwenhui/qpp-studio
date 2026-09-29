//! 前端 IPC 数据传输对象。qppocr 的输出结构没有 serde derive，
//! 这里做镜像映射并顺带做 IPC 瘦身（置信度 2 位小数、毫秒 1 位小数）。

use serde::{Deserialize, Serialize};

/// 单字符坐标(engine 2307909):四角点 TL/TR/BR/BL,原图坐标,与行框同约定。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CharSpanDto {
    pub text: String,
    pub pts: [[f32; 2]; 4],
}

/// 一行识别结果。`pts` 为四点四边形 TL/TR/BR/BL，原始图像坐标。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TextLineDto {
    pub text: String,
    pub confidence: f32,
    pub rotation: i32,
    pub pts: [[f32; 2]; 4],
    /// 逐字坐标(与 text 逐字符对齐;空格也有真实空白区间的框)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chars: Vec<CharSpanDto>,
    /// 本行来自区域重试的第二遍(engine 248b97a)。注意语义:来自第二遍 ≠ 必然更好,
    /// 下游对 retried && 置信度低 的行应给更强警示
    #[serde(default)]
    pub retried: bool,
}

/// 各阶段耗时（毫秒，保留 1 位小数）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct TimingsDto {
    pub det_pre_ms: f64,
    pub det_infer_ms: f64,
    pub det_post_ms: f64,
    pub crop_ms: f64,
    pub cls_ms: f64,
    pub rec_pre_ms: f64,
    pub rec_infer_ms: f64,
    pub rec_post_ms: f64,
    pub total_ms: f64,
}

/// 一次完整识别的结果。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OcrResultDto {
    pub lines: Vec<TextLineDto>,
    pub work_w: i32,
    pub work_h: i32,
    pub num_boxes: u32,
    pub num_merged: u32,
    pub num_decluttered: u32,
    pub num_det_retried: u32,
    pub num_flipped: u32,
    pub num_unread: u32,
    pub timings: TimingsDto,
}

/// 命令/事件的统一载荷：成功带 result，失败带 error。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OcrOutcomeDto {
    pub ok: bool,
    pub error: Option<String>,
    pub result: Option<OcrResultDto>,
}

/// 一张进入应用的图片。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImageItemDto {
    pub id: String,
    pub name: String,
    /// 可展示的源路径（原文件，或剪贴板/截图落盘后的 inbox 路径）
    pub path: String,
    pub w: u32,
    pub h: u32,
    /// file | clipboard | screenshot
    pub origin: String,
    pub added_at: u64,
    /// media:// 协议的访问令牌
    pub media_token: String,
    pub thumb_token: String,
    /// PDF 有文本层时可直提(跳过 OCR)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub can_extract: bool,
}

/// 引擎状态。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatusDto {
    pub ready: bool,
    pub error: Option<String>,
    pub tier: String,
    pub preset: String,
    pub threads: usize,
    pub models_dir: String,
}

/// app_init 返回的启动信息。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InitInfoDto {
    pub settings: crate::settings::Settings,
    pub engine: EngineStatusDto,
    pub version: String,
    pub platform: String,
}

/// hw_info 返回的并行策略(按当前 tier + workers_override 实时计算)。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlanDto {
    pub workers: usize,
    pub threads_each: usize,
    pub inproc_concurrency: usize,
    /// 内存闸允许的最大 worker 数(u32::MAX = 内存未知不设限)
    pub mem_cap: u32,
    pub clamped_by_mem: bool,
}

/// hw_info 返回的硬件检测信息。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HwInfoDto {
    pub cpu_brand: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    /// 总内存 GB(保留 1 位小数;0 = 探测失败)
    pub total_mem_gb: f64,
    pub plan: PlanDto,
}

/// 批量进度事件载荷。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ItemStatusDto {
    pub id: String,
    /// queued | running
    pub phase: String,
}

/// 单图完成事件载荷（单张/批量/worker 共用）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ItemDoneDto {
    pub id: String,
    pub outcome: OcrOutcomeDto,
    /// 搭识别便车生成的缩略图令牌（无则保持原状）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumb_token: Option<String>,
    /// PDF 页任务的页号(图片为 None):前端据此丢弃"晚到的非当前页"结果
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf_page: Option<u32>,
}

/// 「图片 + 识别结果」组合：截图完成事件、历史重开、结果弹窗共用。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ItemOutcomeDto {
    pub item: ImageItemDto,
    pub outcome: OcrOutcomeDto,
}

/// 轻提示。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ToastDto {
    /// info | success | error
    pub level: String,
    pub message: String,
}

/// 截图覆盖层需要的单屏信息。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ShotMonitorDto {
    /// 冻结画面的 media:// URL
    pub url: String,
    /// 物理像素尺寸
    pub w: u32,
    pub h: u32,
    /// 该屏 DPR（CSS px → 物理 px 的换算系数）
    pub dpr: f64,
}

/// 历史条目（持久化到 history.json）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntryDto {
    pub id: String,
    pub name: String,
    pub path: String,
    pub w: u32,
    pub h: u32,
    pub origin: String,
    pub at: u64,
    pub line_count: u32,
    pub total_ms: f64,
    pub text_preview: String,
    pub outcome: OcrOutcomeDto,
    /// 列表时按需注册(media 令牌不持久化)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_token: Option<String>,
}

// ---- qppocr → DTO 映射 ----

fn r2(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

pub fn outcome_from(result: Result<qppocr::OcrResult, qppocr::Error>) -> OcrOutcomeDto {
    match result {
        Ok(r) => OcrOutcomeDto {
            ok: true,
            error: None,
            result: Some(result_dto(&r)),
        },
        Err(e) => OcrOutcomeDto {
            ok: false,
            error: Some(e.to_string()),
            result: None,
        },
    }
}

pub fn outcome_err(msg: impl Into<String>) -> OcrOutcomeDto {
    OcrOutcomeDto {
        ok: false,
        error: Some(msg.into()),
        result: None,
    }
}

pub fn result_dto(r: &qppocr::OcrResult) -> OcrResultDto {
    OcrResultDto {
        lines: r
            .lines
            .iter()
            .map(|l| TextLineDto {
                text: l.text.clone(),
                confidence: r2(l.confidence),
                rotation: l.rotation,
                pts: l.pts,
                retried: l.retried,
                chars: l
                    .chars
                    .iter()
                    .map(|c| CharSpanDto {
                        text: c.text.clone(),
                        pts: c.pts,
                    })
                    .collect(),
            })
            .collect(),
        work_w: r.work_w,
        work_h: r.work_h,
        num_boxes: r.num_boxes as u32,
        num_merged: r.num_merged as u32,
        num_decluttered: r.num_decluttered as u32,
        num_det_retried: r.num_det_retried as u32,
        num_flipped: r.num_flipped as u32,
        num_unread: r.num_unread as u32,
        timings: TimingsDto {
            det_pre_ms: r1(r.timings.det_pre_ms),
            det_infer_ms: r1(r.timings.det_infer_ms),
            det_post_ms: r1(r.timings.det_post_ms),
            crop_ms: r1(r.timings.crop_ms),
            cls_ms: r1(r.timings.cls_ms),
            rec_pre_ms: r1(r.timings.rec_pre_ms),
            rec_infer_ms: r1(r.timings.rec_infer_ms),
            rec_post_ms: r1(r.timings.rec_post_ms),
            total_ms: r1(r.timings.total_ms),
        },
    }
}

pub fn text_preview(outcome: &OcrOutcomeDto, max_chars: usize) -> String {
    let mut s = String::new();
    if let Some(r) = &outcome.result {
        for l in &r.lines {
            if !l.text.is_empty() {
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str(&l.text);
                if s.chars().count() >= max_chars {
                    break;
                }
            }
        }
    }
    s.chars().take(max_chars).collect()
}

/// PDF 页面渲染结果。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PdfPageDto {
    pub media_token: String,
    pub w: u32,
    pub h: u32,
    pub page: u32,
    /// 该页是否已有识别结果(按需模式前端据此触发单页识别)
    pub recognized: bool,
}

/// PDF 文本直提结果(每页)。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PdfExtractedPageDto {
    pub page: u32,
    pub text: String,
    pub line_count: u32,
}
