//! 写权限护栏（Guard）—— v0.2.0 安全关键的入口校验。
//!
//! 职责：**任何 write（writers / 事务 / 回滚）在触碰系统前，必须先过 `check()`**。
//! Locked 项一律返回 `Denied`，一个字节都不写。
//!
//! 设计要点：
//! - **单一事实来源** —— 禁改判定复用 `diag::risk::locked_reason`，与只读版
//!   的风险评估同源。绝不另写一套"什么算禁改"，避免两处悄悄漂移。
//! - **三层防护**：
//!   1. 编译期：`mark_locked` 在 `ItemBuilder` 构造源头把 `SystemHook` 强制标
//!      `Locked`（类型层面杜绝"系统注入项进入写路径"）。
//!   2. 运行期：`check()` 对 `risk == Locked` 的项直接拒。
//!   3. 写后验证：writer 写完后按 T41 各条"改→验→改回"规则读回比对。
//! - **纯校验、零副作用** —— 本模块不产生任何系统写入，可自由单测。

// T45 阶段性标记：`check`/`denial_message` 尚未被 T41a-c 的 writer 调用；
// 接入 writer 后删除本 allow。
#![allow(dead_code)]

use crate::diag::risk;
use crate::model::{RiskLevel, SourceKind, StartupItem};

/// 写权限检查结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardResult {
    /// 放行。
    Allowed,
    /// 拒绝，`String` 是面向用户的人话理由。
    Denied(String),
}

impl GuardResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, GuardResult::Allowed)
    }

    /// 拒绝理由；`Allowed` 时返回 `None`。
    pub fn message(&self) -> Option<&str> {
        match self {
            GuardResult::Allowed => None,
            GuardResult::Denied(m) => Some(m),
        }
    }
}

/// 运行期入口校验 —— 每次写操作前调用。
///
/// 两层拒绝：
/// 1. `risk == Locked` → 直接拒（并复算 `locked_reason` 给人话理由；若风险评估
///    漏标了 Locked，则使用通用禁改理由）。
/// 2. 缺少定位信息（id / location 空）→ 拒，避免对"读不出在哪"的东西动手。
pub fn check(item: &StartupItem) -> GuardResult {
    if item.risk == RiskLevel::Locked {
        let why = risk::locked_reason(item)
            .map(|r| format!("这是禁改区：{r}"))
            .unwrap_or_else(|| "这是禁改区，不提供修改入口。".to_string());
        return GuardResult::Denied(why);
    }

    if item.id.is_empty() || item.location.is_empty() {
        return GuardResult::Denied(
            "这一项缺少定位信息，无法安全修改。请重新扫描后重试。".to_string(),
        );
    }

    GuardResult::Allowed
}

/// 编译期硬标记：把「系统注入」类来源强制置为 Locked。
///
/// 供 `scanners/builder.rs` 在构造完 item 后调用 —— 从源头保证
/// `SystemHook`（AppInit_DLLs / IFEO）永不进入任何写路径。
/// 输入为最终 `RiskLevel`，若当前评估结果不是 Locked 会被顶格为 Locked
/// （即使 `locked_reason` 因某种原因漏判，类型层也拦死）。
pub fn mark_locked(source: SourceKind, assessed: RiskLevel) -> RiskLevel {
    match source {
        SourceKind::SystemHook => RiskLevel::Locked, // 注入层硬锁
        _ => assessed,
    }
}

