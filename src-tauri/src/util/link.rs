//! `.lnk` 快捷方式解析。
//!
//! **必须走 Shell 的 `IShellLinkW`，不能自己读文件字节。** 理由：
//! - `.lnk` 是二进制格式，目标路径可能以「短路径 8.3 格式」「环境变量形式」
//!   「相对路径 + LinkInfo 结构」三种方式之一存放，自己解析等于重写一遍 Shell。
//! - 路径里可能有非 BMP 字符、可能有 UNC 前缀，手写解析很容易在这些边界上出错。
//! - Shell 会顺带把参数、工作目录、描述一起给你，这些都是界面要展示的信息。
//!
//! 这个模块被两个场景复用：启动文件夹里的 `.lnk`，以及 Run 键里
//! 写着 `.lnk` 路径的情况（确实存在，安装程序有时会这么干）。

use crate::util::cmdline;
use crate::util::com::ComGuard;

/// 一个快捷方式解析出来的内容。
#[derive(Debug, Clone, Default)]
pub struct Shortcut {
    /// 目标程序的完整路径（已展开环境变量）
    pub target: String,
    /// 传给目标的参数
    pub args: Vec<String>,
    pub working_dir: Option<String>,
    /// 快捷方式的描述（鼠标悬停时看到的文字）
    pub description: Option<String>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 解析 `.lnk`。失败返回 `None`——调用方会把该文件当"无法解析的项"处理，
/// 而不是让整个目录扫描失败。
pub fn resolve(lnk_path: &str) -> Option<Shortcut> {
    resolve_impl(lnk_path)
}

#[cfg(windows)]
fn resolve_impl(lnk_path: &str) -> Option<Shortcut> {
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink, SLGP_RAWPATH};

    let _com = ComGuard::new();

    let path_w = wide(lnk_path);

    unsafe {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;

        let persist: IPersistFile = link.cast().ok()?;
        persist.Load(PCWSTR(path_w.as_ptr()), STGM_READ).ok()?;

        // 32768 是 Windows 路径的长路径上限，一次给足省得处理截断。
        //
        // 第二个参数是 `WIN32_FIND_DATAW` 的输出指针。它只在 Windows 95 时代
        // 有意义（用来拿目标的文件属性），现在传 null 是标准做法。
        // 第三个参数 `SLGP_RAWPATH` 让 Shell 返回**未展开环境变量**的原始路径——
        // 自己展开比让 Shell 展开更可控（Shell 会顺手把不存在的变量删掉）。
        let mut target_buf = vec![0u16; 32768];
        // `SLGP_FLAGS` 的底层是 i32，而 `GetPath` 要的是 u32——
        // 这是 windows crate 里少见的类型不一致，转一下即可
        link.GetPath(&mut target_buf, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
            .ok()?;

        let target_raw = String::from_utf16_lossy(&target_buf);
        let target_raw = target_raw.trim_end_matches('\0').trim();

        // 目标为空说明这个快捷方式指向的不是文件系统对象
        // （比如「控制面板」「此电脑」这类 shell 命名空间项）
        if target_raw.is_empty() {
            return None;
        }

        // SLGP_RAWPATH 拿到的可能是 `%ProgramFiles%\...` 形式，展开它
        let target = cmdline::expand_env(target_raw);

        let mut args_buf = vec![0u16; 8192];
        let args = match link.GetArguments(&mut args_buf) {
            Ok(()) => {
                let raw = String::from_utf16_lossy(&args_buf);
                cmdline::parse_args_only(raw.trim_end_matches('\0'))
            }
            Err(_) => Vec::new(),
        };

        let working_dir = read_string(|buf| link.GetWorkingDirectory(buf));

        let description = read_string(|buf| link.GetDescription(buf));

        Some(Shortcut {
            target,
            args,
            working_dir,
            description,
        })
    }
}

/// 把 Shell 那套「传缓冲、返回宽字符串」的取字符串接口统一包一层。
///
/// 三个接口（GetWorkingDirectory / GetDescription / GetIconLocation）
/// 签名完全一样，逐个写一遍纯属重复。
#[cfg(windows)]
unsafe fn read_string<F>(f: F) -> Option<String>
where
    F: FnOnce(&mut [u16]) -> windows::core::Result<()>,
{
    let mut buf = vec![0u16; 4096];
    f(&mut buf).ok()?;

    let s = String::from_utf16_lossy(&buf)
        .trim_end_matches('\0')
        .trim()
        .to_string();

    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(not(windows))]
fn resolve_impl(_lnk_path: &str) -> Option<Shortcut> {
    None
}

/// 这个路径是不是快捷方式文件。
pub fn is_shortcut(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_lnk_extension_case_insensitively() {
        assert!(is_shortcut(r"C:\a\b.LNK"));
        assert!(is_shortcut(r"C:\a\b.lnk"));
        assert!(!is_shortcut(r"C:\a\b.exe"));
    }

    #[test]
    fn missing_file_returns_none_without_panic() {
        assert!(resolve(r"D:\__bootflow_missing__\nope.lnk").is_none());
    }
}
