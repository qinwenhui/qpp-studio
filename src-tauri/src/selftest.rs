//! 自测模式(QPP_SELFTEST=1):无人工介入跑通全链路,输出 JSON 摘要后退出。
//! 覆盖:引擎构建与耗时、ingest(缩略图/令牌)、OCR 全流水线、media:// 协议
//! 命中(经截图覆盖窗真实请求)、历史落盘。
//!
//! 用法:`QPP_SELFTEST=1 npm run tauri dev`(需要 vite 在跑,覆盖窗页面由其提供)。

use std::path::PathBuf;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// 基准日志:stdout + 文件双写(release 的 windows 子系统没有控制台,println 会丢)。
macro_rules! blog {
    ($($arg:tt)*) => {{
        let s = format!($($arg)*);
        println!("{s}");
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::temp_dir().join("qpp-bench.log"))
        {
            let _ = writeln!(f, "{s}");
        }
    }};
}

pub fn spawn(app: AppHandle) {
    // PDF 基准模式:QPP_BENCH_PDF=<pdf路径> npm run tauri dev
    // 串行 vs 舰队整册计时 + worker 峰值 RSS 采样(校准 hw.rs 内存预算)
    if let Ok(pdf) = std::env::var("QPP_BENCH_PDF") {
        std::thread::spawn(move || bench_pdf(app, &pdf));
        return;
    }
    // 基准模式:QPP_BENCH=<图片目录> npm run tauri dev
    if let Ok(dir) = std::env::var("QPP_BENCH") {
        std::thread::spawn(move || bench(app, &dir));
        return;
    }
    std::thread::spawn(move || {
        run(app);
    });
}