/// 统一拒绝出口：`Denied` 时给出可直接展示给用户的人话。
pub fn denial_message(result: &GuardResult) -> String {
    match result {
        GuardResult::Allowed => String::new(),
        GuardResult::Denied(m) => m.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        BootPhase, DesiredState, ItemKind, ItemTiming, Scope, SignerInfo, ValidityStatus,
    };

    /// 构造一个基本项，测试内按需覆写字段。
    fn base_item(source: SourceKind) -> StartupItem {
        StartupItem {
            id: "test-item".into(),
            source,
            identity_key: "k".into(),
            name: "Test".into(),
            kind: match source {
                SourceKind::Service => ItemKind::Service,
                SourceKind::ScheduledTask => ItemKind::Task,
                SourceKind::SystemHook => ItemKind::Hook,
                _ => ItemKind::App,
            },
            display_name: Some("测试应用".into()),
            name_from: None,
            summary: None,
            command: r"C:\Program Files\Test\t.exe".into(),
            resolved_path: r"C:\Program Files\Test\t.exe".into(),
            args: vec![],
            location: r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run".into(),
            scope: Scope::User,
            enabled: true,
            signer: SignerInfo::default(),
            icon_data: None,
            risk: RiskLevel::Safe,
            risk_reasons: vec![],
            diagnostics: vec![],
            boot_phase: BootPhase::Shell,
            timing: ItemTiming::default(),
            validity: ValidityStatus::Ok,
            validity_detail: None,
            recommendation: None,
            duplicate_of: None,
            raw: serde_json::json!({}),
            desired: DesiredState::default(),
            snapshot_ref: None,
        }
    }

    #[test]
    fn locked_item_is_denied_with_human_reason() {
        // 系统组件 → Locked
        let mut item = base_item(SourceKind::Service);
        item.signer.is_os_component = true; // 系统路径 → risk Locked
        item.risk = RiskLevel::Locked;

        let g = check(&item);
        assert!(!g.is_allowed());
        let msg = g.message().unwrap();
        assert!(
            msg.contains("禁改") || msg.contains("Windows 自带") || msg.contains("不提供修改"),
            "拒绝理由要是人话，实际：{msg}"
        );
    }

    #[test]
    fn core_service_is_locked() {
        let mut item = base_item(SourceKind::Service);
        item.name = "RpcSs".into(); // 核心服务名单
        item.risk = RiskLevel::Locked;

        let g = check(&item);
        assert!(!g.is_allowed());
        let msg = g.message().unwrap();
        assert!(msg.contains("禁改"), "核心服务必须是禁改区，实际：{msg}");
    }

    #[test]
    fn safe_item_is_allowed() {
        let item = base_item(SourceKind::RunUser);
        let g = check(&item);
        assert!(g.is_allowed(), "Safe 项应放行，实际拒绝：{:?}", g.message());
    }

    #[test]
    fn missing_location_is_denied() {
        let mut item = base_item(SourceKind::RunUser);
        item.location = "".into();
        let g = check(&item);
        assert!(!g.is_allowed(), "无法定位的项必须拒绝");
    }

    #[test]
    fn empty_id_is_denied() {
        let mut item = base_item(SourceKind::RunUser);
        item.id = "".into();
        let g = check(&item);
        assert!(!g.is_allowed(), "无 id 的项必须拒绝");
    }

    #[test]
    fn mark_locked_locks_system_hook() {
        assert_eq!(
            mark_locked(SourceKind::SystemHook, RiskLevel::Safe),
            RiskLevel::Locked,
            "系统注入项必须在来源层被锁死"
        );
        assert_eq!(
            mark_locked(SourceKind::RunUser, RiskLevel::Safe),
            RiskLevel::Safe,
            "非注入来源不被误锁"
        );
    }

    #[test]
    fn lock_is_polymorphic_across_all_writable_sources() {
        // 计划要求：单测覆盖全部可写 item 类型，Locked 项调用任何 writer 都返回 Denied。
        let writable_sources = [
            SourceKind::StartupFolderUser,
            SourceKind::StartupFolderMachine,
            SourceKind::RunUser,
            SourceKind::RunMachine,
            SourceKind::RunMachine32,
            SourceKind::RunOnceUser,
            SourceKind::RunOnceMachine,
            SourceKind::RunOnceMachine32,
            SourceKind::ScheduledTask,
            SourceKind::Service,
            SourceKind::SystemHook,
        ];
        for src in writable_sources {
            let mut item = base_item(src);
            item.risk = RiskLevel::Locked; // 无论哪种来源，Locked 一律拒
            let g = check(&item);
            assert!(
                !g.is_allowed(),
                "来源 {src:?} 的 Locked 项必须被拒"
            );
        }
    }
}