// Windows 发布版不弹控制台黑窗。调试版保留，方便看 log。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    bootflow_lib::run()
}
