//! 窗口特效：macOS 毛玻璃主题在 Windows 上的实现（Mica → Acrylic 按序回退）。
//! 非 glass 主题显式清除效果。窗口 transparent 已在 tauri.conf.json 开启。

use tauri::{AppHandle, Manager};
use tauri::window::{Effect, EffectState, EffectsBuilder};

pub fn apply_theme_effect(app: &AppHandle, theme: &str) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
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
    // 特效失败不致命（老系统/驱动）：静默降级为普通透明窗口
    let _ = win.set_effects(effects);
}
