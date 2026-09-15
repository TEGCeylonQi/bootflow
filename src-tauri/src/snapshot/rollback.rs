//! 回滚引擎（T44）—— 回滚到任意快照，且**回滚本身也产生新快照**（可回滚的回滚）。
//!
//! 设计原则：
//! 1. **纯函数** —— `plan_rollback` 只比较「目标快照」与「当前状态」两组数据，
//!    产出恢复动作清单，不碰系统。真正执行由事务层（txn.rs）负责。
//! 2. **只回滚改过的** —— 当前值 == 快照原值时跳过（无差异不折腾）。
//! 3. **项消失不追回** —— 快照里有、当前不存在（用户手动删了实体）时跳过，
//!    不做"复活实体"这种越界行为（v0.2.0 只恢复启停状态，不恢复被删实体）。
//! 4. **回滚也是快照** —— `new_rollback_snapshot` 生成 reason=Rollback 的新快照，
//!    一次回滚的成果本身可再次回滚（回滚栈）。
//!
//! 验收（计划 §3.5.2 T44）：改 N 项 → 回滚 → 全量扫描结果与改前**逐字段 diff = 0**。
//! 见 tests::rollback_restores_baseline_exactly_diff_zero（状态机模拟）。

// T44 阶段性标记：`new_rollback_snapshot` 待 commands.rs 接线后启用，届时移除。
#![allow(dead_code)]

use std::collections::HashMap;

use crate::snapshot::model::{Snapshot, SnapshotReason, SnapshotRecord, SNAPSHOT_SCHEMA_VERSION};
use crate::snapshot::txn::RollbackOp;
use crate::snapshot::txn::value_str;

/// 一步回滚：把某启动项恢复到快照原值。
#[derive(Debug, Clone)]
pub struct RollbackStep {
    pub item_id: String,
    pub display_name: String,
    /// 恢复动作（交给对应 writer 执行）。
    pub op: RollbackOp,
    /// 当前值（恢复前）。
    pub before: String,
    /// 目标值（快照原值）。
    pub after: String,
}

/// 一份回滚计划（= 若干步 + 摘要信息）。
#[derive(Debug, Clone)]
pub struct RollbackPlan {
    /// 回滚依据的快照。
    pub snapshot_id: String,
    /// 具体要执行的步骤。
    pub steps: Vec<RollbackStep>,
    /// 跳过的项（理由前缀："unchanged"/"missing"/"no-value"）——诚实告知用户。
    pub skipped: Vec<String>,
}

impl RollbackPlan {
    pub fn is_noop(&self) -> bool {
        self.steps.is_empty()
    }
}

/// 纯函数：把「目标快照」与「当前状态」逐项比对，计算回滚动作。
///
/// `current`：`id → 当前值字符串`（用与 `value_str` 相同的编码）。
/// 快照项取 `value_str` 作为目标；两者相异才生成步骤。
///
/// ⚠️ 纯计算，零副作用：可单测、可预览（回滚前先展示"将恢复什么"）。
pub fn plan_rollback(snapshot: &Snapshot, current: &HashMap<String, Option<String>>) -> RollbackPlan {
    let mut plan = RollbackPlan {
        snapshot_id: snapshot.id.clone(),
        steps: Vec::new(),
        skipped: Vec::new(),
    };

    for rec in &snapshot.records {
        let Some(target) = value_str(rec) else {
            plan.skipped.push(format!("{}：快照无记录值", rec.display_name));
            continue;
        };
        let cur = current.get(&rec.id).cloned().unwrap_or(None);
        match cur {
            None => {
                // 该项当前不存在（实体被删/不在扫描中）——不追回
                plan.skipped.push(format!("{}：当前不存在，跳过", rec.display_name));
            }
            Some(cur) if cur == target => {
                // 未变化 —— 无需回滚
                plan.skipped.push(format!("{}：未变化", rec.display_name));
            }
            Some(cur) => {
                // 有差异：根据快照中记录的字段类型构造恢复动作
                if let Some(op) = op_for(rec) {
                    plan.steps.push(RollbackStep {
                        item_id: rec.id.clone(),
                        display_name: rec.display_name.clone(),
                        op,
                        before: cur.clone(),
                        after: target.clone(),
                    });
                } else {
                    plan.skipped.push(format!("{}：无可用恢复字段", rec.display_name));
                }
            }
        }
    }
    plan
}

/// 从快照记录构造恢复动作（快照里哪个字段有值就恢复哪个）。
fn op_for(rec: &SnapshotRecord) -> Option<RollbackOp> {
    if let Some(hex) = &rec.approved_raw {
        return Some(RollbackOp::ApprovedRaw { hex: hex.clone() });
    }
    if let Some(v) = rec.task_enabled {
        return Some(RollbackOp::TaskEnabled { value: v });
    }
    if let Some(v) = rec.trigger_enabled {
        return Some(RollbackOp::TriggerEnabled { value: v });
    }
    if let Some(v) = rec.service_start_type {
        return Some(RollbackOp::ServiceStartType { value: v });
    }
    None
}

