//! 硬件检测与并行策略优化表。
//!
//! 启动时 `detect()` 一次,之后所有并行决策(图片批量 worker 数、PDF 舰队、
//! 小批量进程内并发)统一走 `plan()`,替换掉原先按单台 18 核机调死的
//! `clamp(cores/2, 2, cap)` 启发式。
//!
//! 优化表依据(详见仓库计划文档):
//! - 市面主流 6-8 物理核(Steam 2026-08:6 核 ~29% + 8 核 ~27%),内存 16GB ~41% / 32GB ~37%
//! - 实测原则(18 核大小核机):总线程 ≈ 逻辑核时最优,且「多小 worker 优于少大 worker」
//!   (8×t2 = 3.46× > 4×t4 = 2.82×),OCR 属内存带宽受限型,worker 数超过物理核后收益归零
//! - 内存是第二道闸:每个 worker 各持引擎实例 + 整页像素工作集,峰值 RSS 见
//!   WORKER_BUDGET_*(QPP_BENCH_PDF 实测校准,预算 = 峰值 × ~1.3)

use qppocr::Tier;

const GIB: u64 = 1 << 30;

/// 单 worker 进程峰值内存预算(tiny/small);medium 模型太重不分治,无预算。
/// CPU:tiny = QPP_BENCH_PDF 实测(35页/8×2)峰值 188MB × 1.3 ≈ 0.25GiB;
/// small 按 tiny×2 估。GPU:引擎侧实测(qppocr bench,Intel Arc 核显/UMA)
/// tiny ~2.6GB / small ~1.4GB,核显显存即系统内存,×1.1 余量。
const WORKER_BUDGET_TINY: u64 = GIB / 4;
const WORKER_BUDGET_SMALL: u64 = GIB / 2;
const WORKER_BUDGET_GPU_TINY: u64 = GIB * 3 - GIB / 10;
const WORKER_BUDGET_GPU_SMALL: u64 = GIB + GIB / 2;

/// GPU 模式 worker 进程上限:引擎侧实测 tiny GPU 4 进程仍 2.7× 扩展
/// (qppocr bench:workers=4 → 20.3 张/s 为各自最佳并发),再往上无数据;
/// 真正的约束交给内存闸(每 worker GB 级,核显显存即系统内存)。
const GPU_WORKER_CAP: usize = 4;

/// 给 OS + 应用本体(UI/WebView/主进程引擎)预留的内存。
fn mem_reserve(total: u64) -> u64 {
    (2 * GIB).max(total / 4)
}

fn worker_budget(tier: Tier) -> u64 {
    match tier {
        Tier::Tiny => WORKER_BUDGET_TINY,
        Tier::Small => WORKER_BUDGET_SMALL,
        Tier::Medium => 0, // 不分治
    }
}

fn worker_budget_gpu(tier: Tier) -> u64 {
    match tier {
        Tier::Tiny => WORKER_BUDGET_GPU_TINY,
        Tier::Small => WORKER_BUDGET_GPU_SMALL,
        Tier::Medium => 0, // GPU 侧从未基准,不冒进
    }
}

#[derive(Debug, Clone)]
pub struct HwInfo {
    pub cpu_brand: String,
    pub physical_cores: usize,
    pub logical_cores: usize,
    /// 总内存(字节);0 = 探测失败(跳过内存闸)
    pub total_mem: u64,
    /// GPU 清单(qppocr-gpu Vulkan 枚举;无 loader/无设备 = 空列表)
    pub gpus: Vec<GpuInfo>,
}

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    /// API 版本串,如 "vulkan 1.4"
    pub api: String,
}

#[derive(Debug, Clone)]
pub struct ParallelPlan {
    /// worker 进程数 K;0 = 不分治(medium / 内存不足 / 自动判定不划算)
    pub workers: usize,
    /// 每个 worker 的引擎线程数 t
    pub threads_each: usize,
    /// 小批量(< PROC_BATCH_MIN)进程内并发
    pub inproc_concurrency: usize,
    /// 内存闸允许的最大 worker 数(usize::MAX = 内存未知不设限)
    pub mem_cap: usize,
    /// 是否被内存闸压低(UI 提示用)
    pub clamped_by_mem: bool,
}

