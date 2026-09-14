//! 提权 —— 以管理员身份重新拉起自身。
//!
//! 首版其实**用不到**这个能力：只读扫描不需要管理员权限。
//! 但接口先建好，因为 v1.5 的写操作（改注册表 / 建计划任务）必然需要它，
//! 而提权链路本身有不少坑（参数透传、UAC 取消处理、单实例），
//! 值得早点把地基打好。
//!
//! 其中参数透传最容易漏：以「管理员身份重新启动自身」的方式拉起进程时，
//! 如果没把原始命令行带过去，提权后参数就全丢了——程序会用默认参数再跑一遍。
//! 这里用 `std::env::args()` 完整透传。
//!
//! 实现选择：用 `ShellExecuteW` 而不是 `ShellExecuteExW`。
//! 后者要构造 `SHELLEXECUTEINFOW` 结构体，而在 `windows` 0.58 中该符号的
//! 可用性受额外 feature 约束；前者只需传 verb 字符串即可实现 runas 提权，
//! 返回值语义简单（≤32 即为失败），够用且依赖面更小。

use crate::error::{AppError, Result};

#[cfg(windows)]
pub fn relaunch_elevated() -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    /// ShellExecute 系列约定的失败判据：返回值 <= 32 表示失败，
    /// 且具体数字有含义（5=拒绝访问、2=文件未找到……）。
    const SE_ERR_ACCESSDENIED: isize = 5;

    let exe =
        std::env::current_exe().map_err(|e| AppError::Other(format!("无法定位自身程序路径：{e}")))?;

    // 完整透传原有参数 —— 这一步是历史脚本踩过的坑
    let params: Vec<String> = std::env::args().skip(1).collect();
    let params_joined = params.join(" ");

    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let file: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let param_w: Vec<u16> = params_joined.encode_utf16().chain(Some(0)).collect();

    let params_ptr = if params_joined.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(param_w.as_ptr())
    };

    let ret = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            params_ptr,
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };

    let code = ret.0 as isize;
    if code <= 32 {
        // 用户的 UAC 弹窗上点"否"是最常见的情况（返回 5），
        // 这不是 bug，措辞要让用户明白发生了什么，而不是看到一个吓人的系统错误码
        let hint = if code == SE_ERR_ACCESSDENIED {
            "如果你在权限提示中选择了「否」，这是正常结果，不需要处理。"
        } else {
            "系统拒绝了这次操作。"
        };
        return Err(AppError::Other(format!(
            "未能以管理员身份启动。{hint}（返回码 {code}）"
        )));
    }

    Ok(())
}

#[cfg(not(windows))]
pub fn relaunch_elevated() -> Result<()> {
    Err(AppError::Other("提权功能仅在 Windows 上可用".to_string()))
}
