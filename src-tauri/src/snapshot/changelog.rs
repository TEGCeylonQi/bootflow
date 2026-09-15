//! 变更日志与审计 —— 追加式 `JSONL`，每条写操作一行，永不清除（计划 §3.5.2 T46）。
//!
//! 为什么是 JSONL 而不是再来一份快照：
//! - 快照是**状态**（某一时刻全量），日志是**事件流**（按时间顺序发生了什么）。
//! - 追加写入天然防并发：一行一条，进程崩溃最多丢半行，不会损坏历史。
//! - 审计用途：`changelog.jsonl` 只增不改，回滚、导出、比对自己保留，绝不重写。
//!
//! 每条 `ChangeEntry` 都带 `snapshot_id`，把「事件」和「那个时刻的状态」挂上钩——
//! 遇到纠纷时既能看"改了什么"，又能调出改前快照逐字段核对。
//!
//! 硬性要求：
//! 1. **追加式** —— 任何更新操作都只 `append`，不 truncate、不 rewrite。
//! 2. **可审计** —— 每条含时间 / 项 / 动作 / 原值 / 新值 / 快照 id / 结果。
//! 3. **导出 Markdown** —— 复用体检报告渲染（前端表格 + 复制按钮）。

// T46 阶段性标记：changelog 已实现，待 commands 层接线后移除。
#![allow(dead_code)]

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::error::{AppError, Result};
use crate::snapshot::model::SnapshotRecord;

/// 日志目录名（`%APPDATA%\BootFlow`）。
pub const LOG_DIR_NAME: &str = "BootFlow";
/// 日志文件名。
pub const LOG_FILE_NAME: &str = "changelog.jsonl";

/// 动作种类 —— 与事务层的动作/回滚一一对应，保证审计词表封闭。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// 启用 / 停用（StartupApproved / 任务级 / 触发器级统一用 Enable/Disable）
    Enable,
    Disable,
    /// 服务启动类型更改
    StartType,
    /// 回滚（整体动作标记；逐条还走 Enable/Disable/StartType）
    Rollback,
}

impl Action {
    /// 人话名（导出/前端展示）。
    pub fn label(&self) -> &'static str {
        match self {
            Action::Enable => "启用",
            Action::Disable => "停用",
            Action::StartType => "改启动类型",
            Action::Rollback => "回滚",
        }
    }
}

/// 一条审计记录。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeEntry {
    /// ISO8601 时间。
    pub at: String,
    /// 对应快照 id（挂到改前基线）。
    pub snapshot_id: String,
    /// 本次操作所属事务描述（"批量停用三项"）。
    pub txn_description: String,
    /// 被操作的项。
    pub item_id: String,
    /// 人话名。
    pub display_name: String,
    /// 原值（字符串化；供审计比对）。
    pub before: String,
    /// 新值。
    pub after: String,
    /// 结果（ok / 错误信息）——失败也会记录，便于事后复盘。
    pub result: String,
    /// 动作类型。
    pub action: Action,
    /// 操作来源（哪个"一键"按钮：单改/批改/回滚）。
    pub origin: String,
}

/// 构造审计记所需的「一次性入参」——把重复数据收纳成结构体，
/// 避免 `ok`/`fail` 的签名参数爆表（clippy::too_many_arguments）。
#[derive(Debug, Clone)]
pub struct ChangeDraft {
    pub snap_id: String,
    pub txn: String,
    pub rec: SnapshotRecord,
    pub action: Action,
    pub before: String,
    pub after: String,
    pub origin: String,
}

impl ChangeEntry {
    /// 构造一条"成功"记录。
    pub fn ok(at: &str, draft: &ChangeDraft) -> Self {
        ChangeEntry {
            at: at.to_string(),
            snapshot_id: draft.snap_id.clone(),
            txn_description: draft.txn.clone(),
            item_id: draft.rec.id.clone(),
            display_name: draft.rec.display_name.clone(),
            before: draft.before.clone(),
            after: draft.after.clone(),
            result: "ok".to_string(),
            action: draft.action,
            origin: draft.origin.clone(),
        }
    }

    /// 构造一条"失败"记录（审计也要记下发生了什么）。
    pub fn fail(at: &str, draft: &ChangeDraft, error: &str) -> Self {
        let mut e = Self::ok(at, draft);
        e.result = format!("error: {error}");
        e
    }
}

/// `%APPDATA%\BootFlow\changelog.jsonl`（与快照同根目录，日志是流水，快照是状态）。
pub fn log_path() -> Result<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .ok_or_else(|| AppError::Other("缺少 %APPDATA% 环境变量".into()))?;
    Ok(Path::new(&base).join(LOG_DIR_NAME).join(LOG_FILE_NAME))
}

/// 追加一条记录（原子性：JSONL 单行 append，系统调用级追加写）。
pub fn append(entry: &ChangeEntry) -> Result<()> {
    let path = log_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| AppError::Other(format!("创建日志目录失败：{e}")))?;
    }
    let mut line = serde_json::to_string(entry)
        .map_err(|e| AppError::Other(format!("序列化日志失败：{e}")))?;
    line.push('\n');

    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| AppError::Other(format!("打开日志失败（{}）：{e}", path.display())))?;
    f.write_all(line.as_bytes())
        .map_err(|e| AppError::Other(format!("追加日志失败：{e}")))?;
    f.flush()
        .map_err(|e| AppError::Other(format!("刷新日志失败：{e}")))?;
    Ok(())
}