pub fn detect() -> HwInfo {
    let sys = sysinfo::System::new_all();
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8);
    let physical = sys
        .physical_core_count()
        .filter(|&p| p > 0)
        .unwrap_or((logical + 1) / 2);
    let cpu_brand = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "未知 CPU".into());
    // GPU 枚举(qppocr-gpu):无 loader/无 ICD = 空列表,绝不当错误;
    // Vulkan loader 一旦装载即常驻(引擎侧设计),启动只调这一次
    let gpus = qppocr_gpu::list_devices()
        .into_iter()
        .map(|d| GpuInfo {
            name: d.name,
            api: d.api,
        })
        .collect();
    HwInfo {
        cpu_brand,
        physical_cores: physical.max(1),
        logical_cores: logical.max(1),
        total_mem: sys.total_memory(),
        gpus,
    }
}

/// 优化表(CPU):硬件 × 档位 × (可选)手动覆盖 → 并行策略。
pub fn plan(hw: &HwInfo, tier: Tier, workers_override: usize) -> ParallelPlan {
    plan_with(hw, tier, workers_override, false)
}

/// 优化表(按引擎设备):GPU 模式换用显存预算与 worker 上限。
pub fn plan_for(
    hw: &HwInfo,
    tier: Tier,
    workers_override: usize,
    device: &qppocr::DeviceChoice,
) -> ParallelPlan {
    plan_with(hw, tier, workers_override, matches!(device, qppocr::DeviceChoice::Gpu { .. }))
}