/// 三种批量模式实测:串行 / 进程内并发4 / 进程分治K——给调优出硬数字。
fn bench(app: AppHandle, dir: &str) {
    use std::time::Instant;
    let _ = std::fs::remove_file(std::env::temp_dir().join("qpp-bench.log"));
    blog!("[bench] === QPP Studio 批量基准 ===");
    let state = app.state::<crate::AppCtx>();
    let spec = state.engine.spec();
    let models_dir = state.engine.models_dir();
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
    let workers_override = state.settings.read().unwrap().workers_override;
    let plan = crate::hw::plan(&state.hw, spec.tier, workers_override);
    blog!(
        "[bench] hw: {} · P{}/L{} · {:.1}GB → {}进程×{}线程",
        state.hw.cpu_brand,
        state.hw.physical_cores,
        state.hw.logical_cores,
        state.hw.total_mem as f64 / 1_073_741_824.0,
        plan.workers,
        plan.threads_each
    );

    let mut images: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("读目录失败")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase()),
                Some(ref s) if matches!(s.as_str(), "jpg" | "jpeg" | "png" | "bmp")
            )
        })
        .collect();
    images.sort();
    images.truncate(100);
    let n = images.len();
    if n == 0 {
        blog!("[bench] FAIL 目录里没有图片: {dir}");
        std::process::exit(1);
    }
    let total_mp: f64 = images
        .iter()
        .filter_map(|p| qppocr::probe_dimensions(p).ok())
        .map(|(w, h)| (w as f64) * (h as f64) / 1e6)
        .sum();
    blog!(
        "[bench] {n} 张图 / {total_mp:.1} MP · tier={} · {} 核",
        crate::settings::tier_str(spec.tier),
        cores
    );

    // 共享引擎(线程池自动,与交互模式一致)
    let engine = match crate::engine::build_engine(&spec, 0, &models_dir) {
        Ok(e) => e,
        Err(e) => {
            blog!("[bench] FAIL 引擎: {e}");
            std::process::exit(1);
        }
    };
    // 热身
    for p in images.iter().take(3) {
        let _ = engine.run_image_file(p);
    }

    // A) 串行(基线,单轮)
    let t0 = Instant::now();
    for p in &images {
        let _ = engine.run_image_file(p);
    }
    let serial = t0.elapsed().as_secs_f64();

    // B) 进程内并发 4(参照)
    let eng = std::sync::Arc::new(engine);
    let queue = std::sync::Mutex::new(images.clone());
    let t0 = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..4 {
            let eng = eng.clone();
            let queue = &queue;
            s.spawn(move || loop {
                let p = { queue.lock().unwrap().pop() };
                let Some(p) = p else { return };
                let _ = eng.run_image_file(&p);
            });
        }
    });
    let conc4 = t0.elapsed().as_secs_f64();

    // C) 进程分治(当前产品配置:硬件优化表)
    let k = plan.workers.max(1);
    let t_each = plan.threads_each.max(1);
    let mut proc_rounds = Vec::new();
    for round in 1..=2 {
        let mut chunks: Vec<Vec<PathBuf>> = vec![Vec::new(); k];
        for (i, p) in images.iter().enumerate() {
            chunks[i % k].push(p.clone());
        }
        let t0 = Instant::now();
        let models_dir2 = models_dir.clone();
        std::thread::scope(|s| {
            for chunk in chunks {
                let models_dir = models_dir2.clone();
                let spec = spec;
                s.spawn(move || {
                    bench_worker_chunk(&models_dir, spec, t_each, chunk);
                });
            }
        });
        let secs = t0.elapsed().as_secs_f64();
        proc_rounds.push(secs);
        blog!("[bench] 第{round}轮 进程×{k}(t={t_each}): {secs:.2}s");
    }
    proc_rounds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let procs = proc_rounds[proc_rounds.len() / 2];

    // D) 端到端真实链路:零解码入库 → spawn_proc_batch(缩略图/历史/事件全含)
    let t0 = Instant::now();
    let mut e2e_tasks: Vec<(String, PathBuf, u64)> = Vec::new();
    for p in &images {
        if let Ok(dto) = crate::ingest::ingest_file(&app, p, "file") {
            e2e_tasks.push((
                dto.id,
                PathBuf::from(&dto.path),
                (dto.w as u64) * (dto.h as u64),
            ));
        }
    }
    let ingest_s = t0.elapsed().as_secs_f64();
    state.batch.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
    state.batch.running.store(true, std::sync::atomic::Ordering::SeqCst);
    let t1 = Instant::now();
    crate::batch::spawn_proc_batch(app.clone(), e2e_tasks, plan);
    while state
        .batch
        .running
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    let e2e_batch = t1.elapsed().as_secs_f64();
    state.history.flush_if_dirty();

    blog!("[bench] ─── 结果(含每张解码,墙钟) ───");
    blog!(
        "[bench] 串行           {serial:>6.2}s  {:.1} 张/s",
        n as f64 / serial
    );
    blog!(
        "[bench] 进程内×4       {conc4:>6.2}s  {:.1} 张/s",
        n as f64 / conc4
    );
    blog!(
        "[bench] 进程×{k}(t={t_each})  {procs:>6.2}s  {:.1} 张/s  (最优 {:.2}s)",
        n as f64 / procs,
        proc_rounds[0]
    );
    blog!(
        "[bench] 端到端: 入库 {ingest_s:.2}s + 识别 {e2e_batch:.2}s = 总 {:.2}s  (真实链路,含缩略图/历史/事件)",
        ingest_s + e2e_batch
    );
    blog!("[bench] === BENCH DONE ===");
    std::process::exit(0);
}

fn bench_worker_chunk(
    models_dir: &std::path::Path,
    spec: crate::engine::EngineSpec,
    threads: usize,
    chunk: Vec<std::path::PathBuf>,
) {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe().expect("exe");
    let mut child = Command::new(exe)
        .arg("--worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn worker");
    let request = serde_json::json!({
        "modelsDir": models_dir.to_string_lossy(),
        "tier": crate::settings::tier_str(spec.tier),
        "preset": crate::settings::preset_str(spec.preset),
        "threads": threads,
        "orientation": spec.orientation,
        "enhanceContrast": spec.enhance_contrast,
        "upscale": spec.upscale,
        "tasks": chunk.iter().map(|p| serde_json::json!({
            "id": p.file_name().unwrap_or_default().to_string_lossy(),
            "path": p.to_string_lossy(),
        })).collect::<Vec<_>>(),
    });
    let mut stdin = child.stdin.take().unwrap();
    let _ = writeln!(stdin, "{request}");
    let _ = stdin.flush();
    drop(stdin);
    if let Some(stdout) = child.stdout.take() {
        for _ in std::io::BufReader::new(stdout).lines() {}
    }
    let _ = child.wait();
}

