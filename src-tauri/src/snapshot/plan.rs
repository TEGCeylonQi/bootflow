//! Dry-run 预演引擎 —— 把「目标状态 diff」编译成动作清单，**纯函数、零副作用**。
//!
//! 预演（Dry-run）是「可逆性 > 一切」的第一道闸：用户点「应用」之前，
//! 先看到"将要发生什么"（before → after + 后果 + 风险），确认后才真正动手。
//!
//! 硬性要求（计划 §3.5.2 T43）：
//! 1. **纯函数** —— 不触碰系统，只根据入参计算。可单测、可复现。
//! 2. **before/after 与实际写入 100% 一致** —— 预演显示的 from/to 必须
//!    与事务层真正写进去的完全一致（真机验收会 dry-run 后 apply 比对）。
//! 3. **风险分级** —— 每个动作带上 `risk`，前端据此标色（Safe 绿 / High 橙）。
//! 4. **诚实** —— `consequence` 只说有据可依的（"顺序会变化"），
//!    不编造"会快 X 秒"（计划 §3.5.3 硬规则 5）。

use crate::model::{RiskLevel, StartupItem};

/// 一次原子动作。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedAction {
    pub item_id: String,
    /// 面向用户的名称
    pub display_name: String,
    /// 改哪个字段（`enabled` / `startType` / …）
    pub field: String,
    /// 改动前值（字符串化，供展示与回滚比对）
    pub before: String,
    /// 改动后值
    pub after: String,
    /// 一句话后果（"登录后不再自动启动"）
    pub consequence: String,
    /// 风险（影响回滚界面配色与确认按钮文案）
    pub risk: RiskLevel,
}

/// diff 的输入：目标状态中真正要改的字段。
///
/// 语义上等价于 `DesiredState`，但**只填要动的字段**；
/// `None` = 不改。用结构体而非直接透传 DesiredState，
/// 是为了把"清空/恢复默认"这些显式动作也表达出来。
#[derive(Debug, Clone, Default)]
pub struct EditRequest {
    pub enabled: Option<bool>,
    pub start_type: Option<u32>, // 服务：2=AUTO 3=DEMAND
    pub task_enabled: Option<bool>,
    pub trigger_enabled: Option<bool>,
}

/// 把 EditRequest 编译成一个（单个启动项的）动作计划。
///
/// `current`：启动项当前状态（`enabled`、`raw` 里的字段）。
///
/// ⚠️ 纯函数：不读系统，不写注册表。所有输入都来自参数。
pub fn plan_one(item: &StartupItem, edit: &EditRequest) -> Vec<PlannedAction> {
    let mut out = Vec::new();

    // —— 启停（StartupApproved / 任务级 / 服务都由 enabled 表达）——
    if let Some(desired) = edit.enabled {
        if item.enabled != desired {
            out.push(PlannedAction {
                item_id: item.id.clone(),
                display_name: display(item),
                field: "enabled".to_string(),
                before: item.enabled.to_string(),
                after: desired.to_string(),
                consequence: if desired {
                    "改为「启用」，开机/登录时运行".to_string()
                } else {
                    "改为「停用」，不再随开机启动（实体保留，可随时恢复）".to_string()
                },
                risk: item.risk,
            });
        }
    }

    // —— 服务启动类型（2 AUTO / 3 DEMAND）——
    if let Some(desired_type) = edit.start_type {
        // 预演期就拦截非法类型：DISABLED 是单向深坑，writer 也不接受，
        // 预演更不该展示一个注定失败的"将要发生"
        if desired_type != 2 && desired_type != 3 {
            return out; // 直接返回，不追加非法动作
        }
        let current = item
            .raw
            .get("startType")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        if current != Some(desired_type) {
            out.push(PlannedAction {
                item_id: item.id.clone(),
                display_name: display(item),
                field: "startType".to_string(),
                before: start_type_name(current),
                after: start_type_name(Some(desired_type)),
                consequence: match desired_type {
                    2 => "服务改为「自动」启动".to_string(),
                    _ => "服务改为「手动」启动，由需要时再拉起".to_string(),
                },
                risk: item.risk,
            });
        }
    }

    // —— 计划任务：任务级 / 触发器级（分开表达）——
    if let Some(desired) = edit.task_enabled {
        let current = item
            .raw
            .get("taskEnabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if current != desired {
            out.push(PlannedAction {
                item_id: item.id.clone(),
                display_name: display(item),
                field: "taskEnabled".to_string(),
                before: current.to_string(),
                after: desired.to_string(),
                consequence: if desired {
                    "计划任务改为「启用」".to_string()
                } else {
                    "计划任务改为「停用」（任务级）".to_string()
                },
                risk: item.risk,
            });
        }
    }

    // 触发器级（指 Boot/Logon 触发器）
    if let Some(desired) = edit.trigger_enabled {
        let current = item
            .raw
            .get("triggerEnabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if current != desired {
            out.push(PlannedAction {
                item_id: item.id.clone(),
                display_name: display(item),
                field: "triggerEnabled".to_string(),
                before: current.to_string(),
                after: desired.to_string(),
                consequence: if desired {
                    "开机/登录触发器改为「启用」".to_string()
                } else {
                    "开机/登录触发器改为「停用」（任务仍启用）".to_string()
                },
                risk: item.risk,
            });
        }
    }

    out
}

