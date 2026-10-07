//! 截图 OCR 状态机：热键 → 冻结多屏 → 覆盖窗拖框 → 内存裁剪 → 识别 → 结果。
//!
//! "冻结"用不透明 BMP 覆盖窗实现（Snipaste 式），避开透明窗口穿透的性能坑。
//! 选区在**内存中的 RgbaImage** 上裁剪（不回读磁盘），RGBA→RGB 后喂引擎。

pub mod capture;
pub mod overlay;

use crate::dto::{self, ItemOutcomeDto};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

pub struct Session {
    pub shots: Vec<capture::MonitorShot>,
    pub labels: Vec<String>,
    pub dir: std::path::PathBuf,
    /// 本次截图前隐藏了主窗口(finish/cancel 时负责恢复)
    pub hide_main: bool,
}

/// 由 AppCtx 持有。
pub type SessionSlot = Mutex<Option<Session>>;

/// 开始一次截图。幂等：已有会话先取消。
/// xcap 的 WGC 捕获需要在自有线程跑(命令线程/热键主线程上执行会卡死消息泵 → 未响应)。
pub fn begin(app: AppHandle) {
    std::thread::spawn(move || begin_sync(app));
}

fn begin_sync(app: AppHandle) {
    cancel(&app);
    // 是否隐藏主窗口:由前端在触发截图前决定(工具栏右键菜单选择),
    // 这里只执行(设置里 shot_hide=true 时隐藏)。窗口已不可见则跳过。
    let hide_main = maybe_hide_main(&app);
    let state = app.state::<crate::AppCtx>();
    let dir = state.dirs.shots.join(uuid::Uuid::new_v4().simple().to_string());

    // 错误路径也要恢复主窗口:否则截图失败后窗口永久消失,只能重启应用。
    // 成功路径由 finish/cancel 负责恢复。
    let cleanup = || {
        if hide_main {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
            }
        }
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        emit_error(&app, &format!("创建截图临时目录失败: {e}"));
        cleanup();
        return;
    }

    let shots = match capture::capture_all(&app, &dir) {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            emit_error(&app, &e);
            cleanup();
            return;
        }
    };

    let labels = match overlay::create_windows(&app, &shots) {
        Ok(l) => l,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            emit_error(&app, &e);
            cleanup();
            return;
        }
    };

    *state.shot.lock().unwrap() = Some(Session { shots, labels, dir, hide_main });
    let _ = app.emit("shot://began", ());
}

/// 覆盖层提交选区（物理像素、显示器局部坐标）。
#[allow(clippy::too_many_arguments)]
pub fn finish(app: AppHandle, mon: u32, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
    let state = app.state::<crate::AppCtx>();
    let session = state
        .shot
        .lock()
        .unwrap()
        .take()
        .ok_or("没有进行中的截图会话")?;
    overlay::close_windows(&app, &session.labels);
    if session.hide_main {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.show();
        }
    }

    let m = session
        .shots
        .get(mon as usize)
        .ok_or_else(|| "无效的显示器索引".to_string())?;
    if w < 4 || h < 4 {
        let _ = std::fs::remove_dir_all(&session.dir);
        return Err("选区太小".into());
    }
    // 防越界：夹回屏幕内
    let x = x.clamp(0, m.w as i32 - 4);
    let y = y.clamp(0, m.h as i32 - 4);
    let w = w.min(m.w as i32 - x).max(4);
    let h = h.min(m.h as i32 - y).max(4);

    let crop = image::imageops::crop_imm(&m.image, x as u32, y as u32, w as u32, h as u32)
        .to_image();

    // 弹窗锚点：选区右下角（全局物理坐标）+ 所在屏幕范围，用于摆放结果小窗
    let anchor = overlay::PopupAnchor {
        x: m.x + x + w,
        y: m.y + y,
        mon_x: m.x,
        mon_y: m.y,
        mon_w: m.w,
        mon_h: m.h,
        scale: m.scale,
    };
    let dir = session.dir.clone();
    // session 在此 drop：释放全屏 RGBA 内存
    drop(session);

    std::thread::spawn(move || {
        let _ = std::fs::remove_dir_all(&dir);
        // RGBA → RGB（引擎吃 3 通道）
        let mut rgb = Vec::with_capacity(crop.len() / 4 * 3);
        for px in crop.pixels() {
            rgb.extend_from_slice(&[px[0], px[1], px[2]]);
        }
        let qimg = match qppocr::rgb_from_bytes(w as u32, h as u32, rgb) {
            Ok(i) => i,
            Err(e) => {
                emit_error(&app, &format!("截图数据无效: {e}"));
                return;
            }
        };
        let item = match crate::ingest::ingest_bitmap(&app, "截图.png", crop, "screenshot") {
            Ok(i) => i,
            Err(e) => {
                emit_error(&app, &e);
                return;
            }
        };
        let state = app.state::<crate::AppCtx>();
        let thumb = crate::batch::ensure_thumb(&app, &item.id, &qimg);
        let outcome = dto::outcome_from(state.engine.run_image(qimg));
        crate::batch::finalize(&app, &item.id, &outcome, thumb, None);
        let payload = ItemOutcomeDto {
            item: item.clone(),
            outcome: outcome.clone(),
        };
        *state.last_shot.lock().unwrap() = Some(payload.clone());
        let _ = app.emit("shot://finished", &payload);
        overlay::show_result_popup(&app, &anchor);
    });
    Ok(())
}

