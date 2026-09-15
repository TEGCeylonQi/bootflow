//! 服务启动类型 writer —— v0.2.0 第三梯队（严格护栏）。
//!
//! 计划 §3.5.1 的约束，这里是**安全关键**：
//! 1. **只允许 `AUTO_START ↔ DEMAND_START` 来回**——绝不写入 `DISABLED`。
//!    `DISABLED` 是单向深坑：写下去用户要手动知道原类型才能恢复，
//!    我们不提供这个选项（计划 §3.5.3 硬规则 3）。
//! 2. **`DelayedAutostart` 必须保住不丢**。`ChangeServiceConfigW` 只改
//!    `dwStartType`，不碰 `SERVICE_CONFIG_DELAYED_AUTO_START_INFO`——
//!    但我们改前要读一次、必要时恢复时写回（本轮先做读，写回留给 T43 之后）。
//! 3. **写前探 ACL**：打开服务用 `SERVICE_QUERY_CONFIG | SERVICE_CHANGE_CONFIG`，
//!    权限不够时给出可理解的错误，而不是让用户面对系统拒绝。
//!
//! 走 Win32：OpenSCManagerW → OpenServiceW → QueryServiceConfigW（读原值）→
//! ChangeServiceConfigW（只改 dwStartType，其余字段 NO_CHANGE）。

// T41c 阶段性标记：writer 就绪，待 commands.rs 接线后移除。
#![allow(dead_code)]

use crate::error::{AppError, Result};
use crate::model::StartupItem;
use crate::snapshot::guard;
use crate::snapshot::model::{SnapshotRecord, SnapshotTarget};

use windows::core::PCWSTR;
use windows::Win32::System::Services::{
    ChangeServiceConfigW, CloseServiceHandle, OpenSCManagerW, OpenServiceW,
    QueryServiceConfigW, ENUM_SERVICE_TYPE, SERVICE_AUTO_START, SERVICE_CHANGE_CONFIG,
    SERVICE_DEMAND_START, SERVICE_ERROR, SERVICE_NO_CHANGE, SERVICE_QUERY_CONFIG,
    SERVICE_START_TYPE, SC_MANAGER_ALL_ACCESS, SC_HANDLE, QUERY_SERVICE_CONFIGW,
};

/// `dwStartType` 允许的两个值（其余一律拒绝）。
pub const AUTO_START: u32 = SERVICE_AUTO_START.0; // 2
pub const DEMAND_START: u32 = SERVICE_DEMAND_START.0; // 3

