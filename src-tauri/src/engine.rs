//! 引擎管理：懒构建、catch_unwind 防 SHA panic、tier/preset 热切换（重建引擎）。
//!
//! 线程池是进程级"先到先得"：首次并行算子时定型，之后 set_threads 无效 —— 所以
//! 线程数在启动时随第一次构建锁定，UI 改线程数提示重启生效。
//! Engine 是 Send+Sync，并发 run 安全（算子层会部分串行化，见 qppocr BENCH.md §7）。

use crate::dto::EngineStatusDto;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Debug)]
pub struct EngineSpec {
    pub tier: qppocr::Tier,
    pub preset: qppocr::Preset,
    /// 方向纠正（每行多一次 0/180 分类）
    pub orientation: bool,
    /// 增强对比（Advanced.enhance_contrast）
    pub enhance_contrast: bool,
    /// 检测放大倍数（Advanced.upscale,成本 ~N²）
    pub upscale: i32,
    /// 特殊预设:精度基底 + unclip_margin_thresh 0.45 / unclip_perp 1.6
    pub special: bool,
    /// 计算设备(GPU 需引擎 gpu feature + Vulkan 1.4+;不可用时构建报错不回退)
    pub device: qppocr::DeviceChoice,
}

pub struct EngineManager {
    engine: RwLock<Option<Arc<qppocr::Engine>>>,
    spec: RwLock<EngineSpec>,
    /// 启动时锁定（进程级线程池定型后无法更改）
    pub threads: usize,
    models_dir: RwLock<PathBuf>,
    status_error: RwLock<Option<String>>,
}

impl EngineManager {
    pub fn new(threads: usize, models_dir: PathBuf) -> Self {
        Self {
            engine: RwLock::new(None),
            spec: RwLock::new(EngineSpec {
                tier: qppocr::Tier::Small,
                preset: qppocr::Preset::Balanced,
                orientation: true,
                enhance_contrast: false,
                upscale: 1,
                special: false,
                device: qppocr::DeviceChoice::default(),
            }),
            threads,
            models_dir: RwLock::new(models_dir),
            status_error: RwLock::new(None),
        }
    }

    /// 后台线程构建引擎；完成后广播 engine://status。
    pub fn spawn_init(&self, app: AppHandle, spec: EngineSpec) {
        self.reconfigure(app, spec);
    }

    /// 热切换 tier/preset/开关：后台重建，完成后原子换入（重建期间旧引擎继续服务）。
    pub fn reconfigure(&self, app: AppHandle, spec: EngineSpec) {
        {
            let mut s = self.spec.write().unwrap();
            *s = spec.clone();
        }
        let dir = self.models_dir.read().unwrap().clone();
        let threads = self.threads;
        std::thread::spawn(move || {
            let spec = spec;
            let built = build_engine(&spec, threads, &dir);
            let mgr = app.state::<crate::AppCtx>();
            match built {
                Ok(e) => {
                    *mgr.engine.engine.write().unwrap() = Some(Arc::new(e));
                    *mgr.engine.status_error.write().unwrap() = None;
                }
                Err(msg) => {
                    // 保留旧引擎可用，仅记录错误
                    *mgr.engine.status_error.write().unwrap() = Some(msg);
                }
            }
            let _ = app.emit("engine://status", mgr.engine.status());
        });
    }

    pub fn current(&self) -> Option<Arc<qppocr::Engine>> {
        self.engine.read().unwrap().clone()
    }

    pub fn spec(&self) -> EngineSpec {
        self.spec.read().unwrap().clone()
    }

    pub fn models_dir(&self) -> PathBuf {
        self.models_dir.read().unwrap().clone()
    }

