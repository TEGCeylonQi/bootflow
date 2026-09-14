//! 无效启动项检测。
//!
//! 这一层的价值常被低估。机器用久了，注册表里会积累大量"尸体"——
//! 程序早就卸载了，Run 键还留着。它们**不会拖慢开机**（找不到文件直接跳过），
//! 但会**污染用户的判断**：看到 16 个启动项以为全是负担，实际一半是空的。
//!
//! 把它们标出来，用户第一次就能看清"真正在跑的有几个"。
//!
//! ⚠️ 首版只做**检测与呈现**，绝不执行任何清理。清理是写操作，归 v1.5。

use std::path::Path;

use crate::model::{SourceKind, ValidityStatus};

/// 允许的"可执行目标"扩展名。
///
/// 不必求全——目标是不让 `.dll`、`.txt`、`.log` 这类明显不对的东西
/// 被当成正常启动项。真的指向类型之外的文件，标出来让用户看一眼是好事。
const EXEC_EXTS: &[&str] = &[
    "exe", "com", "bat", "cmd", "ps1", "vbs", "js", "wsf", "lnk", "msc", "url", "cpl",
];

#[derive(Debug, Clone)]
pub struct ValidityCheck {
    pub status: ValidityStatus,
    pub detail: Option<String>,
}

impl ValidityCheck {
    fn new(status: ValidityStatus, detail: impl Into<String>) -> Self {
        Self {
            status,
            detail: Some(detail.into()),
        }
    }

    fn ok() -> Self {
        Self {
            status: ValidityStatus::Ok,
            detail: None,
        }
    }
}

/// 是否为 UNC 网络路径（`\\server\share\...`）
fn is_unc(path: &str) -> bool {
    path.starts_with("\\\\") || path.starts_with("//")
}

/// 路径的盘符（`C` 形式）。非盘符路径（UNC、无前缀的相对路径）返回 `None`。
fn drive_letter(path: &str) -> Option<char> {
    let mut chars = path.chars();
    match (chars.next(), chars.next()) {
        (Some(d), Some(':')) if d.is_ascii_alphabetic() => Some(d.to_ascii_uppercase()),
        _ => None,
    }
}

/// 这个路径所在的**盘符本身不存在**时返回它。
///
/// 为什么要单独判这一步：`Path::exists()` 对"D 盘上的文件"和"D 盘不在系统里"
/// 返回同一个 `false`。而笔记本用户拔掉移动硬盘、拆下扩展坞之后，
/// 一堆指向外接盘的启动项就会集体变成"程序已被卸载"。
/// 那是**误判**——硬盘插回来它们就正常了，用户却可能照着建议把它们清理掉。
fn absent_drive(path: &str) -> Option<String> {
    let letter = drive_letter(path)?;
    let root = format!("{letter}:\\");

    if Path::new(&root).exists() {
        None
    } else {
        Some(root)
    }
}

/// `exists()` 返回假，原因是"看不到"而不是"不存在"。
///
/// 典型位置：`Program Files\WindowsApps`（Store 应用目录）对普通用户不可读。
/// 看不到不等于没有——判成"已失效"会诱导用户清理一个其实正常的项。
fn is_access_denied(path: &str) -> bool {
    const ERROR_ACCESS_DENIED: i32 = 5;

    std::fs::metadata(path)
        .err()
        .and_then(|e| e.raw_os_error())
        == Some(ERROR_ACCESS_DENIED)
}

/// 检查一个启动项的目标是否有效。
///
/// 之所以对 UNC 单独处理：网络路径 `exists()` 为 false 的原因通常是
/// "对方机器没开"或"VPN 没连"，而不是"程序没了"。
/// 把它标成 `MissingTarget` 会诱导用户去清理一个其实正常的东西。
pub fn check(path: &str, source: SourceKind) -> ValidityCheck {
    // 系统注入项不以独立文件形式存在，没有"目标是否存在"这个概念
    if source == SourceKind::SystemHook {
        return ValidityCheck::new(
            ValidityStatus::Unknown,
            "系统注入项不单独启动，没有独立的目标文件",
        );
    }

    let p = path.trim().trim_matches('"').trim();
    if p.is_empty() {
        return ValidityCheck::new(
            ValidityStatus::Unknown,
            "未能从命令行中解析出可执行文件路径",
        );
    }

    let pb = Path::new(p);

    if !pb.exists() {
        return if is_unc(p) {
            ValidityCheck::new(
                ValidityStatus::Unreachable,
                format!("它指向的是一个网络位置，当前访问不到：{p}"),
            )
        } else if let Some(drive) = absent_drive(p) {
            ValidityCheck::new(
                ValidityStatus::Unreachable,
                format!("它所在的磁盘（{drive}）现在不在电脑上：{p}"),
            )
        } else if is_access_denied(p) {
            ValidityCheck::new(
                ValidityStatus::Unknown,
                format!("没有权限确认它是否还在（该位置需要更高权限才能读取）：{p}"),
            )
        } else {
            ValidityCheck::new(
                ValidityStatus::MissingTarget,
                format!("目标文件不存在，程序可能已被卸载：{p}"),
            )
        };
    }

    if pb.is_dir() {
        return ValidityCheck::new(
            ValidityStatus::NotExecutable,
            format!("它指向的是一个文件夹，不是可运行的程序：{p}"),
        );
    }

    let ext = pb
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext.is_empty() || !EXEC_EXTS.contains(&ext.as_str()) {
        let shown = if ext.is_empty() {
            "没有扩展名".to_string()
        } else {
            format!(".{ext}")
        };
        return ValidityCheck::new(
            ValidityStatus::NotExecutable,
            format!("目标文件类型（{shown}）不是可执行程序：{p}"),
        );
    }

    ValidityCheck::ok()
}

