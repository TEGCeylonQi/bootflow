//! 独立回滚脚本导出（T47）—— 从快照生成**不依赖 BootFlow** 的 `.ps1` / `.reg`。
//!
//! 这是「可逆性 > 一切」的终极保障：**软件卸载了也能退回来**。
//!
//! 产物策略：
//! - `.ps1`：全来源通用（注册表 / 计划任务 / 服务三路全覆盖），
//!   只调用 `reg add` / `schtasks /Change` / `sc config` 这些 Windows 自带命令。
//! - `.reg`：注册表来源专用（用户可双击导入，零门槛）。
//!
//! 诚实边界（与 writers 严格一致）：
//! - v0.2.0 的「停用」只写 `StartupApproved` 缓存，**从不删除** Run 值 / 任务 / 服务。
//!   因此回滚脚本也只负责把 `StartupApproved` 写回原字节、启停任务/服务，
//!   不做任何删除 —— 恢复的是"状态"，不是"实体"。
//!
//! 硬性要求（计划 §3.5.2 T47）：
//! 1. **零外部依赖** —— 脚本只调 Windows 自带命令，BootFlow 卸载后仍可执行。
//! 2. **自我说明** —— 头部注明生成时间、依据快照 id、描述。
//! 3. **非破坏** —— 绝不 delete，只 add/config。

// T47 阶段性标记：模块已实现，待 commands.rs 接线（导出按钮）后移除。
#![allow(dead_code)]

use crate::error::Result;
use crate::snapshot::model::{Snapshot, SnapshotRecord, SnapshotTarget};

/// 导出产物。
#[derive(Debug, Clone)]
pub struct ExportBundle {
    /// PowerShell 全文（所有来源）。
    pub ps1: String,
    /// .reg 全文（仅注册表来源；无注册表记录时为空字符串）。
    pub reg: String,
}

/// 生成独立回滚产物（字符串带回调用方，由调用方写文件，本模块不碰 IO 路径）。
pub fn export(snapshot: &Snapshot) -> Result<ExportBundle> {
    let ps1 = make_ps1(snapshot);
    let reg = make_reg(snapshot);
    Ok(ExportBundle { ps1, reg })
}

/// admin 检查头（PowerShell）。
fn admin_header() -> &'static str {
    r#"
# ------------------------------------------------------------
#  BootFlow 独立回滚脚本
#  本脚本把启动项恢复到一个 BootFlow 快照记录的状态。
#  仅调用 Windows 自带命令（reg / schtasks / sc），
#  可在卸载 BootFlow 之后继续使用。
#
#  用法：右键「以管理员身份运行」这个 .ps1，或在管理员 PowerShell 里：
#    powershell -ExecutionPolicy Bypass -File .\rollback.ps1
# ------------------------------------------------------------

# 管理员检查
if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host "请以管理员身份运行此脚本。" -ForegroundColor Red
    exit 1
}
Write-Host "正在恢复启动项……" -ForegroundColor Cyan
"#
}

/// hex 字符串 → `02,02,00,...`（逗号分隔小写，供 reg /t REG_BINARY /d）。
fn hex_dots(hex: &str) -> String {
    hex.chars()
        .collect::<Vec<_>>()
        .chunks(2)
        .map(|c| c.iter().map(|ch| ch.to_ascii_lowercase()).collect::<String>())
        .collect::<Vec<_>>()
        .join(",")
}

/// 是否属于注册表来源（Run / RunOnce / StartupFolder）。
fn is_reg_source(t: &SnapshotTarget) -> bool {
    matches!(
        t.source.as_str(),
        "RunUser"
            | "RunMachine"
            | "RunOnceUser"
            | "RunOnceMachine"
            | "RunOnceMachine32"
            | "StartupFolderUser"
            | "StartupFolderMachine"
    )
}

