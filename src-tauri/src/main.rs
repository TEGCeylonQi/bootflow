// Windows 发布版不弹控制台黑窗。调试版保留，方便看 log。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// 静默参数：开机自启时 BootFlow 用这个参数唤醒自己，只记一条开机耗时然后退出。
/// 这样不需要第二个小 exe，也完全不打搅用户（不弹窗、不进主界面）。
const MARK_BOOT_ARG: &str = "--mark-boot";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == MARK_BOOT_ARG) {
        // 自启记账：失败不影响开机、也不弹窗。静默退出。
        let code = if bootflow_lib::record_boot_marker().is_ok() {
            0
        } else {
            1
        };
        std::process::exit(code);
    }
    bootflow_lib::run()
}