/// 采样本进程之外同名进程(worker)的 RSS:返回 (单进程峰值, 合计峰值)。
/// 用于校准 hw.rs 的单 worker 内存预算。
fn spawn_rss_sampler(stop: &std::sync::Arc<std::sync::atomic::AtomicBool>) -> (
    std::sync::Arc<std::sync::atomic::AtomicU64>,
    std::sync::Arc<std::sync::atomic::AtomicU64>,
) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let peak_one = std::sync::Arc::new(AtomicU64::new(0));
    let peak_sum = std::sync::Arc::new(AtomicU64::new(0));
    let (one, sum) = (peak_one.clone(), peak_sum.clone());
    let stop = stop.clone();
    std::thread::spawn(move || {
        let self_pid = std::process::id();
        let mut sys = sysinfo::System::new();
        while !stop.load(Ordering::Relaxed) {
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
            let mut total = 0u64;
            let mut max = 0u64;
            for (pid, proc_) in sys.processes() {
                let name = proc_.name().to_string_lossy().to_lowercase();
                if name.contains("qpp-studio") && pid.as_u32() != self_pid {
                    let rss = proc_.memory();
                    total += rss;
                    max = max.max(rss);
                }
            }
            sum.fetch_max(total, Ordering::Relaxed);
            one.fetch_max(max, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(500));
        }
    });
    (peak_one, peak_sum)
}

