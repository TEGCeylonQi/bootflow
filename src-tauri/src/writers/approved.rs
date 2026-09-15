//! 启动项启停 writer —— v0.2.0 第一梯队写能力。
//!
//! 机制：写 `StartupApproved` 缓存键，**不删实体**（Run 值 / 快捷方式原样保留）。
//! 这正是任务管理器「禁用启动项」的做法——用户随时能在系统设置里改回来，
//! 机制本身就是为了可逆设计的（计划 §3.5.1 第一梯队）。
//!
//! 三条硬规则（均来自前期真机沉淀）：
//! 1. **值名逐字节保留**（T5 硬规则 2）：`HKLM\...\Run` 下存在值名
//!    `" QQPCTray"`（带前导空格），写回时必须用与注册表完全相同的名字，
//!    绝不能 trim——否则会凭空多出一条"已禁用"记录，真实项仍在自启。
//! 2. **改→验→改回**：写完立即读回比对；不一致即报错并还原。
//! 3. **写入前先过护栏**（T45）：Locked 项一个字节都不写。
//!
//! `REG_BINARY` 格式（12 字节）：首字节 `0x02` = 启用、`0x03` = 停用，
//! 其余 11 字节为停用时刻 FILETIME（Windows 内部用于"多久没用过"排序）。

// T41a 阶段性标记：writer 已实现，待 commands.rs 接线后移除。
#![allow(dead_code)]

use crate::error::{AppError, Result};
use crate::model::{SourceKind, StartupItem};
use crate::snapshot::guard;
use crate::snapshot::model::SnapshotRecord;
use crate::snapshot::model::SnapshotTarget;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE};

/// `StartupApproved` 根。
const APPROVED_BASE: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

/// 目标状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desired {
    Enable,
    Disable,
}

/// 子键映射：来源 → (是否机器级, 子键名)。
///
/// `Run` 与 `RunOnce` 共用 `StartupApproved\Run` —— 系统本身就是这么实现的；
/// 32 位视图（Run32）独立成键；服务与计划任务不在此机制内。
pub fn subkey_for(source: SourceKind) -> Option<(bool, &'static str)> {
    use SourceKind::*;
    match source {
        StartupFolderUser => Some((false, "StartupFolder")),
        StartupFolderMachine => Some((true, "StartupFolder")),
        // Run 与 RunOnce 共用 StartupApproved\Run
        RunUser | RunOnceUser => Some((false, "Run")),
        RunMachine | RunOnceMachine => Some((true, "Run")),
        // 32 位视图独立键
        RunMachine32 | RunOnceMachine32 => Some((true, "Run32")),
        Service | ScheduledTask | SystemHook => None,
    }
}

/// 打开对应根键下的 `StartupApproved\<sub>` 子键。
///
/// 不自动创建：若该键不存在，说明用户从没动过任何启动项——
/// "启用"本就无需记录，"停用"因键不存在也无从写起（任务管理器和系统
/// 都会自动创建；我们刻意保持"写前必须存在"以保守）。
fn open_key(machine: bool, sub: &str) -> Result<winreg::RegKey> {
    let root = if machine {
        winreg::RegKey::predef(HKEY_LOCAL_MACHINE)
    } else {
        winreg::RegKey::predef(HKEY_CURRENT_USER)
    };
    root.open_subkey_with_flags(format!(r"{APPROVED_BASE}\{}", sub), KEY_READ | KEY_WRITE)
        .map_err(|e| AppError::Registry(format!("打开 StartupApproved\\{} 失败：{e}", sub)))
}

/// 从注册表读一条记录（`None` = 没有记录 / 读不到）。
fn read_record(machine: bool, sub: &str, name: &str) -> Option<bool> {
    let key = open_key(machine, sub).ok()?;
    let raw = key.get_raw_value(name).ok()?;
    // REG_BINARY 首字节 0x03 → 停用，其余 → 启用
    Some(!matches!(raw.bytes.first(), Some(0x03)))
}

/// 写一条记录。
fn write_record(machine: bool, sub: &str, name: &str, enabled: bool) -> Result<()> {
    let key = open_key(machine, sub)?;
    // 12 字节 REG_BINARY：首字节状态，其余为 FILETIME 数据（这里填充当前时刻，
    // 让 Windows 的"最近启用/停用"排序有意义）。
    let mut bytes = vec![0u8; 12];
    bytes[0] = if enabled { 0x02 } else { 0x03 };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u64)
        .unwrap_or(0);
    // FILETIME 是 100ns 粒度，这里把秒放进低 4 字节（足够排序 + 人类可读）
    let ft = now.wrapping_mul(10_000_000); // now(s) → 100ns  ticks
    let ticks = ft.to_le_bytes();
    bytes[4..8].copy_from_slice(&ticks[..4]);

    key.set_raw_value(
        name,
        &winreg::RegValue {
            vtype: winreg::enums::RegType::REG_BINARY,
            bytes,
        },
    )
    .map_err(|e| AppError::Registry(format!("写入「{}」失败：{e}", name)))
}

/// 读取某个启动项的当前启停状态（`None`= 无记录 = 启用）。
pub fn current_state(item: &StartupItem) -> Option<bool> {
    let (machine, sub) = subkey_for(item.source)?;
    read_record(machine, sub, &item.name)
}