/// 优化表本体:
/// - CPU:K = min(物理核, 档位上限{tiny:8, small:4}, max(1, 逻辑核/2))
///   ——「总线程≈逻辑核 + worker 数≤物理核」两条实测原则的交集;
///   t = clamp(逻辑核/K, 1, 4)
/// - GPU:K 上限 2(GB 级显存/worker + 引擎内部已并行),
///   预算换 WORKER_BUDGET_GPU_*(核显上显存即系统内存)
/// - 内存闸:mem_cap = (总内存 - 预留) / 单 worker 预算,K 压到 mem_cap
/// - override > 0 时优先,但 medium 不解禁、内存闸仍生效(手动也拦不住 OOM)
fn plan_with(hw: &HwInfo, tier: Tier, workers_override: usize, gpu: bool) -> ParallelPlan {
    let inproc = if gpu {
        // 引擎 Vulkan 会话硬约束:同一引擎实例「同形状」流水深度 ≤2
        // (primary+shadow 计划槽),第三条并发同形状直接报错——共享引擎
        // 的进程内并发必须 ≤2
        2
    } else {
        match tier {
            Tier::Tiny => 4,
            _ => 2,
        }
    }
    .min(hw.logical_cores.max(1));

    let budget = if gpu {
        worker_budget_gpu(tier)
    } else {
        worker_budget(tier)
    };
    if budget == 0 {
        return ParallelPlan {
            workers: 0,
            threads_each: 0,
            inproc_concurrency: inproc,
            mem_cap: 0,
            clamped_by_mem: false,
        };
    }

    let mem_cap = if hw.total_mem == 0 {
        usize::MAX // 内存未知,不设限
    } else {
        (hw.total_mem.saturating_sub(mem_reserve(hw.total_mem)) / budget) as usize
    };

    let want = if workers_override > 0 {
        workers_override
    } else if gpu {
        GPU_WORKER_CAP
    } else {
        let cap = match tier {
            Tier::Tiny => 8,
            Tier::Small => 4,
            Tier::Medium => 0,
        };
        hw.physical_cores
            .min(cap)
            .min(hw.logical_cores.div_ceil(2))
            .max(1)
    };

    let k = want.min(mem_cap);
    ParallelPlan {
        workers: k,
        threads_each: if k == 0 {
            0
        } else {
            (hw.logical_cores / k).clamp(1, 4)
        },
        inproc_concurrency: inproc,
        mem_cap,
        clamped_by_mem: k < want,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qppocr::Tier::{Medium, Small, Tiny};

    fn hw(p: usize, l: usize, gb: u64) -> HwInfo {
        HwInfo {
            cpu_brand: "test".into(),
            physical_cores: p,
            logical_cores: l,
            total_mem: gb * GIB,
            gpus: Vec::new(),
        }
    }

    /// (输入, 期望 workers, threads_each, clamped_by_mem)
    #[test]
    fn table() {
        // 开发机参照:18L/12P/32G/tiny → 实测最优 8×t2
        let p = plan(&hw(12, 18, 32), Tiny, 0);
        assert_eq!((p.workers, p.threads_each), (8, 2));
        assert!(!p.clamped_by_mem);

        // 主流 6 核台式:K=min(6,8,6)=6, t=clamp(12/6)=2
        let p = plan(&hw(6, 12, 16), Tiny, 0);
        assert_eq!((p.workers, p.threads_each), (6, 2));

        // 入门 N100 4C/4T/8G:K=min(4,8,2)=2, t=2;内存闸 8-2/0.7=8 不约束
        let p = plan(&hw(4, 4, 8), Tiny, 0);
        assert_eq!((p.workers, p.threads_each), (2, 2));
        assert!(!p.clamped_by_mem);

        // 小内存机 3G/tiny(8P/16L):want=8 但闸 (3-2)/0.25=4 → 压到 4,t=clamp(16/4)=4
        let p = plan(&hw(8, 16, 3), Tiny, 0);
        assert_eq!((p.workers, p.threads_each), (4, 4));
        assert!(p.clamped_by_mem);
        // small 实测前的保守预算 0.5G:6G 机闸 (6-2)/0.5=8,不约束 want=4
        let p = plan(&hw(8, 8, 6), Small, 0);
        assert_eq!((p.workers, p.threads_each), (4, 2));
        assert!(!p.clamped_by_mem);

        // 内存不足以开任何一个 worker → 退化为不分治
        let p = plan(&hw(8, 8, 2), Tiny, 0);
        assert_eq!(p.workers, 0);
        assert!(p.clamped_by_mem);

        // medium 恒不分治,override 也不解禁
        let p = plan(&hw(16, 32, 64), Medium, 8);
        assert_eq!(p.workers, 0);
        assert_eq!(p.inproc_concurrency, 2);

        // 手动覆盖:优先于档位上限,仍受内存闸
        let p = plan(&hw(12, 18, 32), Tiny, 6);
        assert_eq!((p.workers, p.threads_each), (6, 3));
        let p = plan(&hw(8, 16, 3), Tiny, 8);
        assert_eq!(p.workers, 4);
        assert!(p.clamped_by_mem);

        // 高端 16C/32T/tiny:K=min(16,8,16)=8, t=clamp(32/8)=4(总线程=32≈逻辑核)
        let p = plan(&hw(16, 32, 64), Tiny, 0);
        assert_eq!((p.workers, p.threads_each), (8, 4));

        // 内存探测失败(total=0):跳过闸
        let mut h = hw(4, 4, 16);
        h.total_mem = 0;
        let p = plan(&h, Tiny, 0);
        assert_eq!(p.workers, 2);
        assert_eq!(p.mem_cap, usize::MAX);
        assert!(!p.clamped_by_mem);

        // 单核极端:L=1 → max(1, L/2)=1, t=clamp(1/1)=1
        let p = plan(&hw(1, 1, 4), Tiny, 0);
        assert_eq!((p.workers, p.threads_each), (1, 1));
    }

    #[test]
    fn table_gpu() {
        use qppocr::DeviceChoice;

        // GPU 封顶 4:32G 机 tiny → K=4,t=clamp(18/4)=4,inproc=2(引擎流水深度约束)
        let p = plan_for(&hw(12, 18, 32), Tiny, 0, &DeviceChoice::gpu());
        assert_eq!((p.workers, p.threads_each, p.inproc_concurrency), (4, 4, 2));
        assert!(!p.clamped_by_mem);

        // 16G 机 GPU tiny:预算 2.9G,闸 (16-4)/2.9=4 → K=4 恰好不压
        let p = plan_for(&hw(8, 16, 16), Tiny, 0, &DeviceChoice::gpu());
        assert_eq!(p.workers, 4);

        // 6G 机 GPU tiny:闸 (6-2)/2.9=1 → 压到 1
        let p = plan_for(&hw(8, 8, 6), Tiny, 0, &DeviceChoice::gpu());
        assert_eq!(p.workers, 1);
        assert!(p.clamped_by_mem);

        // medium GPU 同样不分治(引擎侧从未基准)
        let p = plan_for(&hw(16, 32, 64), Medium, 0, &DeviceChoice::gpu());
        assert_eq!(p.workers, 0);

        // CPU 路径不受影响
        let p = plan_for(&hw(12, 18, 32), Tiny, 0, &DeviceChoice::Cpu);
        assert_eq!((p.workers, p.threads_each), (8, 2));
    }
}
