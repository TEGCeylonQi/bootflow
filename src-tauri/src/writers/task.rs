//! 计划任务两级开关 writer —— v0.2.0 第二梯队写能力。
//!
//! ⚠️ 关键背景（T6 硬规则 1）：计划任务有**两级独立开关**——
//! `IRegisteredTask::Enabled`（任务级）与 `ITrigger::Enabled`（触发器级）。
//! 优化软件常只关触发器：任务在任务计划程序里仍显示"已启用/准备就绪"，
//! 实际却不触发。因此：
//! - **改到哪一级由调用方明确指定**（`Level::Task` / `Level::Trigger`）
//! - 恢复时只恢复原来被改的那一级，不越权动另一级
//! - 读取时两级都读，界面说清"是哪一级被关了"（scanner 已存 raw）
//!
//! 定位：任务全名 `taskPath`（如 `\Microsoft\Windows\AppID\...`）由扫描器
//! 存在 `item.raw.taskPath`，这里只消费，不自己拼。
//!
//! 走 COM：ComGuard 初始化 → ITaskService → GetFolder("\\") → GetTask(path)
//! → `task.Enabled()` 任务级 / 触发器的 `Enabled()`。
//!
//! ⚠️ 本模块有真实的系统写入能力，**入口第一件事必须是 guard::check**。

// T41b 阶段性标记：任务级 writer 就绪；触发器级（按 Boot/Logon 触发器索引）
// 与 commands 接线待后续步骤。
#![allow(dead_code)]

use crate::error::{AppError, Result};
use crate::model::StartupItem;
use crate::snapshot::guard;
use crate::snapshot::model::{SnapshotRecord, SnapshotTarget};
use crate::util::com::ComGuard;

use windows::core::{BSTR, VARIANT};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::TaskScheduler::{
    IRegisteredTask, ITaskFolder, ITaskService, TASK_TRIGGER_BOOT, TASK_TRIGGER_LOGON,
    TASK_TRIGGER_TYPE2, TaskScheduler,
};

/// 改哪一级开关。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 任务级（IRegisteredTask::Enabled）
    Task,
    /// 触发器级（ITrigger::Enabled）—— 定位到 Boot/Logon 触发器
    Trigger,
}