/// 展示名（预演时用户看到的名称）。
fn display(item: &StartupItem) -> String {
    item.display_name.clone().unwrap_or_else(|| item.name.clone())
}

/// 服务启动类型的人话名。
fn start_type_name(t: Option<u32>) -> String {
    match t {
        Some(2) => "自动".to_string(),
        Some(3) => "手动".to_string(),
        Some(4) => "禁用".to_string(),
        Some(v) => format!("未知({v})"),
        None => "未知".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BootPhase, DesiredState, ItemKind, ItemTiming, Scope, SignerInfo, ValidityStatus};

    fn item(id: &str, enabled: bool, source: crate::model::SourceKind) -> StartupItem {
        StartupItem {
            id: id.into(),
            source,
            identity_key: "k".into(),
            name: id.into(),
            kind: match source {
                crate::model::SourceKind::Service => ItemKind::Service,
                crate::model::SourceKind::ScheduledTask => ItemKind::Task,
                _ => ItemKind::App,
            },
            display_name: Some(id.into()),
            name_from: None,
            summary: None,
            command: String::new(),
            resolved_path: String::new(),
            args: vec![],
            location: String::new(),
            scope: Scope::User,
            enabled,
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
            raw: serde_json::json!({
                "startType": 2,
                "taskEnabled": true,
            }),
            desired: DesiredState::default(),
            snapshot_ref: None,
        }
    }

    #[test]
    fn enable_an_app_produces_one_action() {
        let it = item("app", false, crate::model::SourceKind::RunUser);
        let plan = plan_one(&it, &EditRequest { enabled: Some(true), ..Default::default() });
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].field, "enabled");
        assert_eq!(plan[0].before, "false");
        assert_eq!(plan[0].after, "true");
    }

    #[test]
    fn noop_edit_produces_no_actions() {
        // 已经是期望状态 → 无动作（预演必须诚实：什么都不改）
        let it = item("app", true, crate::model::SourceKind::RunUser);
        let plan = plan_one(&it, &EditRequest { enabled: Some(true), ..Default::default() });
        assert!(plan.is_empty());
    }

    #[test]
    fn service_start_type_change_is_planned() {
        let it = item("svc", true, crate::model::SourceKind::Service);
        let plan = plan_one(&it, &EditRequest { start_type: Some(3), ..Default::default() });
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].field, "startType");
        assert_eq!(plan[0].before, "自动");
        assert_eq!(plan[0].after, "手动");
    }

    #[test]
    fn task_enabled_change_is_planned() {
        let it = item("task", true, crate::model::SourceKind::ScheduledTask);
        let plan = plan_one(&it, &EditRequest { task_enabled: Some(false), ..Default::default() });
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].field, "taskEnabled");
        assert_eq!(plan[0].after, "false");
    }

    #[test]
    fn trigger_enabled_change_is_planned_separately() {
        let it = item("task", true, crate::model::SourceKind::ScheduledTask);
        let plan = plan_one(&it, &EditRequest { trigger_enabled: Some(false), ..Default::default() });
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].field, "triggerEnabled");
        assert_eq!(plan[0].after, "false");
    }

    #[test]
    fn disabled_bad_start_type_is_rejected_at_plan_time() {
        // DISABLED(4) 不该成为可预演的动作——它连 writer 都不接受
        let it = item("svc", true, crate::model::SourceKind::Service);
        let plan = plan_one(&it, &EditRequest { start_type: Some(4), ..Default::default() });
        // 预期 0 个动作：预演把非法类型直接过滤（计划 §3.5.3 硬规则 3）
        assert!(plan.is_empty(), "DISABLED 不应进入预演，实际 {} 个动作", plan.len());
    }
}