/// 从 `location` 得到 `reg add` 用的完整键路径（保留根前缀）。
fn full_key_path(location: &str) -> String {
    if location.starts_with("HKEY_") || location.starts_with("HKLM") || location.starts_with("HKCU") {
        location.to_string()
    } else {
        // 裸相对路径：机器级 → HKLM，用户级 → HKCU（调用方已决定，这里按 location 前缀兜底）
        // 更稳的是调用方传 scope；本函数不猜，直接原样返回（测试用相对路径）
        location.to_string()
    }
}

/// 生成 PowerShell 全文。
fn make_ps1(snapshot: &Snapshot) -> String {
    let mut s = String::new();
    s.push_str(admin_header());
    s.push_str(&format!(
        "# 依据快照：{}  ({})\n",
        snapshot.id, snapshot.created_at
    ));
    s.push_str("# 恢复以下启动项的启停状态：\n\n");

    let mut task_count = 0;
    let mut svc_count = 0;

    for rec in &snapshot.records {
        // —— 注册表来源（StartupApproved 缓存）——
        if is_reg_source(&rec.target) {
            if let Some(hex) = &rec.approved_raw {
                let key = full_key_path(&rec.target.location);
                let vn = rec.target.value_name.as_deref().unwrap_or("");
                s.push_str(&format!("reg add \"{key}\" /v \"{vn}\" /t REG_BINARY /d {} /f\n", hex_dots(hex)));
                continue;
            }
        }

        // —— 计划任务（两级开关中，任务级可恢复，触发器级在主程序内处理）——
        if rec.target.source == "ScheduledTask" {
            if let Some(v) = rec.task_enabled {
                let action = if v { "enable" } else { "disable" };
                s.push_str(&format!(
                    "schtasks /Change /TN \"{}\" /{action}   # 任务级恢复\n",
                    rec.target.location
                ));
                task_count += 1;
            }
            if let Some(v) = rec.trigger_enabled {
                s.push_str(&format!(
                    "# 触发器级状态（{v}）需用任务计划程序手动调整；本版不直接改触发器\n"
                ));
            }
            continue;
        }

        // —— 服务（仅 AUTO/DEMAND 恢复）——
        if rec.target.source == "Service" {
            if let Some(t) = rec.service_start_type {
                let start = match t {
                    2 => "auto",
                    3 => "demand",
                    _ => continue, // 4=disabled 不在 v0.2.0 可写范围，跳过
                };
                s.push_str(&format!(
                    "sc.exe config \"{}\" start= {start}\n",
                    rec.target.location
                ));
                svc_count += 1;
            }
        }
    }

    if task_count + svc_count == 0 {
        s.push_str("# （本快照无计划任务 / 服务改动）\n");
    }

    s.push_str("\nWrite-Host \"恢复完成。\" -ForegroundColor Green\n");
    s
}

/// 生成 .reg（仅注册表来源：StartupApproved 二进制写回）。
fn make_reg(snapshot: &Snapshot) -> String {
    let mut body = String::new();
    for rec in &snapshot.records {
        if !is_reg_source(&rec.target) {
            continue;
        }
        if rec.approved_raw.is_some() {
            body.push_str(&reg_value_for(rec));
            body.push('\n');
        }
    }
    if body.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    s.push_str("Windows Registry Editor Version 5.00\n\n");
    s.push_str(&body);
    s
}

/// 单条 .reg 值（StartupApproved 二进制）。
fn reg_value_for(rec: &SnapshotRecord) -> String {
    let t = &rec.target;
    let Some(approved_raw) = &rec.approved_raw else { return String::new() };
    // location 形如 `HKCU\Software\...\Explorer\StartupApproved\Run`
    // （writers/approved.rs snapshot_record_for 产生）；.reg 需要完整 `[HKEY_...\...]` 键路径。
    let (root, path) = split_root(&t.location, rec.scope.as_str());
    let key = format!("{root}\\{path}");
    format!(
        "[{key}]\n\"{}\"=hex:{}\n",
        t.value_name.as_deref().unwrap_or(""),
        hex_dots(approved_raw)
    )
}