/// Esc 取消。
pub fn cancel(app: &AppHandle) {
    let state = app.state::<crate::AppCtx>();
    if let Some(session) = state.shot.lock().unwrap().take() {
        overlay::close_windows(app, &session.labels);
        if session.hide_main {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
            }
        }
        let _ = std::fs::remove_dir_all(&session.dir);
        let _ = app.emit("shot://cancelled", ());
    }
}

fn emit_error(app: &AppHandle, msg: &str) {
    let _ = app.emit("app://toast", crate::dto::ToastDto {
        level: "error".into(),
        message: msg.into(),
    });
}

/// 按设置隐藏主窗口(shot_hide=true 且窗口当前可见)。
/// 返回是否已经发出隐藏请求(finish/cancel 负责恢复)。
fn maybe_hide_main(app: &AppHandle) -> bool {
    let state = app.state::<crate::AppCtx>();
    if !state.settings.read().unwrap().shot_hide {
        return false;
    }
    let Some(w) = app.get_webview_window("main") else {
        return false;
    };
    if !w.is_visible().unwrap_or(true) {
        return false;
    }

    let _ = w.hide();
    // hide() 是投递到主线程的异步操作,先等 is_visible 真正翻转,
    // 否则捕获到的冻结帧里还留着窗口画面。
    let mut hidden = false;
    for _ in 0..30 {
        if !w.is_visible().unwrap_or(false) {
            hidden = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if hidden {
        // is_visible 翻转只是「窗口已 orderOut」,捕获后端还未必看得到:
        // macOS 侧 xcap 走 ScreenCaptureKit,帧管线自带滞后,立刻捕获会拿到
        // 隐藏前的那一帧(冻结图里带出本应用窗口);Windows 侧是 DWM 的 1-2 帧滞后。
        #[cfg(target_os = "macos")]
        std::thread::sleep(std::time::Duration::from_millis(200));
        wait_compositor();
    }

    // 只要发出过 hide 就按「已隐藏」记账:finish/cancel 会 show() 回来,对本来就
    // 可见的窗口是无害的;反过来记 false 而实际已隐藏,窗口就永久消失了。
    true
}

/// 等 DWM 合成器追上窗口可见性变化(仅 Windows;其他平台空实现)。
#[cfg(windows)]
fn wait_compositor() {
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmFlush() -> i32;
    }
    for _ in 0..3 {
        unsafe { DwmFlush() };
    }
    std::thread::sleep(std::time::Duration::from_millis(60));
}

#[cfg(not(windows))]
fn wait_compositor() {}
