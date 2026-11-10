//! 构建脚本:除 Tauri 常规产物外,从 Cargo.lock 提取实际解析的 qppocr
//! 版本注入编译期环境——关于面板要展示"用的是哪个版本的引擎",跟着
//! lockfile 走升级后自动对,无需手改。

fn main() {
    // 提取失败也照常构建,显示 unknown
    let ver = lock_qppocr_version().unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=QPPOCR_ENGINE_VERSION={ver}");
    println!("cargo:rerun-if-changed=Cargo.lock");
    tauri_build::build()
}

/// 在 Cargo.lock 里找 `[[package]] name = "qppocr"` 条目的 version。
/// lockfile 格式(v3/v4)该段是稳定的 `name = "..."` / `version = "..."`
/// 平面键值对,无需完整 TOML 解析。
fn lock_qppocr_version() -> Option<String> {
    let lock = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").ok()?)
        .join("Cargo.lock");
    let text = std::fs::read_to_string(lock).ok()?;
    let mut in_target = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("[[package]]") {
            in_target = false;
        } else if line.starts_with("name = \"qppocr\"") {
            // 精确名匹配:排除 qppocr-core/gpu 等前缀同族条目
            in_target = true;
        } else if in_target && let Some(v) = line.strip_prefix("version = ") {
            return Some(v.trim_matches('"').to_string());
        }
    }
    None
}