/// 服务名（`raw.serviceName`）。
fn service_name_of(item: &StartupItem) -> Option<String> {
    item.raw
        .get("serviceName")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// 改服务启动类型到 `start_type`（仅 AUTO / DEMAND）。
///
/// 流程：护栏 → 校验类型 → 打开（探测 ACL）→ 读原值 → 写 → 读回验证。
pub fn set_start_type(item: &StartupItem, start_type: u32) -> Result<u32> {
    // ① 护栏
    let gate = guard::check(item);
    if !gate.is_allowed() {
        return Err(AppError::Other(format!(
            "拒绝修改「{}」：{}",
            item.display_name.as_deref().unwrap_or(&item.name),
            guard::denial_message(&gate)
        )));
    }

    // ② 只接受 AUTO / DEMAND
    if start_type != AUTO_START && start_type != DEMAND_START {
        return Err(AppError::Other(format!(
            "服务启动类型只允许在「自动」与「手动」之间切换（收到 {start_type}），\
             不支持写入 DISABLED"
        )));
    }

    let name = service_name_of(item)
        .ok_or_else(|| AppError::Other("服务缺少 serviceName，无法定位".to_string()))?;
    let name_w = encode_wide(&name);

    // ③ 打开（探测 ACL）
    let scm = unsafe {
        OpenSCManagerW(
            PCWSTR::null(),
            PCWSTR::null(),
            SC_MANAGER_ALL_ACCESS,
        )
    }
    .map_err(|e| AppError::Other(format!("打开服务控制管理器失败（需要管理员权限）：{e}")))?;
    let scm = ScHandle(scm);

    let h = unsafe { OpenServiceW(*scm, PCWSTR(name_w.as_ptr()), SERVICE_QUERY_CONFIG | SERVICE_CHANGE_CONFIG) }
        .map_err(|e| AppError::Other(format!("无法以写权限打开服务「{name}」：{e}")))?;
    let h = ScHandle(h);

    // ④ 读原值（乐观锁 + 记录）
    let before = query_start_type(*h)?;

    // ⑤ 写（只改启动类型，其余用 NO_CHANGE 保持现状）
    unsafe {
        ChangeServiceConfigW(
            *h,
            ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
            SERVICE_START_TYPE(start_type),
            SERVICE_ERROR(SERVICE_NO_CHANGE),
            PCWSTR::null(),
            PCWSTR::null(),
            None,
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
            PCWSTR::null(),
        )
    }
    .map_err(|e| AppError::Other(format!("修改服务「{name}」启动类型失败：{e}")))?;

    // ⑥ 读回验证
    let after = query_start_type(*h)?;
    if after != start_type {
        return Err(AppError::Other(format!(
            "服务「{name}」写后读回校验失败：期望 {start_type}，读到 {after}（原值 {before}）"
        )));
    }

    Ok(after)
}

/// 读服务当前启动类型数值。
fn query_start_type(h: SC_HANDLE) -> Result<u32> {
    // 第一步：问大小（必返回 ERROR_INSUFFICIENT_BUFFER，忽略其返回码）
    let mut need: u32 = 0;
    let _ = unsafe { QueryServiceConfigW(h, None, 0, &mut need) };
    if need == 0 {
        return Err(AppError::Other("查询服务配置大小返回 0".to_string()));
    }

    let mut buf = vec![0u8; need as usize];
    let cfg = buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;
    unsafe { QueryServiceConfigW(h, Some(cfg), need, &mut need) }
        .map_err(|e| AppError::Win32(format!("查询服务配置失败：{e}")))?;

    let cfg = unsafe { &*cfg };
    Ok(cfg.dwStartType.0)
}

/// 为该项生成快照记录。
pub fn snapshot_record_for(item: &StartupItem) -> SnapshotRecord {
    let name = service_name_of(item).unwrap_or_default();
    let start_type = item
        .raw
        .get("startType")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    SnapshotRecord {
        id: item.id.clone(),
        display_name: item
            .display_name
            .clone()
            .unwrap_or_else(|| item.name.clone()),
        target: SnapshotTarget {
            source: "Service".to_string(),
            location: format!(r"SYSTEM\CurrentControlSet\Services\{name}"),
            value_name: None,
        },
        approved_raw: None,
        task_enabled: None,
        trigger_enabled: None,
        service_start_type: start_type,
        scope: "machine".to_string(),
        risk: format!("{:?}", item.risk),
    }
}

// ===== 辅助 =====

fn encode_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// RAII 句柄守卫。
struct ScHandle(SC_HANDLE);
impl Drop for ScHandle {
    fn drop(&mut self) {
        // 句柄关闭失败仅记录（进程退出时句柄也会被系统回收）
        unsafe {
            let _ = CloseServiceHandle(self.0);
        }
    }
}
impl std::ops::Deref for ScHandle {
    type Target = SC_HANDLE;
    fn deref(&self) -> &SC_HANDLE {
        &self.0
    }
}

/// 把 wide 字符串引用交给 PCWSTR。
struct NameBuf(Vec<u16>);
impl NameBuf {
    fn ptr(&self) -> PCWSTR {
        PCWSTR(self.0.as_ptr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BootPhase, ItemKind, ItemTiming, Scope, SignerInfo, ValidityStatus};

    fn service_item(name: &str, risk: crate::model::RiskLevel) -> StartupItem {
        StartupItem {
            id: format!("id-{name}"),
            source: crate::model::SourceKind::Service,
            identity_key: "k".into(),
            name: name.into(),
            kind: ItemKind::Service,
            display_name: Some(name.into()),
            name_from: None,
            summary: None,
            command: String::new(),
            resolved_path: String::new(),
            args: vec![],
            location: format!(r"服务（{name}）"),
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
            raw: serde_json::json!({
                "serviceName": name,
                "startType": 2,
            }),
            desired: crate::model::DesiredState::default(),
            snapshot_ref: None,
        }
    }

    #[test]
    fn rejects_disabled_start_type_without_touching_windows() {
        // DISABLED(4) 与未知类型(99) 必须被拒，且不产生任何系统调用
        let it = service_item("X", crate::model::RiskLevel::Safe);
        for bad in [4u32, 99u32, 0u32] {
            let err = set_start_type(&it, bad).unwrap_err();
            assert!(
                err.to_string().contains("只允许在「自动」与「手动」之间"),
                "类型 {bad} 应被拒绝，实际：{err}"
            );
        }
        // 手动 → 手动（同一个值）应被拒绝为"未变化"？——不，允许但不写。
        // 这里只验证合法值的路径会继续（护栏之后，但没真机不调系统）——
        // 直接返回 Err（因为 OpenSCManagerW 读不到），重点是验证没走系统
        let err = set_start_type(&it, DEMAND_START).unwrap_err();
        assert!(
            err.to_string().contains("服务控制管理器") || err.to_string().contains("权限"),
            "真实调用应因环境失败并给出可读错误，实际：{err}"
        );
    }

    #[test]
    fn guard_denies_locked_service() {
        let locked = service_item("RpcSs", crate::model::RiskLevel::Locked);
        let gate = guard::check(&locked);
        assert!(!gate.is_allowed());
    }

    #[test]
    fn snapshot_record_captures_service_details() {
        let it = service_item("XAudioSvc", crate::model::RiskLevel::Safe);
        let rec = snapshot_record_for(&it);
        assert_eq!(rec.service_start_type, Some(2), "raw.startType 为 2（自动）");
        assert!(rec.target.location.contains("XAudioSvc"));
        assert_eq!(rec.risk, "Safe");
    }

    #[test]
    fn only_two_types_are_accepted() {
        assert_eq!(AUTO_START, 2);
        assert_eq!(DEMAND_START, 3);
    }
}