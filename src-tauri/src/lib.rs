//! BootFlow 后端入口。
//!
//! 分层（从下往上）：
//! ```text
//!   sys / elevate / util/*   系统能力（版本、权限、提权、签名、图标、.lnk、http）
//!   scanners/*               各来源扫描器，只负责"读出来"
//!   pipeline                 后处理（身份归一 → 有效性 → 去重 → 建议）
//!   valid / dedupe / advise  纯逻辑算法，可脱离 Windows 单测
//!   model                    前后端共用契约
//!   update                   版本比对与新版本检测（唯一会主动联网的模块）
//!   commands                 暴露给前端的唯一出口
//! ```
//!
//! 关于「联网」这一条：v0.1.0 之前整个程序不发任何网络请求。
//! 从 v0.1.1 起，`update` 会在启动后读一次 GitHub 的发布信息——
//! 除此之外没有第二个联网点，也不会把任何本机信息发出去。

mod advise;
mod commands;
mod dedupe;
mod diag;
mod elevate;
mod error;
mod model;
mod pipeline;
mod scanners;
mod snapshot;
mod sys;
mod update;
mod util;
mod valid;
mod writers;

pub use error::AppError;

/// 开机自启的静默记账入口：追加一条开机耗时记录。失败只记日志，不弹窗、不阻塞。
///
/// 供 `main.rs` 的 `--mark-boot` 分支调用。也常作为测试入口。
pub fn record_boot_marker() -> Result<(), String> {
    crate::diag::boot_marker::record_once()
}

/// 首次启动时注册「开机自记账」自启条目（写 `HKCU\...\Run`,带 `--mark-boot`）。
/// 幂等：已存在则跳过。这是**默认开启**的核心功能，不提供开关——见 boot_marker 模块头。
///
/// ⚠️ 只在主程序正常启动（非 `--mark-boot` 静默分支）时调用。
pub fn ensure_boot_marker_autostart() -> Result<(), String> {
    crate::diag::boot_marker::ensure_autostart()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 「每次开机自记账」：正常启动 GUI 时注册自启动（幂等）。
    // 静默分支（--mark-boot）不会走到这里，避免每次开机都注册一遍。
    if let Err(e) = ensure_boot_marker_autostart() {
        log::warn!("开机自记账注册失败：{e}");
    }

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
            commands::get_boot_records,
            commands::get_os_info,
            commands::check_elevation,
            commands::request_elevation,
            commands::get_icons,
            commands::check_update,
            commands::open_release_page,
            commands::open_boot_log,
            commands::probe_boot_record,
            commands::enable_boot_record,
            commands::diagnose_boot_performance,
            commands::install_update,
            commands::clean_install_cache,
            // v0.2.0 可写可控
            commands::dry_run_edits,
            commands::apply_edits,
            commands::list_snapshots,
            commands::rollback_to,
            commands::export_snapshot,
        ])
        .run(tauri::generate_context!())
        .expect("BootFlow 启动失败");
}
