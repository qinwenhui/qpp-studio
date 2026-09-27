//! 多屏捕获：xcap 逐屏截取 → BMP 冻结落盘（无压缩，4K < 100ms；
//! PNG 编码 4K 要 1-2s 不可接受）→ 注册 media 令牌，内存保留原图供选区裁剪。

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
        image::save_buffer_with_format(
            &bmp,
            image.as_raw(),
            w,
            h,
            image::ColorType::Rgba8,
            image::ImageFormat::Bmp,
        )
        .map_err(|e| format!("写入冻结帧失败: {e}"))?;
        let token = state.media.register(bmp, "image/bmp");

        let ex = |e: xcap::XCapError| format!("读取显示器 {i} 信息失败: {e}");
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
