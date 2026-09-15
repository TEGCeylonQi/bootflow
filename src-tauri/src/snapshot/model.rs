//! 快照数据结构 —— v0.2.0「可写可控」的地基。
//!
//! 一条快照 = 某一时刻所有启动项「定位 + 原值」的权威记录。
//! 它是回滚的唯一真相来源（rollback.rs）、变更日志的比对基准（changelog.rs）、
//! 以及独立 .ps1/.reg 脚本（export_script.rs）的输入。因此：
//!
//! 1. **定位必须自足** —— 快照必须能从 `id` 反推出「这个东西写在哪里」
//!    （注册表键 + 值名 / 任务名 / 服务名），不能依赖扫描器重新猜。
//! 2. **原值必须精确** —— `approved_raw` 存的是 `StartupApproved` 的
//!    逐字节 REG_BINARY，回滚时原样写回，绝不能重新编码（T5 硬规则 2：
//!    值名带前导空格 `" QQPCTray"` 不能 trim）。
//! 3. **schema 版本永续** —— 快照是跨版本可读的对外契约（计划 v1.0.0 冻结），
//!    加载旧版本快照时必须能够迁移或明确报错，绝不静默错读。
//!
//! 字段命名用 `rename_all = "camelCase"`，与前端 TS 类型对齐（同 model.rs 约定）。

// T40 阶段性标记：数据模型已实现，待 T41+ 的 writer / commands 消费后移除。
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// 快照格式版本。只在**破坏性变更**（字段重命名/删除/语义变化）时递增。
///
/// v0.2.0 首版定为 `1`。加载时：
/// - `schema == CURRENT` → 直接反序列化
/// - `schema < CURRENT` → 有迁移路径则迁移，否则明确报错
/// - `schema > CURRENT` → 快照来自更新的版本，拒绝读取（不能拿老程序读新契约）
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// 一条记录的写入目标 —— 「写在哪里」的精确描述。
///
/// 用 `source` 表达类型，用 `location`/`value_name` 表达落点，避免前端
/// 或回滚引擎需要自己翻译。`value_name` 是可选的：启动文件夹的快捷方式、
/// 计划任务、服务没有「值名」概念。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotTarget {
    /// 复用 `model::SourceKind`（序列化为其原始形式）
    pub source: String,
    /// 注册表键路径 / 文件夹路径 / 任务名 / 服务路径 —— 完整、自足。
    /// 例：`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
    pub location: String,
    /// StartupApproved 缓存键 / Run 值名（逐字节保留，含前导空格）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_name: Option<String>,
}

/// 一条启动项在快照中的记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRecord {
    /// 与 `StartupItem.id` 一致（稳定 UUID，跨扫描不变）
    pub id: String,
    /// 人话展示名（回滚确认时给用户看）
    pub display_name: String,
    /// 写目标（定位）
    pub target: SnapshotTarget,
    /// 进入可恢复的原值 —— `StartupApproved` 的原始 REG_BINARY（十六进制）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_raw: Option<String>,
    /// 计划任务的任务级 enabled 原值（true/false）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_enabled: Option<bool>,
    /// 触发器级 enabled 原值（有的任务只有任务级开关）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_enabled: Option<bool>,
    /// 服务启动类型原值（仅记录，回滚走原生 API）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_start_type: Option<u32>,
    /// 该项属于哪个作用域（user/machine），决定写到哪里
    pub scope: String,
    /// 是否属于禁改区（Locked）——快照本身不做拦截，护栏 guard.rs 负责
    pub risk: String,
}

/// 一个快照文件（= 一次写操作前的完整基线）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub schema_version: u32,
    /// 快照唯一 ID（UUID v5，由时间派生）
    pub id: String,
    /// ISO8601 创建时刻
    pub created_at: String,
    /// 人话描述（"改前基线：全量" / "回滚到 v0.1.5"）
    pub description: String,
    /// 触发原因（scan / modify / rollback）
    pub reason: SnapshotReason,
    /// 本条快照覆盖的启动项原值
    pub records: Vec<SnapshotRecord>,
}

/// 快照触发原因。回滚本身也会产生新快照，所以需要四态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapshotReason {
    /// 全量扫描前的基线
    Scan,
    /// 一次修改操作前
    Modify,
    /// 回滚动作前（回滚也是可回滚的）
    Rollback,
    /// 手动导出前
    Export,
}

/// 生成快照 id（UUID v5）。
///
/// 🔧 v0.2.0 阶段：尚未被 commands/scan 调用（T41+ 接入），
/// 待快照真正产生时启用；`_namespace` 预留为命名空间（避免落盘 id 碰撞）。
pub fn snapshot_id(_namespace: &str, created_at: &str) -> String {
    // 快照 id 不要求确定性跨机器，但保持同一时刻同一上下文可复现。
    // 用 v5 而非 v4 便于测试与审计（同一份内容不产生新 id）。
    let uid = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("BootFlow:{created_at}").as_bytes(),
    );
    uid.to_string()
}