//! 全局截图快捷键：注册 / 改键 / 冲突回报。

use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// 注册快捷键（重复注册同一组合会覆盖处理器，无副作用）。
pub fn register(app: &AppHandle, accel: &str) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(accel, |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                crate::screenshot::begin(app.clone());
            }
        })
        .map_err(|e| format!("快捷键 “{accel}” 注册失败：可能已被其他应用占用（{e}）"))
}

pub fn unregister(app: &AppHandle, accel: &str) {
    use tauri_plugin_global_shortcut::Shortcut;
    if let Ok(shortcut) = accel.parse::<Shortcut>() {
        let _ = app.global_shortcut().unregister(shortcut);
    }
}

/// 改键：先试注册新的，成功后注销旧的，返回最终生效的组合。
pub fn set(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    if old == new {
        return Ok(());
    }
    register(app, new)?;
    unregister(app, old);
    Ok(())
}
