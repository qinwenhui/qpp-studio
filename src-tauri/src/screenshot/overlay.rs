//! 覆盖窗口生命周期：每屏一个冻结窗（Physical 定位，多 DPI 关键），
//! 以及选区旁的迷你结果弹窗。

use super::capture::MonitorShot;
use tauri::{AppHandle, Manager};

/// 创建所有覆盖窗。先隐藏构建 → Physical 定位/尺寸 → 再显示，避免尺寸跳闪。
pub fn create_windows(app: &AppHandle, shots: &[MonitorShot]) -> Result<Vec<String>, String> {
    let mut labels = Vec::new();
    for m in shots {
        let label = format!("shot-{}", m.index);
        let win = tauri::WebviewWindowBuilder::new(
            app,
            &label,
            tauri::WebviewUrl::App("screenshot.html".into()),
        )
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible(false)
        .build()
        .map_err(|e| format!("创建截图覆盖窗口失败: {e}"))?;
        let _ = win.set_position(tauri::PhysicalPosition::new(m.x, m.y));
        let _ = win.set_size(tauri::PhysicalSize::new(m.w, m.h));
        let _ = win.show();
        let _ = win.set_focus();
        labels.push(label);
    }
    Ok(labels)
}

pub fn close_windows(app: &AppHandle, labels: &[String]) {
    for label in labels {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.close();
        }
    }
}

/// 结果弹窗的摆放锚点（全部物理像素）。
pub struct PopupAnchor {
    /// 选区右下角（全局）
    pub x: i32,
    pub y: i32,
    /// 所在显示器
    pub mon_x: i32,
    pub mon_y: i32,
    pub mon_w: u32,
    pub mon_h: u32,
    pub scale: f32,
}

/// 在选区旁弹出迷你结果窗。偏好右下方，放不下则翻转/夹回屏内。
pub fn show_result_popup(app: &AppHandle, anchor: &PopupAnchor) {
    if let Some(w) = app.get_webview_window("shot-result") {
        let _ = w.close();
    }
    let win = match tauri::WebviewWindowBuilder::new(
        app,
        "shot-result",
        tauri::WebviewUrl::App("shot-result.html".into()),
    )
    .decorations(false)
    .resizable(false)
    .skip_taskbar(true)
    .always_on_top(true)
    .transparent(true)
    .visible(false)
    .inner_size(400.0, 300.0)
    .build()
    {
        Ok(w) => w,
        Err(_) => return,
    };

    let scale = anchor.scale as f64;
    let (ww, wh) = (400.0 * scale, 300.0 * scale);
    let mon_right = (anchor.mon_x + anchor.mon_w as i32) as f64;
    let mon_bottom = (anchor.mon_y + anchor.mon_h as i32) as f64;
    let margin = 12.0 * scale;

    let mut px = anchor.x as f64 + margin;
    let mut py = anchor.y as f64 + margin;
    if px + ww > mon_right {
        // 右边放不下：放到选区左侧（锚点 x 是选区右缘，退 ww + 2*margin）
        px = (anchor.x as f64) - ww - margin;
    }
    if px < anchor.mon_x as f64 {
        px = anchor.mon_x as f64 + margin;
    }
    if py + wh > mon_bottom {
        py = (anchor.y as f64) - wh - margin;
    }
    if py < anchor.mon_y as f64 {
        py = anchor.mon_y as f64 + margin;
    }
    // 最终夹回屏内
    let px = px.clamp(anchor.mon_x as f64, (mon_right - ww).max(anchor.mon_x as f64));
    let py = py.clamp(anchor.mon_y as f64, (mon_bottom - wh).max(anchor.mon_y as f64));

    let _ = win.set_position(tauri::PhysicalPosition::new(px as i32, py as i32));
    let _ = win.show();
    let _ = win.set_focus();
}