/// 读取全部记录（按写入顺序，最旧在前；文件缺失 → 空表）。
///
/// 容错：一行损坏（JSON 解析失败）**跳过而不是整体失败**——审计日志永不
/// 因个别坏行而不可读（计划 §3.5.3 硬规则 1：损坏数据不致命）。
pub fn list() -> Vec<ChangeEntry> {
    let mut out = Vec::new();
    let Ok(path) = log_path() else {
        return out;
    };
    let Ok(f) = fs::File::open(&path) else {
        return out;
    };
    for line in BufReader::new(f).lines() {
        let Ok(line) = line else { continue };
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<ChangeEntry>(&line) {
            out.push(entry);
        }
        // 坏行忽略——不 fail 整个读取
    }
    out
}

/// 导出为 Markdown（复用体检报告的表格渲染风格）。
///
/// 返回 Markdown 字符串，由调用方落盘或放进前端预览。
pub fn export_markdown() -> String {
    let entries = list();
    let mut s = String::new();
    s.push_str("# 变更日志\n\n");
    s.push_str(&format!("共 {} 条记录\n\n", entries.len()));
    s.push_str("| 时间 | 项 | 动作 | 原值 | 新值 | 结果 | 快照 |\n");
    s.push_str("|---|---|---|---|---|---|---|\n");
    for e in &entries {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            e.at, e.display_name, e.action.label(), e.before, e.after, e.result, e.snapshot_id
        ));
    }
    s
}

/// 最近 N 条（供前端"最近变更"面板）。
pub fn recent(n: usize) -> Vec<ChangeEntry> {
    let all = list();
    all.into_iter().rev().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与 store.rs 相同的临时 %APPDATA% 沙箱。锁必须**共用** `crate::testenv` 那把：
    /// 两个模块改的是同一个 `APPDATA`，各持一把锁彼此拦不住，会随机失败。
    fn with_temp_appdata<F: FnOnce()>(test_name: &str, f: F) {
        let _guard = crate::testenv::lock();
        let dir = std::env::temp_dir().join(format!("bootflow-{}-{}", test_name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("APPDATA", &dir);
        f();
    }

    fn rec(id: &str) -> SnapshotRecord {
        SnapshotRecord {
            id: id.into(),
            display_name: format!("应用-{id}"),
            target: crate::snapshot::model::SnapshotTarget {
                source: "RunUser".into(),
                location: r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run".into(),
                value_name: Some("App".into()),
            },
            approved_raw: Some("0202".into()),
            task_enabled: None,
            trigger_enabled: None,
            service_start_type: None,
            scope: "user".into(),
            risk: "Safe".into(),
        }
    }

    /// 快速构造测试入参。
    fn draft_of(id: &str, action: Action, before: &str, after: &str) -> ChangeDraft {
        ChangeDraft {
            snap_id: "s".into(),
            txn: "tx".into(),
            rec: rec(id),
            action,
            before: before.into(),
            after: after.into(),
            origin: "o".into(),
        }
    }

    #[test]
    fn append_then_list_round_trips() {
        with_temp_appdata("append", || {
            let entry = ChangeEntry::ok(
                "2026-09-15T10:00:00Z",
                &ChangeDraft {
                    snap_id: "snap-1".into(),
                    txn: "批量停用".into(),
                    rec: rec("item-1"),
                    action: Action::Disable,
                    before: "true".into(),
                    after: "false".into(),
                    origin: "bulk".into(),
                },
            );
            append(&entry).unwrap();
            let entries = list();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].item_id, "item-1");
            assert_eq!(entries[0].action, Action::Disable);
            assert_eq!(entries[0].result, "ok");
        });
    }

    #[test]
    fn append_is_append_only_never_rewrites() {
        with_temp_appdata("APPEND", || {
            // 追加两条，第二条不覆盖第一条
            append(&ChangeEntry::ok("t1", &draft_of("a", Action::Enable, "f", "t"))).unwrap();
            append(&ChangeEntry::ok("t2", &draft_of("b", Action::Disable, "t", "f"))).unwrap();
            let all = list();
            assert_eq!(all.len(), 2);
            assert_eq!(all[0].item_id, "a");
            assert_eq!(all[1].item_id, "b");
        });
    }

    #[test]
    fn corrupted_line_is_skipped_not_fatal() {
        with_temp_appdata("CORRUPT", || {
            // 先写一条合法，再手动追加一条坏行，再写一条合法
            append(&ChangeEntry::ok("t1", &draft_of("x", Action::Enable, "f", "t"))).unwrap();
            let path = log_path().unwrap();
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(f, "{{{{ 这不是合法 JSON").unwrap();
            append(&ChangeEntry::ok("t2", &draft_of("y", Action::Enable, "f", "t"))).unwrap();

            let all = list();
            assert_eq!(all.len(), 2, "坏行应被跳过，而不是让整个日志不可读");
        });
    }

    #[test]
    fn export_markdown_contains_header_and_rows() {
        with_temp_appdata("MD", || {
            append(&ChangeEntry::ok("t", &draft_of("z", Action::Disable, "true", "false"))).unwrap();
            let md = export_markdown();
            assert!(md.contains("变更日志"));
            assert!(md.contains("停用"));
            assert!(md.contains("| true | false |"));
        });
    }
}