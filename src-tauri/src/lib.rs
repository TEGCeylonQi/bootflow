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
mod paths;
mod pipeline;
mod scanners;
mod settings;
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

/// 按设置对齐自记账的自启条目。
///
/// **开启时**注册（幂等），**关闭时**主动清掉两个入口。后者不是可有可无的：
/// 旧版本曾无条件注册，用户升级上来时那条自启还在系统里，光看设置是清不掉的。
///
/// ⚠️ 只在主程序正常启动（非 `--mark-boot` 静默分支）时调用。
pub fn reconcile_boot_marker_autostart() {
    let settings = crate::settings::load();

    if !settings.boot_recording {
        // 关闭状态：清掉可能残留的自启条目，然后什么都不做。
        crate::settings::reconcile(&settings);
        return;
    }

    if let Err(e) = crate::diag::boot_marker::ensure_autostart() {
        log::warn!("开机自记账注册失败：{e}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 「每次开机自记账」：按用户设置对齐自启条目（开启则注册，关闭则清掉）。
    // 静默分支（--mark-boot）不会走到这里，避免每次开机都改一遍系统。
    reconcile_boot_marker_autostart();

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
            commands::get_settings,
            commands::set_boot_recording,
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

/// 单测共享的「进程级环境变量」串行锁。
///
/// `std::env::set_var` 改的是**整个进程**的环境，而 cargo 的测试默认是多线程并行。
/// 只要有两个用例分别改同一个变量（`APPDATA` / `LOCALAPPDATA`），它们就会互相
/// 读到对方设的目录——表现为「这次跑绿、下次跑红」的随机失败，最难查。
///
/// 要点是**锁必须按变量共享，不能按模块各持一把**：
/// 两个模块各持一把却改同一个变量，等于没锁，谁也拦不住谁。
/// `snapshot::store` 与 `snapshot::changelog` 之前正是这么写的，已统一到这里。
///
/// 【为什么放在文件最末尾】它是 `#[cfg(test)]` 的测试基建，而上面几个
/// （`record_boot_marker` / `reconcile_boot_marker_autostart` / `run`）都是
/// 生产代码。把测试模块插在生产代码中间会触发 clippy 的 `items_after_test_module`
/// ——「测试模块之后还有 item」——CI 是 `-D warnings`，会直接红。
/// 组织上的规矩：**测试基建一律排在所有生产 item 之后**。
#[cfg(test)]
pub(crate) mod testenv {
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 所有会改动读写型进程环境变量的用例都必须持有它。
    ///
    /// 上一次持有者 panic 会让锁「中毒」；这里取回内部值继续用，
    /// 免得一次偶发失败把后面所有用例连带变成连环失败。
    pub(crate) fn lock() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
}
