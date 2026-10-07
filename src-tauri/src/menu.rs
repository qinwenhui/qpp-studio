//! macOS 原生菜单栏。Windows/Linux 不建菜单(保持自绘 UI 不变)。
//!
//! 自定义项(设置/打开/粘贴/截图)通过 `app://menu` 事件转发前端,
//! 与应用内快捷键走同一批处理函数;预定义项(退出/最小化/编辑组)由
//! 系统/WKWebView 响应链自行处理,不需要事件。

use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
use tauri::Emitter;

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    // 应用菜单(第一个子菜单在 mac 上自动成为 App 菜单)
    let app_menu = SubmenuBuilder::new(app, "QPP Studio")
        .item(&PredefinedMenuItem::about(app, Some("关于 QPP Studio"), None)?)
        .separator()
        .item(
            &MenuItemBuilder::with_id("settings", "设置…")
                .accelerator("Cmd+,")
                .build(app)?,
        )
        .separator()
        .item(&PredefinedMenuItem::services(app, Some("服务"))?)
        .separator()
        .item(&PredefinedMenuItem::hide(app, Some("隐藏 QPP Studio"))?)
        .item(&PredefinedMenuItem::hide_others(app, Some("隐藏其他"))?)
        .item(&PredefinedMenuItem::show_all(app, Some("全部显示"))?)
        .separator()
        .item(&PredefinedMenuItem::quit(app, Some("退出 QPP Studio"))?)
        .build()?;

    // 文件:与前端快捷键同款动作
    let file_menu = SubmenuBuilder::new(app, "文件")
        .item(
            &MenuItemBuilder::with_id("open", "打开图片…")
                .accelerator("Cmd+O")
                .build(app)?,
        )
        .item(
            &MenuItemBuilder::with_id("paste", "粘贴图片")
                .accelerator("Cmd+V")
                .build(app)?,
        )
        .separator()
        .item(
            &MenuItemBuilder::with_id("shot", "截图识别")
                .accelerator("Cmd+Shift+O")
                .build(app)?,
        )
        .build()?;

    // 编辑:预定义项,焦点在 WKWebView 时系统自动响应(撤销/剪切/复制/粘贴/全选)
    let edit_menu = SubmenuBuilder::new(app, "编辑")
        .item(&PredefinedMenuItem::undo(app, Some("撤销"))?)
        .item(&PredefinedMenuItem::redo(app, Some("重做"))?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, Some("剪切"))?)
        .item(&PredefinedMenuItem::copy(app, Some("复制"))?)
        .item(&PredefinedMenuItem::paste(app, Some("粘贴"))?)
        .item(&PredefinedMenuItem::select_all(app, Some("全选"))?)
        .build()?;

    // 窗口:最小化/缩放(performZoom)/关闭
    let window_menu = SubmenuBuilder::new(app, "窗口")
        .item(&PredefinedMenuItem::minimize(app, Some("最小化"))?)
        .item(&PredefinedMenuItem::maximize(app, Some("缩放"))?)
        .separator()
        .item(&PredefinedMenuItem::close_window(app, Some("关闭窗口"))?)
        .build()?;

    let menu = MenuBuilder::new(app)
        .items(&[&app_menu, &file_menu, &edit_menu, &window_menu])
        .build()?;

    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        let action = match event.id().as_ref() {
            "open" => "open",
            "paste" => "paste",
            "shot" => "shot",
            "settings" => "settings",
            _ => return,
        };
        let _ = app.emit("app://menu", action);
    });
    Ok(())
}
