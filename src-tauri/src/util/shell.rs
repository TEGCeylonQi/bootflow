//! 用系统默认程序打开一个网址。
//!
//! 【为什么不给前端一个通用的"打开任意链接"能力】
//! 一旦前端能随手把任意字符串交给系统 shell，这个程序就变成了一个
//! "只要界面里能塞进一行字符串就能拉起本机任意程序"的跳板。
//! 检查更新只需要打开 GitHub 的发布页，所以这里只放行 https，
//! 且域名必须落在 GitHub 自家（白名单在 `update::ensure_trusted_url`）。

use windows::core::PCWSTR;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::error::{AppError, Result};

/// 用默认浏览器打开 `url`。
///
/// 调用方**必须**先用 `update::ensure_trusted_url` 校验过来源，
/// 这个函数只负责"把字符串交给系统"，不判断该不该打开。
pub fn open_url(url: &str) -> Result<()> {
    if !url.starts_with("https://") {
        return Err(AppError::Other(format!("只允许打开 https 地址：{url}")));
    }

    let verb = wide("open");
    let file = wide(url);

    let ret = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW 的返回值 > 32 才算成功。
    // 这个约定是从 16 位 Windows 继承下来的：小数值本身是错误码，
    // 0 表示内存不足，2 表示找不到文件，5 表示被拒绝访问。
    let code = ret.0 as isize;
    if code <= 32 {
        return Err(AppError::Other(format!(
            "没能唤起浏览器（ShellExecute 返回 {code}）"
        )));
    }

    Ok(())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