/// 把 `HKCU\` / `HKLM\` 前缀（或裸路径）规范成 .reg 根 + 余下路径。
fn split_root(location: &str, scope: &str) -> (String, String) {
    for (prefix, root) in [
        ("HKCU\\", "HKEY_CURRENT_USER"),
        ("HKLM\\", "HKEY_LOCAL_MACHINE"),
        ("HKEY_CURRENT_USER\\", "HKEY_CURRENT_USER"),
        ("HKEY_LOCAL_MACHINE\\", "HKEY_LOCAL_MACHINE"),
    ] {
        if let Some(rest) = location.strip_prefix(prefix) {
            return (root.to_string(), rest.to_string());
        }
    }
    // 裸相对路径：靠 scope 兜底
    let root = if scope == "machine" {
        "HKEY_LOCAL_MACHINE"
    } else {
        "HKEY_CURRENT_USER"
    };
    (root.to_string(), location.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::model::{SnapshotReason, SNAPSHOT_SCHEMA_VERSION};

    fn rec_app(id: &str, approved: &str) -> SnapshotRecord {
        SnapshotRecord {
            id: id.into(),
            display_name: "应用".into(),
            target: SnapshotTarget {
                source: "RunUser".into(),
                location: r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run".into(),
                value_name: Some("QQ".into()),
            },
            approved_raw: Some(approved.to_string()),
            task_enabled: None,
            trigger_enabled: None,
            service_start_type: None,
            scope: "user".into(),
            risk: "Safe".into(),
        }
    }

    fn snap(records: Vec<SnapshotRecord>) -> Snapshot {
        Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            id: "snap-ex".into(),
            created_at: "2026-09-15T10:00:00Z".into(),
            description: "导出测试".into(),
            reason: SnapshotReason::Scan,
            records,
        }
    }

    #[test]
    fn ps1_contains_admin_check_and_reg_add() {
        let s = snap(vec![rec_app("app", "0202")]);
        let ps1 = export(&s).unwrap().ps1;
        assert!(ps1.contains("以管理员身份运行"));
        assert!(ps1.contains("reg add"));
        assert!(ps1.contains("依据快照"));
    }

    #[test]
    fn hex_dots_formats_bytes() {
        assert_eq!(hex_dots("0202"), "02,02");
        assert_eq!(hex_dots("0202000000000000"), "02,02,00,00,00,00,00,00");
    }

    #[test]
    fn reg_contains_startup_approved() {
        let s = snap(vec![rec_app("app", "0202")]);
        let reg = export(&s).unwrap().reg;
        assert!(reg.contains("Windows Registry Editor Version 5.00"));
        assert!(reg.contains("StartupApproved"));
        assert!(reg.contains("\"QQ\"=hex:02,02"));
    }

    #[test]
    fn task_and_service_go_only_to_ps1() {
        let task_rec = SnapshotRecord {
            id: "task".into(),
            display_name: "任务".into(),
            target: SnapshotTarget {
                source: "ScheduledTask".into(),
                location: "\\MyTask".into(),
                value_name: None,
            },
            approved_raw: None,
            task_enabled: Some(false),
            trigger_enabled: None,
            service_start_type: None,
            scope: "user".into(),
            risk: "Safe".into(),
        };
        let svc_rec = SnapshotRecord {
            id: "svc".into(),
            display_name: "服务".into(),
            target: SnapshotTarget {
                source: "Service".into(),
                location: "MySvc".into(),
                value_name: None,
            },
            approved_raw: None,
            task_enabled: None,
            trigger_enabled: None,
            service_start_type: Some(3),
            scope: "machine".into(),
            risk: "Safe".into(),
        };

        let s = snap(vec![task_rec, svc_rec]);
        let b = export(&s).unwrap();
        assert!(b.ps1.contains("schtasks /Change"));
        assert!(b.ps1.contains("sc.exe config"));
        assert!(b.reg.is_empty(), "任务/服务不应出现在 .reg 中");
    }

    #[test]
    fn empty_snapshot_ok() {
        let s = snap(vec![]);
        let b = export(&s).unwrap();
        assert!(b.reg.is_empty());
        assert!(!b.ps1.is_empty(), "头部仍在");
    }
}