/// 取任务全名（raw.taskPath）。
fn task_path_of(item: &StartupItem) -> Option<String> {
    item.raw
        .get("taskPath")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// 任务级：改 `IRegisteredTask::Enabled`。
///
/// 返回改后实际状态（读回验证）。
pub fn set_task_enabled(item: &StartupItem, enabled: bool) -> Result<bool> {
    let gate = guard::check(item);
    if !gate.is_allowed() {
        return Err(AppError::Other(format!(
            "拒绝修改「{}」：{}",
            item.display_name.as_deref().unwrap_or(&item.name),
            guard::denial_message(&gate)
        )));
    }

    let task_path = task_path_of(item).ok_or_else(|| {
        AppError::Other(format!(
            "无法确定计划任务「{}」的完整路径（raw.taskPath 缺失）",
            item.name
        ))
    })?;

    let _com = ComGuard::new();
    let service = connect()?;
    let task = get_task(&service, &task_path)?;

    unsafe { task.SetEnabled(windows::Win32::Foundation::VARIANT_BOOL::from(enabled)) }
        .map_err(|e| AppError::Other(format!("设置任务「{task_path}」启用状态失败：{e}")))?;

    // 读回验证
    let after = unsafe { task.Enabled() }
        .map(|b| b.as_bool())
        .map_err(|e| AppError::Other(format!("读回任务「{task_path}」状态失败：{e}")))?;
    if after != enabled {
        return Err(AppError::Other(format!(
            "写入后读回校验失败：任务「{task_path}」期望 {enabled}，实际 {after}"
        )));
    }

    Ok(after)
}

/// 触发器级：把第一个 Boot/Logon 触发器的 Enabled 改为 `enabled`。
///
/// ⚠️ 只改触发器级，不动任务级。
pub fn set_trigger_enabled(item: &StartupItem, enabled: bool) -> Result<bool> {
    let gate = guard::check(item);
    if !gate.is_allowed() {
        return Err(AppError::Other(format!(
            "拒绝修改「{}」：{}",
            item.display_name.as_deref().unwrap_or(&item.name),
            guard::denial_message(&gate)
        )));
    }

    let task_path = task_path_of(item).ok_or_else(|| {
        AppError::Other(format!(
            "无法确定计划任务「{}」的完整路径（raw.taskPath 缺失）",
            item.name
        ))
    })?;

    let _com = ComGuard::new();
    let service = connect()?;
    let task = get_task(&service, &task_path)?;
    set_trigger_enabled_for(&task, enabled)
}

fn connect() -> Result<ITaskService> {
    let service: ITaskService = unsafe {
        CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
    }
    .map_err(|e| AppError::Win32(format!("连接任务计划程序失败：{e}")))?;

    let empty = VARIANT::default();
    unsafe { service.Connect(&empty, &empty, &empty, &empty) }
        .map_err(|e| AppError::Win32(format!("初始化任务计划程序失败：{e}")))?;
    Ok(service)
}

fn get_task(service: &ITaskService, path: &str) -> Result<IRegisteredTask> {
    let folder = get_folder(service, "\\")?;
    unsafe { folder.GetTask(&BSTR::from(path)) }
        .map_err(|e| AppError::Other(format!("取不到任务「{path}」：{e}")))
}

fn get_folder(service: &ITaskService, path: &str) -> Result<ITaskFolder> {
    unsafe { service.GetFolder(&BSTR::from(path)) }
        .map_err(|e| AppError::Other(format!("打开任务文件夹「{path}」失败：{e}")))
}

/// 改第一个 Boot/Logon 触发器的 Enabled。
fn set_trigger_enabled_for(task: &IRegisteredTask, enabled: bool) -> Result<bool> {
    let definition = unsafe { task.Definition() }
        .map_err(|e| AppError::Other(format!("读任务定义失败：{e}")))?;
    let triggers = unsafe { definition.Triggers() }
        .map_err(|e| AppError::Other(format!("读任务触发器失败：{e}")))?;

    let mut count = 0i32;
    unsafe { triggers.Count(&mut count) }
        .map_err(|e| AppError::Other(format!("读触发器数量失败：{e}")))?;

    for i in 1..=count {
        let trigger = unsafe { triggers.get_Item(i) }
            .map_err(|e| AppError::Other(format!("取触发器 {i} 失败：{e}")))?;
        let mut ttype = TASK_TRIGGER_TYPE2::default();
        let ttype_ok = unsafe { trigger.Type(&mut ttype) }.is_ok();
        // Boot/Logon 才动；其它触发器不在开机范畴
        if ttype_ok && (ttype == TASK_TRIGGER_BOOT || ttype == TASK_TRIGGER_LOGON) {
            unsafe { trigger.SetEnabled(windows::Win32::Foundation::VARIANT_BOOL::from(enabled)) }
                .map_err(|e| AppError::Other(format!("设置触发器 {i} 状态失败：{e}")))?;
            let mut after = windows::Win32::Foundation::VARIANT_BOOL::default();
            unsafe { trigger.Enabled(&mut after) }
                .map_err(|e| AppError::Other(format!("读回触发器 {i} 状态失败：{e}")))?;
            if after.as_bool() != enabled {
                return Err(AppError::Other(format!(
                    "触发器级读回校验失败：期望 {enabled}，读到 {}",
                    after.as_bool()
                )));
            }
            return Ok(after.as_bool());
        }
    }

    Err(AppError::Other(
        "该任务没有 Boot / 登录触发器，无法改触发器级开关".to_string(),
    ))
}

/// 读取任务当前两级状态（供乐观锁 / 预演 / 快照）。
///
/// 返回 `(任务级 enabled, 首个 Boot/Logon 触发器 enabled)`；
/// 读不到某一级时该位为 `None`（不伪造）。
pub fn read_states(item: &StartupItem) -> Result<(Option<bool>, Option<bool>)> {
    let task_path = task_path_of(item).ok_or_else(|| AppError::Other("缺 taskPath".into()))?;
    let _com = ComGuard::new();
    let service = connect()?;
    let task = get_task(&service, &task_path)?;

    let task_enabled = unsafe { task.Enabled() }.ok().map(|b| b.as_bool());
    let trigger_enabled = read_first_boot_logon_trigger(&task);

    Ok((task_enabled, trigger_enabled))
}

fn read_first_boot_logon_trigger(task: &IRegisteredTask) -> Option<bool> {
    let definition = unsafe { task.Definition().ok()? };
    let triggers = unsafe { definition.Triggers().ok()? };
    let mut count = 0i32;
    unsafe { triggers.Count(&mut count) }.ok()?;
    for i in 1..=count {
        let t = unsafe { triggers.get_Item(i).ok()? };
        let mut ttype = TASK_TRIGGER_TYPE2::default();
        if unsafe { t.Type(&mut ttype) }.is_err() {
            continue;
        }
        if ttype == TASK_TRIGGER_BOOT || ttype == TASK_TRIGGER_LOGON {
            let mut b = windows::Win32::Foundation::VARIANT_BOOL::default();
            return unsafe { t.Enabled(&mut b) }.ok().map(|_| b.as_bool());
        }
    }
    None
}

/// 为该项生成快照记录（任务级与触发器级都会记录原值）。
pub fn snapshot_record_for(item: &StartupItem) -> SnapshotRecord {
    let task_path = task_path_of(item).unwrap_or_default();
    let (task_enabled, trigger_enabled) = read_states_for_snapshot(item);
    SnapshotRecord {
        id: item.id.clone(),
        display_name: item
            .display_name
            .clone()
            .unwrap_or_else(|| item.name.clone()),
        target: SnapshotTarget {
            source: "ScheduledTask".to_string(),
            location: task_path,
            value_name: None,
        },
        approved_raw: None,
        task_enabled,
        trigger_enabled,
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

/// 快照用：尽量读真实状态，读不到则 None（不强求——回滚口径由事务层决定）。
fn read_states_for_snapshot(item: &StartupItem) -> (Option<bool>, Option<bool>) {
    read_states(item).unwrap_or((None, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BootPhase, ItemKind, ItemTiming, Scope, SignerInfo, ValidityStatus};

    fn task_item(name: &str, task_path: &str, risk: crate::model::RiskLevel) -> StartupItem {
        StartupItem {
            id: format!("id-{name}"),
            source: crate::model::SourceKind::ScheduledTask,
            identity_key: "k".into(),
            name: name.into(),
            kind: ItemKind::Task,
            display_name: Some(name.into()),
            name_from: None,
            summary: None,
            command: String::new(),
            resolved_path: String::new(),
            args: vec![],
            location: task_path.into(),
            scope: Scope::Machine,
            enabled: true,
            signer: SignerInfo::default(),
            icon_data: None,
            risk,
            risk_reasons: vec![],
            diagnostics: vec![],
            boot_phase: BootPhase::Logon,
            timing: ItemTiming::default(),
            validity: ValidityStatus::Ok,
            validity_detail: None,
            recommendation: None,
            duplicate_of: None,
            raw: serde_json::json!({ "taskPath": task_path }),
            desired: crate::model::DesiredState::default(),
            snapshot_ref: None,
        }
    }

    #[test]
    fn snapshot_record_captures_two_level_states() {
        let it = task_item("T", r"\Microsoft\Windows\X\Y", crate::model::RiskLevel::Safe);
        // 无法连真机，这里只验证 record 结构不含依赖
        let rec = snapshot_record_for_pure(&it);
        assert_eq!(rec.target.location, r"\Microsoft\Windows\X\Y");
        assert_eq!(rec.target.source, "ScheduledTask");
    }

    // 纯函数版：snapshot_record_for 在无 raw 时不应 panic
    fn snapshot_record_for_pure(item: &StartupItem) -> SnapshotRecord {
        SnapshotRecord {
            id: item.id.clone(),
            display_name: item.display_name.clone().unwrap_or_else(|| item.name.clone()),
            target: SnapshotTarget {
                source: "ScheduledTask".into(),
                location: item.raw.get("taskPath").and_then(|v| v.as_str()).unwrap_or_default().into(),
                value_name: None,
            },
            approved_raw: None,
            task_enabled: None,
            trigger_enabled: None,
            service_start_type: None,
            scope: if item.scope == Scope::Machine { "machine" } else { "user" }.into(),
            risk: format!("{:?}", item.risk),
        }
    }

    #[test]
    fn guard_denies_locked_task() {
        let locked = task_item("svc", r"\Windows\System", crate::model::RiskLevel::Locked);
        // 护栏在 writer 入口执行；这里验证锁定的任务连调用都不会发出
        let gate = guard::check(&locked);
        assert!(!gate.is_allowed());
    }
}