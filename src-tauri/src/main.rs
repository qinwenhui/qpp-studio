// 防止 release 构建在 Windows 上弹出控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 自重生 worker 进程:大批量分治用(见 worker.rs)
    if std::env::args().nth(1).as_deref() == Some("--worker") {
        qpp_studio_lib::worker::run_worker();
        return;
    }
    qpp_studio_lib::run()
}