    /// 切档前体检:模型文件与字典是否齐备。缺失时返回可直接展示的修复指引。
    /// 引擎字典查找顺序:{tier}/dict.txt → models/dict.txt → ppocr_keys.txt → 内嵌。
    pub fn tier_preflight(dir: &std::path::Path, tier: qppocr::Tier) -> Result<(), String> {
        let name = crate::settings::tier_str(tier);
        let tdir = dir.join(name);
        let missing: Vec<&str> = ["det.onnx", "rec.onnx"]
            .iter()
            .filter(|f| !tdir.join(f).is_file())
            .map(|f| *f)
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "「{}」档模型文件缺失:{},请把对应 onnx 放入 {} ,或运行 node tools/stage-models.mjs 重新装配",
                name,
                missing.join(" / "),
                tdir.display()
            ));
        }
        let dict_ok = tdir.join("dict.txt").is_file()
            || dir.join("dict.txt").is_file()
            || dir.join("ppocr_keys.txt").is_file();
        if !dict_ok {
            return Err(format!(
                "「{name}」档缺少字典(dict.txt),请在 {} 或模型根目录放置字典文件",
                tdir.display()
            ));
        }
        // 字典行数体检:行数错一都会在首次识别时炸字符表校验,提前拦下
        let expected_lines: usize = match tier {
            qppocr::Tier::Tiny => 6904,
            _ => 18708,
        };
        let dict_path = ["dict.txt"]
            .iter()
            .map(|f| tdir.join(f))
            .chain([dir.join("dict.txt"), dir.join("ppocr_keys.txt")])
            .find(|p| p.is_file());
        if let Some(dp) = dict_path {
            let lines = std::fs::read_to_string(&dp)
                .map(|s| s.lines().count()) // 与引擎字符表构建同口径(不滤空行)
                .unwrap_or(0);
            if lines != expected_lines {
                return Err(format!(
                    "「{name}」档字典行数 {lines} 与期望 {expected_lines} 不符({}),\
                     请用 stage-models.mjs 重新装配或更换正确的 dict.txt",
                    dp.display()
                ));
            }
        }
        Ok(())
    }

    pub fn status(&self) -> EngineStatusDto {
        let s = self.spec.read().unwrap();
        // 设备取引擎实际解析结果(未就绪时用 spec 意图)
        let device: String = {
            let eng = self.engine.read().unwrap();
            let d = eng
                .as_ref()
                .map(|e| e.device().clone())
                .unwrap_or_else(|| s.device.clone());
            match d {
                qppocr::DeviceChoice::Cpu => "cpu".into(),
                qppocr::DeviceChoice::Gpu { .. } => "gpu".into(),
                // DeviceChoice 是 non_exhaustive,未来新设备先按 CPU 展示
                _ => "cpu".into(),
            }
        };
        EngineStatusDto {
            ready: self.engine.read().unwrap().is_some(),
            error: self.status_error.read().unwrap().clone(),
            tier: crate::settings::tier_str(s.tier).into(),
            preset: crate::settings::preset_str(s.preset).into(),
            threads: self.threads,
            device,
            models_dir: self
                .models_dir
                .read()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        }
    }

    // ---- 识别入口（解码在引擎锁外）----

    pub fn run_file(&self, path: &std::path::Path) -> Result<qppocr::OcrResult, qppocr::Error> {
        let engine = self.current().ok_or_else(not_ready)?;
        let image = qppocr::decode_file(path)?;
        engine.run(&image)
    }

    pub fn run_image(&self, image: qppocr::Image) -> Result<qppocr::OcrResult, qppocr::Error> {
        let engine = self.current().ok_or_else(not_ready)?;
        engine.run(&image)
    }
}

fn not_ready() -> qppocr::Error {
    qppocr::Error::Model("引擎尚未就绪（模型加载中或加载失败，见设置）".into())
}

/// 构建引擎。SHA 校验失败在引擎侧已改为 Err,catch_unwind 保留作双保险。
pub(crate) fn build_engine(
    spec: &EngineSpec,
    threads: usize,
    dir: &std::path::Path,
) -> Result<qppocr::Engine, String> {
    let dir = dir.to_path_buf();
    let spec = spec.clone();
    let result = std::panic::catch_unwind(AssertUnwindSafe(move || {
        let enhance = spec.enhance_contrast;
        let upscale = spec.upscale;
        let special = spec.special;
        let tier = spec.tier;
        // 特殊(监控)预设:关方向分类——上游 det 的收紧框本就正立,
        // cls 反而会把裁剪转正引入干扰(引擎侧最终方案)
        let orientation = if special { false } else { spec.orientation };
        qppocr::Engine::builder()
            .tier(spec.tier)
            .preset(spec.preset)
            .threads(threads)
            .device(spec.device.clone())
            .detect_orientation(orientation)
            .advanced(move |a| {
                a.enhance_contrast = enhance;
                a.upscale = upscale;
                if special {
                    // 《特殊》= C++ 监控截图预设:杂波判定 0.45 + 垂直扩张收缩到 0.5
                    // (白字压在栏杆/栅栏上时,垂直扩张会把背景纹理吃进裁剪框)
                    a.unclip_margin_thresh = 0.45;
                    a.unclip_perp = 0.5;
                }
                // Apple Silicon:引擎的 rec 自动分片按 x86 档位标定(tiny=12),
                // M 系 4P+6E 上深切分的 fork/merge 开销盖过收益——tiny 收到 8。
                // 显式值引擎原样透传;medium 单进程不分治,保持自动。
                if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
                    a.rec_shards = match tier {
                        qppocr::Tier::Tiny => 8,
                        qppocr::Tier::Small => 4,
                        qppocr::Tier::Medium => a.rec_shards,
                        _ => a.rec_shards,
                    };
                }
            })
            .build(&dir)
    }));
    match result {
        Ok(Ok(engine)) => Ok(engine),
        Ok(Err(e)) => Err(format!("引擎构建失败：{e}")),
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "未知 panic".into());
            Err(format!("模型加载失败（panic）：{msg} — 模型文件可能损坏或被替换，请重新安装或校验 models 目录"))
        }
    }
}
