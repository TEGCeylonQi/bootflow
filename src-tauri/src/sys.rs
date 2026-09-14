//! 系统信息与权限探测。
//!
//! 这些是"无论扫描结果如何都要能拿到"的信息——顶部状态栏靠它们渲染。
//! 因此这里**不允许失败**：拿不到就退化成保守默认值，而不是让整个应用起不来。

use crate::error::{AppError, Result};
use crate::model::OsInfo;

/// 读取操作系统版本。
///
/// 为什么不直接用 `GetVersionExW`？因为从 Win8.1 起，该 API 对未声明
/// 应用清单的程序会撒谎（一律返回 6.2）。读注册表是更可靠的做法，
/// 也能顺便拿到 `DisplayVersion`（24H2 这种），比版本号更好读。
pub fn os_info() -> Result<OsInfo> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .map_err(|e| AppError::Registry(format!("打开 CurrentVersion 失败：{e}")))?;

    let major: u32 = key.get_value("CurrentMajorVersionNumber").unwrap_or(10);
    let minor: u32 = key.get_value("CurrentMinorVersionNumber").unwrap_or(0);

    // CurrentBuildNumber 是字符串，且有极少数机器缺失，这里全部容错
    let build: u32 = key
        .get_value::<String, _>("CurrentBuildNumber")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let product: String = key
        .get_value("ProductName")
        .unwrap_or_else(|_| "Windows".to_string());

    // DisplayVersion（24H2 / 23H2）是 Win10 2004 之后才有的字段
    let display: String = key.get_value("DisplayVersion").unwrap_or_default();

    let sku = if display.is_empty() {
        product
    } else {
        format!("{product} {display}")
    };

    Ok(OsInfo {
        major,
        minor,
        build,
        sku,
    })
}

/// 当前进程是否以管理员身份运行。
///
/// 用 token elevation 而不是"尝试打开一个需要权限的注册表键"——
/// 后者会因 ACL 配置差异产生误判（某些键普通用户也能读）。
#[cfg(windows)]
pub fn is_elevated() -> Result<bool> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();

        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| AppError::Win32(format!("OpenProcessToken 失败：{e}")))?;

        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned: u32 = 0;

        let res = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );

        // 无论成败都要关闭句柄，否则每次调用泄漏一个内核对象
        let _ = CloseHandle(token);

        res.map_err(|e| AppError::Win32(format!("GetTokenInformation 失败：{e}")))?;

        Ok(elevation.TokenIsElevated != 0)
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_info_is_readable_on_windows() {
        let info = os_info().expect("应能读到系统版本");
        assert!(info.major >= 10, "现代 Windows 主版本号应至少为 10");
        assert!(!info.sku.is_empty());
    }
}
