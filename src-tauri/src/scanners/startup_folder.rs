//! 启动文件夹扫描器。
//!
//! Windows 有两个"启动"文件夹，放进去的东西会在用户登录后由 Explorer 启动：
//!
//! | 位置 | 环境变量 | 影响范围 |
//! |---|---|---|
//! | `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup` | 用户级 | 只对当前用户 |
//! | `%ProgramData%\Microsoft\Windows\Start Menu\Programs\Startup` | 全局 | 所有用户 |
//!
//! 这个来源有几个**只在这里才会出现**的特征，正是它值得单独一个扫描器的原因：
//!
//! - **里面的项通常是 `.lnk`**，不是裸 exe。目标路径藏在快捷方式里，
//!   不去解析 `IShellLinkW` 就只看到一个文件名，连它要启动什么都说不出来。
//! - **`IShellLinkW` 会丢信息**：不带 `SLGP_RAWPATH` 时 Shell 可能返回
//!   一个"看起来对但其实是别的路径"的结果（8.3 短名、重定向后的位置）。
//! - **它的启用状态记在 `StartupApproved\StartupFolder`** 而不是这个目录本身——
//!   用户从任务管理器里"禁用"某个项，文件仍然躺在那里。
//!
//! ⚠️ 只读。不对启动文件夹做任何增删改。

use std::path::{Path, PathBuf};

use crate::model::{BootPhase, Scope, SourceKind, StartupItem};
use crate::scanners::builder::{approved_enabled, displayable_name, is_ignored_file_name, ItemBuilder};
use crate::util::link;

/// 用户级启动文件夹。
///
/// 用环境变量而不是拼 `C:\Users\<名>\...`：`APPDATA` 可能因企业策略被重定向，
/// 拼路径会指向一个不存在的位置，而且这个来源会**静默为空**——
/// 用户看到"启动文件夹里没有东西"却是错的。
pub fn user_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| {
        PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\Startup")
    })
}

/// 全局启动文件夹。
pub fn common_dir() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|p| {
        PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs\Startup")
    })
}

/// 扫描两个启动文件夹。
///
/// 返回 `Err` 只在**两个目录都读不到**时发生——单个目录失败属于部分失败，
/// 应该照常返回另一边的结果，并把原因写进日志。
pub fn collect() -> Result<Vec<StartupItem>, String> {
    let mut items = Vec::new();
    let mut errors = Vec::new();

    let targets: [(SourceKind, Option<PathBuf>); 2] = [
        (SourceKind::StartupFolderUser, user_dir()),
        (SourceKind::StartupFolderMachine, common_dir()),
    ];

    for (source, dir) in targets {
        let Some(dir) = dir else {
            errors.push(format!("{source:?}：读不到对应的环境变量"));
            continue;
        };

        match scan_dir(source, &dir) {
            Ok(mut v) => items.append(&mut v),
            Err(e) => errors.push(format!("{}：{e}", dir.display())),
        }
    }

    if items.is_empty() && !errors.is_empty() {
        return Err(errors.join("；"));
    }

    for e in &errors {
        log::warn!("启动文件夹部分失败：{e}");
    }

    log::info!("启动文件夹：扫出 {} 项", items.len());
    Ok(items)
}

fn scan_dir(source: SourceKind, dir: &Path) -> std::io::Result<Vec<StartupItem>> {
    let mut out = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // 目录不存在是正常情况（比如某些精简系统删掉了全局启动文件夹），
        // 不应该污染错误列表让用户紧张
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // 用 file_type() 而不是 is_file()：后者会跟随符号链接，
        // 而启动文件夹里出现符号链接时我们想按它本身处理
        if !matches!(entry.file_type(), Ok(t) if t.is_file()) {
            continue;
        }

        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };

        if is_ignored_file_name(file_name) {
            continue;
        }

        match build_item(source, dir, &path, file_name) {
            Some(item) => out.push(item),
            None => log::warn!("无法解析启动项：{}", path.display()),
        }
    }

    Ok(out)
}