/// PDF 整册基准:串行(旧产品路径) vs 舰队(新路径),附 worker 峰值 RSS 采样。
/// 用法:QPP_BENCH_PDF=<pdf路径> npm run tauri dev,输出到 qpp-bench.log。
fn bench_pdf(app: AppHandle, pdf_path: &str) {
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let _ = std::fs::remove_file(std::env::temp_dir().join("qpp-bench.log"));
    blog!("[bench-pdf] === QPP Studio PDF 并行基准 ===");
    let state = app.state::<crate::AppCtx>();
    let spec = state.engine.spec();
    let models_dir = state.engine.models_dir();
    let workers_override = state.settings.read().unwrap().workers_override;
    let plan = crate::hw::plan(&state.hw, spec.tier, workers_override);
    blog!(
        "[bench-pdf] hw: {} · P{}/L{} · {:.1}GB → {}进程×{}线程",
        state.hw.cpu_brand,
        state.hw.physical_cores,
        state.hw.logical_cores,
        state.hw.total_mem as f64 / 1_073_741_824.0,
        plan.workers,
        plan.threads_each
    );

    let path = PathBuf::from(pdf_path);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            blog!("[bench-pdf] FAIL 读 PDF 失败: {e}");
            std::process::exit(1);
        }
    };
    let sizes = match crate::pdf::page_sizes(&bytes) {
        Ok(s) => s,
        Err(e) => {
            blog!("[bench-pdf] FAIL 解析失败: {e}");
            std::process::exit(1);
        }
    };
    let pages = sizes.len() as u32;
    let weights: Vec<u64> = sizes
        .iter()
        .map(|&pt| crate::pdf::page_weight(pt, crate::pdf::DPI_OCR))
        .collect();
    blog!(
        "[bench-pdf] {pages} 页 · tier={} · preset={}",
        crate::settings::tier_str(spec.tier),
        crate::settings::preset_str(spec.preset)
    );

    // A) 串行(旧产品路径:主进程单线程逐页 渲染+共享引擎)
    let engine = match crate::engine::build_engine(&spec, 0, &models_dir) {
        Ok(e) => e,
        Err(e) => {
            blog!("[bench-pdf] FAIL 引擎: {e}");
            std::process::exit(1);
        }
    };
    let doc = crate::pdf::load_doc(&bytes).expect("load_doc");
    // 热身 1 页
    if let Ok((w, h, rgb)) = crate::pdf::render_doc_page(&doc, 0, crate::pdf::DPI_OCR) {
        if let Ok(img) = qppocr::rgb_from_bytes(w, h, rgb) {
            let _ = engine.run(&img);
        }
    }
    let t0 = Instant::now();
    for page in 0..pages {
        let _ = (|| {
            let (w, h, rgb) = crate::pdf::render_doc_page(&doc, page, crate::pdf::DPI_OCR)?;
            let img = qppocr::rgb_from_bytes(w, h, rgb).map_err(|e| e.to_string())?;
            let _ = engine.run(&img);
            Ok::<(), String>(())
        })();
    }
    let serial = t0.elapsed().as_secs_f64();
    blog!("[bench-pdf] 串行: {serial:.2}s");

    // B) 舰队 ×2 取中位 + RSS 采样
    let mut rounds = Vec::new();
    let mut peak_one_final = 0u64;
    let mut peak_sum_final = 0u64;
    for round in 1..=2 {
        let id = format!("benchpdf-r{round}");
        crate::ingest::PDF_STORE.lock().unwrap().insert(id.clone(), path.clone());
        crate::ingest::PDF_PAGES.lock().unwrap().insert(
            id.clone(),
            crate::ingest::PdfMeta {
                count: pages,
                on_demand: false,
                weights: weights.clone(),
            },
        );
        crate::ingest::PDF_RESULTS
            .lock()
            .unwrap()
            .insert(id.clone(), BTreeMap::new());
        let job = crate::ingest::PdfOcrJob {
            id: id.clone(),
            name: "bench".into(),
            src_path: path.clone(),
            pages: (0..pages).collect(),
            total: pages,
            weights: weights.clone(),
            extract: false,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let (peak_one, peak_sum) = spawn_rss_sampler(&stop);
        let t0 = Instant::now();
        crate::ingest::run_pdf_fleet(&app, &job, &plan);
        let secs = t0.elapsed().as_secs_f64();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let one = peak_one.load(std::sync::atomic::Ordering::Relaxed);
        let sum = peak_sum.load(std::sync::atomic::Ordering::Relaxed);
        peak_one_final = peak_one_final.max(one);
        peak_sum_final = peak_sum_final.max(sum);
        blog!(
            "[bench-pdf] 第{round}轮 舰队 {}×{}: {secs:.2}s · worker RSS 单{:.0}MB/合计{:.0}MB",
            plan.workers,
            plan.threads_each,
            one as f64 / 1_048_576.0,
            sum as f64 / 1_048_576.0
        );
        rounds.push(secs);
        // 清理本伪造条目
        crate::ingest::PDF_STORE.lock().unwrap().remove(&id);
        crate::ingest::PDF_PAGES.lock().unwrap().remove(&id);
        crate::ingest::PDF_RESULTS.lock().unwrap().remove(&id);
    }
    rounds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let fleet = rounds[rounds.len() / 2];

    blog!(
        "[bench-pdf] 串行 {serial:.2}s vs 舰队 {fleet:.2}s → 加速比 {:.2}×",
        serial / fleet
    );
    blog!(
        "[bench-pdf] worker 峰值RSS: 单进程 {:.0}MB · 合计 {:.0}MB → hw.rs WORKER_BUDGET_{} 建议改为 {:.2} GiB(峰值×1.3)",
        peak_one_final as f64 / 1_048_576.0,
        peak_sum_final as f64 / 1_048_576.0,
        crate::settings::tier_str(spec.tier).to_uppercase(),
        peak_one_final as f64 * 1.3 / 1_073_741_824.0
    );
    blog!("[bench-pdf] === BENCH DONE ===");
    std::process::exit(0);
}

