//! 设置持久化：app_data/settings.json，原子写（tmp + rename）。
//! 不用 tauri-plugin-store —— 一个 60 行的 struct 更小更可控。

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// dark-tech | light-refined | macos-glass | cute | classical
    pub theme: String,
    /// tiny | small | medium
    pub tier: String,
    /// 计算设备:cpu | gpu(GPU 需 Vulkan 1.4+;不默认开——冷启动慢、
    /// small 档有浮点末位差异、老机驱动玄学,由用户实测后选择)
    pub device: String,
    /// speed | balanced | accuracy
    pub preset: String,
    /// 0 = 自动（按可用核数）。首次构建引擎时锁定，改动重启后生效
    pub threads: usize,
    /// 全局截图快捷键
    pub hotkey: String,
    pub models_dir: Option<String>,
    /// 批量并发上限,0 = 自动(按档位:tiny=4 / 其他=2)。8 张以上自动改走多进程分治,此项不生效
    pub batch_concurrency: usize,
    /// worker 进程数手动覆盖,0 = 自动(硬件优化表:CPU 核数 + 内存闸)。
    /// 仅 tiny/small 生效;手动值仍受内存安全上限约束。对下一批生效
    pub workers_override: usize,
    /// 方向纠正（0/180 分类自动翻正；引擎默认开，略增耗时）
    pub orientation: bool,
    /// 增强对比（低对比图片提升识别；引擎 Advanced.enhance_contrast）
    pub enhance_contrast: bool,
    /// 检测放大倍数（成本 ~N²;裁剪仍取原图;引擎 Advanced.upscale）
    pub upscale: i32,
    /// 截图时是否隐藏主窗口（前端工具栏右键菜单设定;默认不隐藏）
    pub shot_hide: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "macos-glass".into(),
            tier: "tiny".into(),
            device: "cpu".into(),
            preset: "speed".into(),
            threads: 0,
            hotkey: "ctrl+shift+o".into(),
            models_dir: None,
            batch_concurrency: 0,
            workers_override: 0,
            orientation: true,
            enhance_contrast: false,
            upscale: 1,
            shot_hide: false,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Settings {
        let mut s: Settings = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // 迁移:旧默认 2 → 自动(0)。旧档里 2 从不是刻意选择,是当时的默认值
        if s.batch_concurrency == 2 {
            s.batch_concurrency = 0;
        }
        s
    }

    /// 原子写：先写 tmp 再 rename（Windows 上 rename 可覆盖已存在文件）。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub fn parse_tier(s: &str) -> Option<qppocr::Tier> {
    match s {
        "tiny" => Some(qppocr::Tier::Tiny),
        "small" => Some(qppocr::Tier::Small),
        "medium" => Some(qppocr::Tier::Medium),
        _ => None,
    }
}

/// 设备字符串 → DeviceChoice("gpu" → Vulkan 自动选择,其余 CPU)。
pub fn parse_device(s: &str) -> qppocr::DeviceChoice {
    if s == "gpu" {
        qppocr::DeviceChoice::gpu()
    } else {
        qppocr::DeviceChoice::Cpu
    }
}

pub fn tier_str(t: qppocr::Tier) -> &'static str {
    match t {
        qppocr::Tier::Tiny => "tiny",
        qppocr::Tier::Small => "small",
        qppocr::Tier::Medium => "medium",
    }
}

pub fn parse_preset(s: &str) -> Option<qppocr::Preset> {
    match s {
        "speed" => Some(qppocr::Preset::Speed),
        "balanced" => Some(qppocr::Preset::Balanced),
        "accuracy" => Some(qppocr::Preset::Accuracy),
        _ => None,
    }
}

/// "special" 预设:引擎枚举没有这档,用 精度 基底 + Advanced 覆盖实现。
pub fn is_special_preset(s: &str) -> bool {
    s == "special"
}

pub fn preset_str(p: qppocr::Preset) -> &'static str {
    match p {
        qppocr::Preset::Speed => "speed",
        qppocr::Preset::Balanced => "balanced",
        qppocr::Preset::Accuracy => "accuracy",
    }
}
