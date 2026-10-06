//! 多屏捕获：xcap 逐屏截取 → 24 位 BMP 冻结落盘（无压缩，4K < 100ms；
//! PNG 编码 4K 要 1-2s 不可接受）→ 注册 media 令牌，内存保留原图供选区裁剪。
//!
//! BMP 必须存 24 位（丢 alpha）：image crate 的 Rgba8 BMP 是 32 位 BI_BITFIELDS
//! 变体，WebKit(macOS WKWebView)解不了 → 覆盖层破损图占位(问号)+黑屏；
//! Chromium 无所谓，但 24 位是全平台最大公约数。

use std::path::Path;
use tauri::{AppHandle, Manager};

pub struct MonitorShot {
    pub index: u32,
    /// 全局物理坐标
    pub x: i32,
    pub y: i32,
    /// 物理尺寸
    pub w: u32,
    pub h: u32,
    pub scale: f32,
    /// 冻结 BMP 的 media 令牌
    pub token: String,
    /// 内存原图（选区裁剪用，会话结束释放）
    pub image: image::RgbaImage,
}

pub fn capture_all(app: &AppHandle, dir: &Path) -> Result<Vec<MonitorShot>, String> {
    let monitors =
        xcap::Monitor::all().map_err(|e| format!("枚举显示器失败: {e}"))?;
    let state = app.state::<crate::AppCtx>();
    let mut shots = Vec::new();
    for (i, m) in monitors.iter().enumerate() {
        let cap = m
            .capture_image()
            .map_err(|e| format!("截取显示器 {i} 失败: {e}"))?;
        // 经 raw 缓冲转换，规避 xcap 与本工程 image 版本不一致的类型分裂
        let (w, h) = (cap.width(), cap.height());
        let raw = cap.into_raw();
        let image = image::RgbaImage::from_raw(w, h, raw)
            .ok_or_else(|| format!("显示器 {i} 截图为空"))?;

        let bmp = dir.join(format!("m{i}.bmp"));
        let rgb = image::DynamicImage::ImageRgba8(image.clone())
            .to_rgb8(); // RGBA→RGB:24 位 BMP(见模块注释)
        image::save_buffer_with_format(
            &bmp,
            rgb.as_raw(),
            w,
            h,
            image::ColorType::Rgb8,
            image::ImageFormat::Bmp,
        )
        .map_err(|e| format!("写入冻结帧失败: {e}"))?;
        let token = state.media.register(bmp, "image/bmp");

        let ex = |e: xcap::XCapError| format!("读取显示器 {i} 信息失败: {e}");
        // macOS:刚授权屏幕录制但没重启应用时,捕获 API 静默返回全黑帧——
        // 给指引而不是让用户对着一整块黑屏猜(见 Info.plist NSScreenCaptureDescription)
        #[cfg(target_os = "macos")]
        if is_all_black(&image) {
            return Err(
                "屏幕捕获内容为空(全黑):若刚在系统设置里授予屏幕录制权限,请完全退出 QPP Studio 后重新打开(权限对已运行进程不生效);仍不行请到 系统设置→隐私与安全性→屏幕录制 检查".into(),
            );
        }
        shots.push(MonitorShot {
            index: i as u32,
            x: m.x().map_err(ex)?,
            y: m.y().map_err(ex)?,
            w,
            h,
            scale: m.scale_factor().map_err(ex)?,
            token,
            image,
        });
    }
    if shots.is_empty() {
        return Err("没有可用的显示器".into());
    }
    Ok(shots)
}

/// 采样判定画面是否全黑(约 1/64 像素抽点,亮度 <12 记黑)。
#[cfg(target_os = "macos")]
fn is_all_black(img: &image::RgbaImage) -> bool {
    let (w, h) = img.dimensions();
    let mut dark = 0usize;
    let mut total = 0usize;
    for y in (0..h).step_by(8) {
        for x in (0..w).step_by(8) {
            let p = img.get_pixel(x, y);
            let lum = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
            total += 1;
            if lum < 12.0 {
                dark += 1;
            }
        }
    }
    total > 0 && dark * 200 >= total * 199 // ≥99.5% 黑
}