fn build_item(
    source: SourceKind,
    dir: &Path,
    path: &Path,
    file_name: &str,
) -> Option<StartupItem> {
    let full_path = path.to_string_lossy().to_string();

    // 快捷方式要解析出真正的目标；直接放进去的 exe / bat 就是它自己
    let (target, args, command, shortcut_meta) = if link::is_shortcut(&full_path) {
        let sc = link::resolve(&full_path)?;

        let command = if sc.args.is_empty() {
            format!("\"{}\"", sc.target)
        } else {
            format!("\"{}\" {}", sc.target, sc.args.join(" "))
        };

        // 描述与工作目录进 raw，属性面板的「技术详情」会展示。
        // 工作目录尤其值得留着：它配错时程序会以错误的相对路径启动，
        // 表现是"能起来但功能不对"，这类问题很难查。
        let meta = serde_json::json!({
            "description": sc.description,
            "workingDir": sc.working_dir,
            "rawTarget": sc.target,
        });

        (sc.target, sc.args, command, Some(meta))
    } else {
        (full_path.clone(), Vec::new(), full_path.clone(), None)
    };

    let enabled = approved_enabled(source, file_name);

    let mut raw = serde_json::json!({
        "kind": "startupFolder",
        "fileName": file_name,
        "filePath": full_path,
        "approvedEnabled": enabled,
    });

    if let Some(meta) = shortcut_meta {
        raw["shortcut"] = meta;
    }

    let item = ItemBuilder::new(source, displayable_name(file_name))
        .scope(if source == SourceKind::StartupFolderMachine {
            Scope::Machine
        } else {
            Scope::User
        })
        .location(dir.to_string_lossy().to_string())
        .command(command)
        .target(target)
        .args(args)
        .enabled(enabled)
        // 启动文件夹里的东西都在用户登录后才由 Explorer 拉起来
        .boot_phase(BootPhase::Logon)
        .raw(raw)
        .build();

    Some(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_dirs_resolve_on_windows() {
        // 这两个环境变量在所有 Windows 上都存在
        assert!(user_dir().is_some(), "应能定位用户启动文件夹");
        assert!(common_dir().is_some(), "应能定位全局启动文件夹");
    }

    #[test]
    fn collect_never_panics_and_returns_items() {
        // 真实机器上这个来源可能为空（干净系统），也可能有若干项。
        // 这里只验证"能跑完且不 panic"，不断言数量——那会让测试绑死在具体机器状态上。
        match collect() {
            Ok(items) => {
                for it in items {
                    assert!(!it.name.is_empty(), "名称不应为空");
                    assert!(
                        matches!(
                            it.source,
                            SourceKind::StartupFolderUser | SourceKind::StartupFolderMachine
                        ),
                        "来源只能是启动文件夹两类之一"
                    );
                }
            }
            Err(e) => panic!("启动文件夹扫描不应整体失败：{e}"),
        }
    }

    #[test]
    fn found_shortcuts_have_a_resolved_target() {
        let items = collect().unwrap_or_default();

        for it in items {
            let is_lnk = it
                .raw
                .get("fileName")
                .and_then(|v| v.as_str())
                .is_some_and(|n| n.to_ascii_lowercase().ends_with(".lnk"));

            if is_lnk {
                // 解析失败的 .lnk 会在 build_item 里返回 None 被丢弃，
                // 因此能出现在结果里的快捷方式必须已经拿到目标
                assert!(
                    !it.resolved_path.is_empty(),
                    "「{}」是快捷方式却没有解析出目标",
                    it.name
                );
            }
        }
    }

    /// 诊断用：把这台机器上启动文件夹的实际内容打出来。
    ///
    /// 不断言数量（那会绑死在具体机器上），但它是验证 `.lnk` 解析、
    /// 友好名识别、签名读取三条链路是否真的可用的最快方式：
    /// `cargo test report_startup_folder -- --nocapture`
    #[test]
    fn report_startup_folder_contents() {
        let items = collect().unwrap_or_default();

        println!("\n启动文件夹共 {} 项", items.len());
        for it in &items {
            println!(
                "  {:<28} 名称={:<28} 目标={}",
                format!("{:?}", it.source),
                it.display_name.as_deref().unwrap_or(&it.name),
                it.resolved_path,
            );
            println!(
                "  {:<28} 签名={:<6} 发布者={:<24} 启用={}",
                "",
                it.signer.is_signed,
                it.signer.publisher.as_deref().unwrap_or("-"),
                it.enabled,
            );
        }
    }
}
