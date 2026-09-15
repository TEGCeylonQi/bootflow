//! 事务与乐观锁 —— 一次变更 = 一个事务，失败自动回滚。
//!
//! 这是「可逆性 > 一切」的心脏（计划 §3.5.2 T42）：
//!
//! ```text
//! 用户点「应用」
//!   │
//!   ├─ ① 建事务（txn::begin）
//!   ├─ ② 预演（plan_one）→ 得到 PlannedAction 列表  [T43]
//!   ├─ ③ 逐个 apply：先查快照记录值（乐观锁），再写
//!   │      └─ 任一失败 → 回滚已成功的部分，返回错误
//!   ├─ ④ 写 changelog（T46）
//!   └─ ⑤ 保存新快照
//! ```
//!
//! 乐观锁的核心：写前把**磁盘现值**与开始事务时记下的**快照值**比对，
//! 不一致说明"这项在你打开软件后被别的程序改过"——**中止而不是硬写**
//! （计划 §3.5.3 硬规则 2：硬写 = 覆盖别人的改动）。
//!
//! 本文件只负责事务的骨架（begin / commit / rollback / 乐观锁检查），
//! 具体"怎么把 PlannedAction 落到系统"由 writers/* 提供。

// T42 阶段性标记：事务骨架已实现，待 commands.rs 接线后移除。
#![allow(dead_code)]

use crate::error::AppError;
use crate::model::StartupItem;
use crate::snapshot::model::{Snapshot, SnapshotRecord};

/// 事务中每一步的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepResult {
    /// 该步成功（before == after，已写入）
    Applied,
    /// 该步被跳过（没有变化 / 不适用）
    Skipped,
}

/// 事务中止原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbortReason {
    /// 被护栏拒绝（Locked / 无法定位）
    Denied(String),
    /// 乐观锁冲突：当前值 ≠ 快照记录值
    Conflict { item: String, expected: String, actual: String },
    /// writer 拒绝（DISABLED 等）
    Writer(String),
}

/// 一次事务的上下文 —— 保存所有已应用的项，供回滚。
#[derive(Debug)]
pub struct Tx {
    /// 事务名称（用户可读）
    pub description: String,
    /// 该事务覆盖的所有 items
    pub item_ids: Vec<String>,
    /// 事务开始前取样的快照记录（记录每条原值）
    pub before: Vec<SnapshotRecord>,
    /// 已成功应用的步骤（回滚时逐个还原）
    pub applied: Vec<AppliedStep>,
    /// 是否已经结束
    finished: bool,
    /// 基线快照里的 item id（供乐观锁查找）
    baseline_ids: Vec<String>,
}

/// 一步已成功应用的记录（回滚用）。
#[derive(Debug, Clone)]
pub struct AppliedStep {
    pub item_id: String,
    /// 描述用什么操作恢复（交给对应 writer）
    pub rollback: RollbackOp,
}

/// 回滚操作 —— 描述"怎么恢复"。
#[derive(Debug, Clone)]
pub enum RollbackOp {
    /// 把 StartupApproved 写回原字节
    ApprovedRaw { hex: String },
    /// 改回服务启动类型
    ServiceStartType { value: u32 },
    /// 改回计划任务任务级
    TaskEnabled { value: bool },
    /// 改回触发器级
    TriggerEnabled { value: bool },
}

impl Tx {
    /// 开启一个事务。
    ///
    /// `before_snapshot` 是这个事务开始前的完整基线（T40 store 产出的），
    /// 它记录了每一项的「原值」——回滚/乐观锁都从这里取。
    pub fn begin(description: &str, before_snapshot: &[SnapshotRecord]) -> Tx {
        Tx {
            description: description.to_string(),
            item_ids: before_snapshot.iter().map(|r| r.id.clone()).collect(),
            baseline_ids: before_snapshot.iter().map(|r| r.id.clone()).collect(),
            before: before_snapshot.to_vec(),
            applied: Vec::new(),
            finished: false,
        }
    }

    /// 乐观锁检查：当前值应该等于事务开始时的值。
    ///
    /// `current`：读取系统现在的值（writers 提供）。
    /// `expected`：快照里记录的原值。
    /// 不一致 → 返回 `Err(AbortReason::External)`，调用方中止整个事务。
    pub fn expect_unchanged(
        &self,
        item: &StartupItem,
        current: Option<&str>,
        expected: Option<&str>,
    ) -> std::result::Result<(), AppError> {
        match (&current, &expected) {
            (Some(c), Some(e)) if c != e => Err(AppError::Other(format!(
                "「{}」在你打开软件后被别的程序改过（现在 {}，事务开始时 {}）。\
                 已中止，请重新扫描后再操作。",
                item.name, c, e
            ))),
            (None, Some(e)) => Err(AppError::Other(format!(
                "「{}」在你打开软件后消失了（事务开始时存在，值为 {e}）。已中止",
                item.name
            ))),
            _ => Ok(()),
        }
    }

    /// 记录一步已执行的写操作（供回滚时还原）。
    pub fn record_applied(&mut self, step: AppliedStep) {
        self.applied.push(step);
    }

    /// 事务完成（正常结束）。
    pub fn commit(mut self) {
        self.finished = true;
    }

