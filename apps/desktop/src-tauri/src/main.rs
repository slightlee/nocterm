// Windows Release 使用 GUI 子系统，避免启动桌面应用时附带控制台窗口；Debug 保留诊断能力。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    nocterm_desktop_lib::run();
}
