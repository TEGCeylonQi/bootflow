//! BootFlow 后端入口。
//!
//! 分层（从下往上）：
//! ```text
//!   sys / elevate / util/*   系统能力（版本、权限、提权、签名、图标、.lnk）
//!   scanners/*               各来源扫描器，只负责"读出来"
//!   pipeline                 后处理（身份归一 → 有效性 → 去重 → 建议）
//!   valid / dedupe / advise  纯逻辑算法，可脱离 Windows 单测
//!   model                    前后端共用契约
//!   commands                 暴露给前端的唯一出口
//! ```

mod advise;
mod commands;
mod dedupe;
mod diag;
mod elevate;
mod error;
mod model;
mod pipeline;
mod scanners;
mod sys;
mod util;
mod valid;

pub use error::AppError;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::scan_all,
            commands::scan_source,
            commands::get_boot_timeline,
            commands::get_os_info,
            commands::check_elevation,
            commands::request_elevation,
            commands::get_icons,
        ])
        .run(tauri::generate_context!())
        .expect("BootFlow 启动失败");
}