/// 写启停状态，含「改→验→改回」。
///
/// 返回值：写入成功后的实际状态（与 `desired` 一致）。
pub fn set_enabled(item: &StartupItem, desired: Desired) -> Result<bool> {
    // ① 护栏
    let gate = guard::check(item);
    if !gate.is_allowed() {
        return Err(AppError::Other(format!(
            "拒绝修改「{}」：{}",
            item.display_name.as_deref().unwrap_or(&item.name),
            guard::denial_message(&gate)
        )));
    }

    // ② 定位子键
    let (machine, sub) = subkey_for(item.source).ok_or_else(|| {
        AppError::Other(format!(
            "来源 {:?} 不支持 StartupApproved 写法（服务/计划任务请用各自 writer）",
            item.source
        ))
    })?;

    // ③ 乐观锁：写前读当前值（调用方在事务层决定是否因外部改动中止）
    let _before = read_record(machine, sub, &item.name);

    // ④ 写入
    let target = desired == Desired::Enable;
    write_record(machine, sub, &item.name, target)?;

    // ⑤ 改→验
    let after = read_record(machine, sub, &item.name);
    if after != Some(target) {
        // 写后读回不一致：尝试还原原值
        let _ = write_record(machine, sub, &item.name, _before.unwrap_or(true));
        return Err(AppError::Other(format!(
            "写入「{}」后读回校验失败（期望 {}，读到 {:?}），已尝试还原原值。",
            item.name,
            if target { "启用" } else { "停用" },
            after
        )));
    }

    Ok(target)
}

/// 为该项生成快照记录（快照层建基线用）。
pub fn snapshot_record_for(item: &StartupItem) -> SnapshotRecord {
    let (machine, sub) = subkey_for(item.source).unwrap_or((false, ""));
    SnapshotRecord {
        id: item.id.clone(),
        display_name: item
            .display_name
            .clone()
            .unwrap_or_else(|| item.name.clone()),
        target: SnapshotTarget {
            source: format!("{:?}", item.source),
            location: if machine {
                format!(r"HKLM\{APPROVED_BASE}\{}", sub)
            } else {
                format!(r"HKCU\{APPROVED_BASE}\{}", sub)
            },
            // 值名逐字节保留（含前导空格）
            value_name: Some(item.name.clone()),
        },
        approved_raw: current_raw(machine, sub, &item.name),
        task_enabled: None,
        trigger_enabled: None,
        service_start_type: None,
        scope: if item.scope == crate::model::Scope::Machine {
            "machine"
        } else {
            "user"
        }
        .to_string(),
        risk: format!("{:?}", item.risk),
    }
}

/// 读取当前 REG_BINARY 原值（十六进制字符串），无记录 = None。
fn current_raw(machine: bool, sub: &str, name: &str) -> Option<String> {
    let key = open_key(machine, sub).ok()?;
    let raw = key.get_raw_value(name).ok()?;
    Some(hex(&raw.bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BootPhase, ItemKind, ItemTiming, Scope, SignerInfo, ValidityStatus};

    fn item(source: SourceKind, name: &str, scope: Scope, location: &str) -> StartupItem {
        StartupItem {
            id: format!("id-{}", name),
            source,
            identity_key: "k".into(),
            name: name.into(),
            kind: ItemKind::App,
            display_name: Some(name.into()),
            name_from: None,
            summary: None,
            command: r"C:\App\app.exe".into(),
            resolved_path: r"C:\App\app.exe".into(),
            args: vec![],
            location: location.into(),
            scope,
            enabled: true,
            signer: SignerInfo::default(),
            icon_data: None,
            risk: crate::model::RiskLevel::Safe,
            risk_reasons: vec![],
            diagnostics: vec![],
            boot_phase: BootPhase::Shell,
            timing: ItemTiming::default(),
            validity: ValidityStatus::Ok,
            validity_detail: None,
            recommendation: None,
            duplicate_of: None,
            raw: serde_json::json!({}),
            desired: crate::model::DesiredState::default(),
            snapshot_ref: None,
        }
    }

    #[test]
    fn snapshot_record_keeps_value_name_byte_exact() {
        // T5 硬规则 2：值名前导空格绝不能 trim
        let it = item(SourceKind::RunMachine, " QQPCTray", Scope::Machine, "");
        let rec = snapshot_record_for(&it);
        assert_eq!(rec.target.value_name.as_deref(), Some(" QQPCTray"));
        assert!(rec.target.location.contains("StartupApproved"));
    }

    #[test]
    fn set_enabled_denies_locked_item() {
        let mut it = item(SourceKind::RunUser, "App", Scope::User, "");
        it.risk = crate::model::RiskLevel::Locked;
        let err = set_enabled(&it, Desired::Disable).unwrap_err();
        assert!(err.to_string().contains("拒绝修改"), "应提示拒绝修改，实际：{err}");
    }

    #[test]
    fn service_has_no_approved_record() {
        // 服务不属于 StartupApproved 机制：先给合法 location 通过护栏，
        // 再验证返回的是"不支持"而非"拒绝修改"
        let it = item(
            SourceKind::Service,
            "svc",
            Scope::Machine,
            r"HKLM\SYSTEM\CurrentControlSet\Services\svc",
        );
        let err = set_enabled(&it, Desired::Disable).unwrap_err();
        assert!(
            err.to_string().contains("不支持"),
            "服务应无 StartupApproved 写法，实际：{err}"
        );
    }

    #[test]
    fn run_machine32_maps_to_run32_key() {
        assert_eq!(subkey_for(SourceKind::RunMachine32), Some((true, "Run32")));
    }
}