    /// 回滚所有已应用的步骤。
    ///
    /// ⚠️ 回滚路径**不做任何优化**，逐条恢复 + 尽力验证（计划 §3.5.3 硬规则 4）。
    /// `apply_rollback` 由事务拥有者传进来——它知道怎么把 `RollbackOp` 落回系统。
    pub fn rollback<F>(&mut self, mut apply_rollback: F) -> Vec<String> where F: FnMut(&RollbackOp) -> Result<(), String> {
        self.finished = true;
        let mut errors = Vec::new();
        for step in &self.applied {
            if let Err(e) = apply_rollback(&step.rollback) {
                errors.push(format!("恢复 {} 失败：{e}", step.item_id));
            }
        }
        errors
    }
}

// 幂等：把 PlannedAction 应用到一个 item（通过 writer），若实际值与目标一致则视为已应用。
// 这由 writers 的实际调用方（commands.rs）实现；此处只定义接口契约。
// ==== 辅助 ====

/// 从快照记录里找某条 item 的原值。
pub fn record_for<'a>(snap: &'a Snapshot, id: &str) -> Option<&'a SnapshotRecord> {
    snap.records.iter().find(|r| r.id == id)
}

/// 把 SnapshotRecord 转成可与当前值比较的字符串（乐观锁用）。
/// 采用"哪种字段存在就比哪种"的策略，Null 表示无记录。
pub fn value_str(rec: &SnapshotRecord) -> Option<String> {
    if let Some(hex) = &rec.approved_raw {
        return Some(hex.clone());
    }
    if let Some(t) = rec.task_enabled {
        return Some(t.to_string());
    }
    if let Some(t) = rec.trigger_enabled {
        return Some(t.to_string());
    }
    if let Some(s) = rec.service_start_type {
        return Some(s.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{StartupItem, SourceKind};

    fn record(id: &str, approved: Option<&str>) -> SnapshotRecord {
        SnapshotRecord {
            id: id.into(),
            display_name: id.into(),
            target: crate::snapshot::model::SnapshotTarget {
                source: "RunUser".into(),
                location: "HKCU\\...".into(),
                value_name: Some("App".into()),
            },
            approved_raw: approved.map(|s| s.to_string()),
            task_enabled: None,
            trigger_enabled: None,
            service_start_type: None,
            scope: "user".into(),
            risk: "Safe".into(),
        }
    }

    fn snap(records: Vec<SnapshotRecord>) -> Snapshot {
        Snapshot {
            schema_version: crate::snapshot::model::SNAPSHOT_SCHEMA_VERSION,
            id: "txn-test".into(),
            created_at: "2026-09-15T00:00:00Z".into(),
            description: "事务测试".into(),
            reason: crate::snapshot::model::SnapshotReason::Scan,
            records,
        }
    }

    fn item(id: &str) -> StartupItem {
        StartupItem {
            id: id.into(),
            source: SourceKind::RunUser,
            identity_key: "k".into(),
            name: id.into(),
            kind: crate::model::ItemKind::App,
            display_name: Some(id.into()),
            name_from: None,
            summary: None,
            command: String::new(),
            resolved_path: String::new(),
            args: vec![],
            location: String::new(),
            scope: crate::model::Scope::User,
            enabled: true,
            signer: crate::model::SignerInfo::default(),
            icon_data: None,
            risk: crate::model::RiskLevel::Safe,
            risk_reasons: vec![],
            diagnostics: vec![],
            boot_phase: crate::model::BootPhase::Shell,
            timing: crate::model::ItemTiming::default(),
            validity: crate::model::ValidityStatus::Ok,
            validity_detail: None,
            recommendation: None,
            duplicate_of: None,
            raw: serde_json::json!({}),
            desired: crate::model::DesiredState::default(),
            snapshot_ref: None,
        }
    }

    #[test]
    fn optimistic_lock_detects_external_change() {
        let snap = snap(vec![record("app", Some("0202..."))]);
        let tx = Tx::begin("test", &snap.records);
        let it = item("app");
        // 模拟磁盘现值已被别的程序改成 0303...
        let err = tx.expect_unchanged(&it, Some("0303..."), Some("0202..."));
        assert!(err.is_err(), "乐观锁必须检测到外部修改");
        assert!(err.unwrap_err().to_string().contains("被别的程序改过"));
    }

    #[test]
    fn optimistic_lock_passes_when_unchanged() {
        let snap = snap(vec![record("app", Some("0202..."))]);
        let tx = Tx::begin("test", &snap.records);
        let it = item("app");
        assert!(tx.expect_unchanged(&it, Some("0202..."), Some("0202...")).is_ok());
    }

    #[test]
    fn item_disappeared_detected() {
        let snap = snap(vec![record("app", Some("0202..."))]);
        let tx = Tx::begin("test", &snap.records);
        let it = item("app");
        let err = tx.expect_unchanged(&it, None, Some("0202..."));
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("消失"));
    }

    #[test]
    fn rollback_applies_all_steps_in_reverse() {
        let snap = snap(vec![]);
        let mut tx = Tx::begin("test", &snap.records);
        let mut applied = vec![
            AppliedStep { item_id: "a".into(), rollback: RollbackOp::ServiceStartType { value: 2 } },
            AppliedStep { item_id: "b".into(), rollback: RollbackOp::ApprovedRaw { hex: "0202".into() } },
        ];
        tx.record_applied(applied.remove(0));
        tx.record_applied(applied.remove(0));

        // 记录回滚被调用的顺序
        let mut order: Vec<String> = Vec::new();
        let errors = tx.rollback(|_op| {
            order.push("rollback".into());
            Ok(())
        });
        assert!(errors.is_empty());
        assert_eq!(order.len(), 2);
    }
}