/// 检查命令行引号是否配对。
///
/// 这是个真实且常见的故障源：路径带空格但忘了加引号，
/// Windows 会按空格切开，去执行 `C:\Program`——找不到就静默失败。
/// 用户只会觉得"这个程序开机不启动"，很难想到是引号问题。
pub fn check_command_quotes(command: &str) -> Option<String> {
    if command.matches('"').count() % 2 != 0 {
        Some("命令行里的引号不配对。路径含空格时可能被截断，导致这个项实际启动不起来".to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_hook_has_no_target_concept() {
        let c = check("", SourceKind::SystemHook);
        assert_eq!(c.status, ValidityStatus::Unknown);
    }

    #[test]
    fn missing_local_target_is_flagged() {
        let c = check("D:\\NotExist\\__bootflow_test__.exe", SourceKind::RunUser);
        assert_eq!(c.status, ValidityStatus::MissingTarget);
        assert!(c.detail.unwrap().contains("不存在"));
    }

    #[test]
    fn unreachable_network_path_is_not_treated_as_uninstalled() {
        let c = check("\\\\192.0.2.1\\share\\app.exe", SourceKind::RunUser);
        assert_eq!(
            c.status,
            ValidityStatus::Unreachable,
            "网络路径不可达不等于程序被卸载"
        );
    }

    /// 找一个当前不存在的盘符。找不到就跳过——不断言"一定有闲置盘符"。
    fn unused_drive() -> Option<char> {
        ('D'..='Z').rev().find(|c| !Path::new(&format!("{c}:\\")).exists())
    }

    #[test]
    fn missing_drive_is_unreachable_not_uninstalled() {
        // 笔记本拔掉移动硬盘后，指向外接盘的启动项会集体"目标不存在"。
        // 那是误判：硬盘插回来它就正常了。判成 MissingTarget 会诱导用户
        // 照着建议把它清理掉，而用户多半会照做。
        let Some(letter) = unused_drive() else {
            eprintln!("本机没有闲置盘符，跳过");
            return;
        };

        let c = check(&format!("{letter}:\\__bootflow_missing__\\a.exe"), SourceKind::RunUser);
        assert_eq!(
            c.status,
            ValidityStatus::Unreachable,
            "盘符不在系统里 ≠ 程序被卸载"
        );
        assert!(c.detail.unwrap().contains("磁盘"));
    }

    #[test]
    fn existing_drive_with_missing_file_is_still_dead() {
        // 反面：盘符在、文件不在 —— 这才是真的卸载残留。
        // 上一条规则不能矫枉过正，否则"失效检测"就废了。
        let root = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        let c = check(
            &format!("{root}\\__bootflow_missing__\\a.exe"),
            SourceKind::RunUser,
        );
        assert_eq!(
            c.status,
            ValidityStatus::MissingTarget,
            "盘符存在而文件不存在，就是标准的卸载残留"
        );
    }

    #[test]
    fn drive_letter_parsing_is_conservative() {
        assert_eq!(drive_letter("C:\\a\\b.exe"), Some('C'));
        assert_eq!(drive_letter("d:/a/b.exe"), Some('D'), "小写盘符要归一");
        // UNC 和相对路径没有盘符，不能硬套
        assert_eq!(drive_letter("\\\\server\\share\\a.exe"), None);
        assert_eq!(drive_letter("app.exe"), None);
        assert_eq!(drive_letter(""), None);
        // 只有冒号没有盘符的，不能当成盘符
        assert_eq!(drive_letter(":x"), None);
    }

    #[test]
    fn unpaired_quotes_are_detected() {
        assert!(check_command_quotes("\"C:\\Program Files\\A B\\a.exe --x").is_some());
        assert!(check_command_quotes("\"C:\\Program Files\\A B\\a.exe\" --x").is_none());
    }
}