/// 回滚本身也产生新快照（reason = Rollback）——「可回滚的回滚」。
///
/// `records`：回滚后的全量状态（通常来自回滚执行后的重新扫描结果）。
pub fn new_rollback_snapshot(records: Vec<SnapshotRecord>, description: &str) -> Snapshot {
    let now = chrono::Utc::now().to_rfc3339();
    Snapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        id: crate::snapshot::model::snapshot_id("BootFlow", &now),
        created_at: now,
        description: description.to_string(),
        reason: SnapshotReason::Rollback,
        records,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::model::{SnapshotTarget, Snapshot};

    fn rec(
        id: &str,
        approved: Option<&str>,
        task: Option<bool>,
        trigger: Option<bool>,
        svc: Option<u32>,
    ) -> SnapshotRecord {
        SnapshotRecord {
            id: id.into(),
            display_name: format!("应用-{id}"),
            target: SnapshotTarget {
                source: "RunUser".into(),
                location: r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run".into(),
                value_name: Some("App".into()),
            },
            approved_raw: approved.map(|s| s.to_string()),
            task_enabled: task,
            trigger_enabled: trigger,
            service_start_type: svc,
            scope: "user".into(),
            risk: "Safe".into(),
        }
    }

    fn snap(records: Vec<SnapshotRecord>) -> Snapshot {
        Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            id: "rollback-target".into(),
            created_at: "2026-09-15T00:00:00Z".into(),
            description: "改前基线".into(),
            reason: SnapshotReason::Scan,
            records,
        }
    }

    /// 一条覆盖 5 类来源的基线快照（验收用）。
    fn snapshot_baseline() -> Snapshot {
        snap(vec![
            rec("app1", Some("0202"), None, None, None),
            rec("svc1", None, None, None, Some(2)),
            rec("task1", None, Some(true), None, None),
            rec("trg1", None, None, Some(true), None),
            rec("app2", Some("0202"), None, None, None),
        ])
    }

    /// 从快照取某项的原值（模拟"执行回滚后的状态"）。
    fn value_of(s: &Snapshot, id: &str) -> Option<String> {
        s.records.iter().find(|r| r.id == id).and_then(value_str)
    }

    #[test]
    fn unchanged_items_produce_no_steps() {
        let s = snap(vec![rec("a", Some("0202"), None, None, None)]);
        let mut cur = HashMap::new();
        cur.insert("a".to_string(), Some("0202".to_string()));
        let plan = plan_rollback(&s, &cur);
        assert!(plan.is_noop(), "未变化的项不需要回滚：{:?}", plan.steps);
        assert_eq!(plan.steps.len(), 0);
    }

    #[test]
    fn changed_item_produces_approved_raw_step() {
        let s = snap(vec![rec("a", Some("0202"), None, None, None)]);
        let mut cur = HashMap::new();
        // 当前是 0303（被禁用），快照是 0202（原本启用）
        cur.insert("a".to_string(), Some("0303".to_string()));
        let plan = plan_rollback(&s, &cur);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].item_id, "a");
        assert_eq!(plan.steps[0].before, "0303");
        assert_eq!(plan.steps[0].after, "0202");
        match &plan.steps[0].op {
            RollbackOp::ApprovedRaw { hex } => assert_eq!(hex, "0202"),
            other => panic!("期望 ApprovedRaw，实际 {other:?}"),
        }
    }

    #[test]
    fn service_and_task_fields_restored() {
        let s = snap(vec![
            rec("svc", None, None, None, Some(2)),
            rec("task", None, Some(true), None, None),
        ]);
        let mut cur = HashMap::new();
        cur.insert("svc".to_string(), Some("3".to_string())); // 手动→自动
        cur.insert("task".to_string(), Some("false".to_string())); // 停用→启用
        let plan = plan_rollback(&s, &cur);
        assert_eq!(plan.steps.len(), 2);
        assert!(matches!(plan.steps[0].op, RollbackOp::ServiceStartType { value: 2 }));
        assert!(matches!(plan.steps[1].op, RollbackOp::TaskEnabled { value: true }));
    }

    #[test]
    fn missing_item_is_skipped_not_restored() {
        let s = snap(vec![rec("gone", Some("0202"), None, None, None)]);
        let mut cur = HashMap::new();
        cur.insert("gone".to_string(), None); // 当前不存在
        let plan = plan_rollback(&s, &cur);
        assert!(plan.is_noop());
        assert_eq!(plan.skipped.len(), 1);
        assert!(plan.skipped[0].contains("不存在"));
    }

    #[test]
    fn rollback_restores_baseline_exactly_diff_zero() {
        // 验收核心：改 5 项 → 回滚 → 与基线逐字段 diff = 0
        let baseline = snapshot_baseline();
        // 模拟改动后的当前状态（把他改成"被改过"的样子）
        let mut current = HashMap::new();
        current.insert("app1".into(), Some("0303".into())); // approved 改
        current.insert("svc1".into(), Some("3".into()));    // start_type 改
        current.insert("task1".into(), Some("false".into())); // task 改
        current.insert("trg1".into(), Some("false".into())); // trigger 改
        current.insert("app2".into(), Some("0202".into())); // 未动 → 与快照同

        let plan = plan_rollback(&baseline, &current);
        // 4 项有差异（app1/svc1/task1/trg1），应该各 1 步；app2 跳过
        assert_eq!(plan.steps.len(), 4);
        assert_eq!(plan.skipped.len(), 1);

        // 模拟执行回滚：把当前状态中每个改过的恢复成快照值
        let mut restored = current.clone();
        for step in &plan.steps {
            let target = value_of(&baseline, &step.item_id);
            restored.insert(step.item_id.clone(), target);
        }
        // 与基线 diff == 0：每一项当前值都等于快照原值
        for rec in &baseline.records {
            let cur = restored.get(&rec.id).cloned().unwrap_or(None);
            let target = value_str(rec);
            assert_eq!(cur, target, "项 {} 回滚后与基线不一致", rec.display_name);
        }
    }
}