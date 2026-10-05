//! 窗口特效:系统材质按主题映射(主题切换时重应用)。
//!
//! - Windows:通透玻璃 → Mica → Acrylic 按序回退;其余主题显式清除。
//!   特效失败不致命(老系统/驱动),静默降级为普通透明窗口。
//! - macOS:三套玻璃系主题给 NSVisualEffectView 材质(tauri set_effects
//!   底层即 window-vibrancy);粉彩/纸墨不启用,靠不透明 CSS 背景遮住
//!   (mac 端 set_effects(None) 无法移除已挂的材质视图,只能盖住)。
//! 窗口 transparent 已开启(mac 走 tauri.conf.json 的 macOSPrivateApi)。

use tauri::{AppHandle, Manager};
use tauri::window::{Effect, EffectState, EffectsBuilder};

pub fn apply_theme_effect(app: &AppHandle, theme: &str) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };

    if cfg!(target_os = "macos") {
        // 通透玻璃:Titlebar 为主;深色科技:窗口底衬;浅色精致:浅色内容底
        let effects = match theme {
            "macos-glass" => Some(
                EffectsBuilder::new()
                    .effects([Effect::Titlebar, Effect::HudWindow])
                    .state(EffectState::Active)
                    .build(),
            ),
            "dark-tech" => Some(
                EffectsBuilder::new()
                    .effects([Effect::UnderWindowBackground])
                    .state(EffectState::Active)
                    .build(),
            ),
            "light-refined" => Some(
                EffectsBuilder::new()
                    .effects([Effect::ContentBackground])
                    .state(EffectState::Active)
                    .build(),
            ),
            _ => None, // 可爱粉彩 / 古典纸墨:纯色背景
        };
        let _ = win.set_effects(effects);
    } else {
        let effects = if theme == "macos-glass" {
            Some(
                EffectsBuilder::new()
                    .effects([Effect::Mica, Effect::Acrylic])
                    .state(EffectState::Active)
                    .build(),
            )
        } else {
            None
        };
        // 特效失败不致命(老系统/驱动):静默降级为普通透明窗口
        let _ = win.set_effects(effects);
    }
}