fn run(app: AppHandle) {
    let t_total = Instant::now();
    println!("[selftest] === QPP Studio 自测开始 ===");

    // 1) 等引擎就绪(最长 120s:dev 首次编译模型解析可能慢)
    let state = app.state::<crate::AppCtx>();
    let t0 = Instant::now();
    let mut engine_err = String::new();
    loop {
        if state.engine.current().is_some() {
            break;
        }
        if let Some(e) = state.engine.status().error {
            engine_err = e;
            break;
        }
        if t0.elapsed() > Duration::from_secs(120) {
            engine_err = "等待引擎超时(120s)".into();
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    if !engine_err.is_empty() {
        println!("[selftest] FAIL engine: {engine_err}");
        return finish(false);
    }
    let status = state.engine.status();
    println!(
        "[selftest] engine ready in {:.1}s (tier={}, preset={}, threads={}, dir={})",
        t0.elapsed().as_secs_f64(),
        status.tier,
        status.preset,
        status.threads,
        status.models_dir
    );

    // 2) 造一张测试图:白底 + 三条黑杠(检测器眼中的"文本行")
    let (w, h) = (960u32, 540u32);
    let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([248, 248, 244]));
    for (i, bar) in [90u32, 210, 330].iter().enumerate() {
        let x0 = 120 + (i as u32 % 2) * 60;
        for y in *bar..*bar + 46 {
            for x in x0..x0 + 520 {
                img.put_pixel(x, y, image::Rgb([16, 18, 22]));
            }
        }
    }
    let test_path = state.dirs.cache.join("selftest-input.png");
    img.save_with_format(&test_path, image::ImageFormat::Png)
        .expect("写测试图失败");

    // 3) ingest
    let dto = match crate::ingest::ingest_file(&app, &test_path, "file") {
        Ok(d) => d,
        Err(e) => {
            println!("[selftest] FAIL ingest: {e}");
            return finish(false);
        }
    };
    println!(
        "[selftest] ingest ok: id={} {}x{} token={}..{}",
        dto.id, dto.w, dto.h,
        &dto.media_token[..6.min(dto.media_token.len())],
        &dto.thumb_token[..6.min(dto.thumb_token.len())]
    );

    // 4) OCR 全流水线(跑两遍取第二遍,避开冷启动)
    let t1 = Instant::now();
    let _cold_outcome = crate::batch::run_item_blocking(&app, &dto.id);
    let cold_ms = t1.elapsed().as_secs_f64() * 1000.0;
    let out2 = crate::batch::run_item_blocking(&app, &dto.id);
    let warm = out2
        .result
        .as_ref()
        .map(|r| (r.timings.total_ms, r.lines.len(), r.num_boxes))
        .unwrap_or((0.0, 0, 0));
    println!(
        "[selftest] ocr: cold={cold_ms:.0}ms warm={:.1}ms lines={} boxes={} ok={} chars={}",
        warm.0,
        warm.1,
        warm.2,
        out2.ok,
        out2.result
            .as_ref()
            .map(|r| r.lines.iter().map(|l| l.chars.len()).sum::<usize>())
            .unwrap_or(0)
    );
    if let Some(e) = &out2.error {
        println!("[selftest] ocr error: {e}");
    }

    // 5) media:// 协议:开截图覆盖窗(页面会真实请求冻结 BMP),再取消
    let hits_before = state.media.hits();
    crate::screenshot::begin(app.clone());
    std::thread::sleep(Duration::from_millis(2500));
    let hits = state.media.hits();
    crate::screenshot::cancel(&app);
    println!(
        "[selftest] media protocol: hits +{} (overlay 请求{}命中)",
        hits - hits_before,
        if hits > hits_before { "" } else { "未" }
    );

    // 6) 驱动真实 UI:注入 3 张全新图(每次 ingest 产生新 id,同真实"打开图片"),
    //    走完整"入库→替换当前识别→批量队列→进度→结果"流
    let mut dtos = Vec::new();
    for _ in 0..3 {
        if let Ok(d) = crate::ingest::ingest_file(&app, &test_path, "file") {
            dtos.push(d);
        }
    }
    let _ = app.emit("app://add-items", dtos);
    std::thread::sleep(Duration::from_millis(3500));

    // 6b) 用户真实路径:JPEG + add_files 命令的 invoke 返回通道(拖拽/打开共用)
    let jpg_path = state.dirs.cache.join("selftest-real.jpg");
    {
        let dyn_img = image::DynamicImage::ImageRgb8(
            image::imageops::resize(&img, 640, 360, image::imageops::FilterType::Triangle),
        );
        let f = std::fs::File::create(&jpg_path).expect("写 jpg 失败");
        let mut w = std::io::BufWriter::new(f);
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 90);
        dyn_img.write_with_encoder(enc).expect("编码 jpg 失败");
    }
    let hits_before_cmd = state.media.hits();
    let _ = app.emit(
        "app://selftest-open",
        vec![jpg_path.to_string_lossy().into_owned()],
    );
    std::thread::sleep(Duration::from_millis(3000));
    println!(
        "[selftest] add_files 通道: media hits +{}(经 invoke 返回的条目{}加载了图片)",
        state.media.hits() - hits_before_cmd,
        if state.media.hits() > hits_before_cmd { "" } else { "未" }
    );

    // 7) 历史
    let list = state.history.list(0, 5);
    println!("[selftest] history entries: {}", list.len());
    state.history.flush_if_dirty();

    // 7b) PDF 渲染验证(如果测试 PDF 存在)
    let test_pdf = std::path::Path::new("D:/qinwh/idea-2026.2.1.win/help/ReferenceCard.pdf");
    if test_pdf.exists() {
        blog!("[selftest] PDF 测试: {}", test_pdf.display());
        match std::fs::read(test_pdf) {
            Ok(bytes) => {
                match crate::pdf::page_count(&bytes) {
                    Ok(n) => blog!("[selftest] PDF 页数: {}", n),
                    Err(e) => blog!("[selftest] PDF 页数失败: {}", e),
                }
                blog!("[selftest] PDF 文本层: {}", crate::pdf::has_text_layer(&bytes));
                match crate::pdf::render_page(&bytes, 0, crate::pdf::DPI_OCR) {
                    Ok((w, h, rgb)) => blog!("[selftest] PDF 首页渲染: {}x{} ({}KB)", w, h, rgb.len() / 1024),
                    Err(e) => blog!("[selftest] PDF 渲染失败: {}", e),
                }
            }
            Err(e) => blog!("[selftest] PDF 读取失败: {}", e),
        }

        // 7c) PDF 端到端并行识别:真实队列→舰队(QPP_PDF_FLEET_FORCE 强制小册
        // 也走舰队)→逐页回执→PDF_RESULTS 完备性。
        // unsafe(set_var):edition 2024 要求;此刻队列线程仅在本任务的
        // run_pdf_job 里读该变量,设置→移除窗口内无并发读者
        unsafe { std::env::set_var("QPP_PDF_FLEET_FORCE", "1") };
        match crate::ingest::ingest_file(&app, test_pdf, "file") {
            Ok(dto) => {
                let id = dto.id.clone();
                let pages = crate::ingest::PDF_PAGES
                    .lock()
                    .unwrap()
                    .get(&id)
                    .map(|m| m.count)
                    .unwrap_or(0);
                let t0 = Instant::now();
                let mut got = 0usize;
                while t0.elapsed() < Duration::from_secs(120) {
                    got = crate::ingest::PDF_RESULTS
                        .lock()
                        .unwrap()
                        .get(&id)
                        .map(|m| m.len())
                        .unwrap_or(0);
                    if pages > 0 && got >= pages as usize {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(300));
                }
                blog!(
                    "[selftest] PDF 并行识别: {}/{} 页 · {:.1}s · 结果完备={}",
                    got,
                    pages,
                    t0.elapsed().as_secs_f64(),
                    pages > 0 && got >= pages as usize
                );
                // 清理,不影响后续人工检查
                app.state::<crate::AppCtx>().remove_item(&id);
            }
            Err(e) => blog!("[selftest] PDF ingest 失败: {e}"),
        }
        unsafe { std::env::remove_var("QPP_PDF_FLEET_FORCE") };
    }

    // 8) 留 12s 给外部截屏取证,然后退出
    std::thread::sleep(Duration::from_millis(12000));

    let ok = out2.ok && hits > hits_before && !list.is_empty();
    println!(
        "[selftest] === {} (总耗时 {:.1}s) ===",
        if ok { "PASS" } else { "FAIL" },
        t_total.elapsed().as_secs_f64()
    );
    finish(ok);
}

fn finish(_ok: bool) {
    std::process::exit(0);
}
