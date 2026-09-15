/**
 * v0.2.0「可写可控」—— 快照 / 回滚 / 导出 的数据契约。
 *
 * 与 Rust 侧 `commands.rs` 的 v0.2.0 区块一一对应：
 *   dry_run_edits / apply_edits / list_snapshots / rollback_to / export_snapshot
 * 字段一律 camelCase（Rust 侧 `#[serde(rename_all = "camelCase")]`）。
 */

/** 后端预演返回的单个动作：before → after，前端直接渲染 */
export interface PlannedAction {
  itemId: string
  /** 面向用户的名称 */
  displayName: string
  /** 改哪个字段（enabled / startType / taskEnabled / triggerEnabled） */
  field: string
  /** 改动前值（字符串化） */
  before: string
  /** 改动后值 */
  after: string
  /** 一句话后果 */
  consequence: string
  /** 风险（影响配色与确认按钮文案） */
  risk: string
}

/** dry_run_edits 的返回 */
export interface DryRunOutcome {
  steps: PlannedAction[]
  /** 被护栏拒绝的项（Locked / 找不到），不进步骤区 */
  denied: string[]
  /** 无事可做（全部 noop 或已被拒绝） */
  noop: boolean
}

/** 前端传给后端的一条「期望修改」 */
export interface EditInput {
  itemId: string
  /** 期望启停状态 */
  enabled?: boolean
  /** 服务启动类型：2=自动 3=手动 */
  startType?: number
  /** 计划任务任务级开关 */
  taskEnabled?: boolean
  /** 计划任务触发器级开关 */
  triggerEnabled?: boolean
}

/** apply_edits 单项执行结果 */
export interface AppliedResult {
  itemId: string
  ok: boolean
  message?: string
}

/** apply_edits 的返回 */
export interface ApplyOutcome {
  /** 本次修改前基线快照 id（用于回滚） */
  snapshotId: string
  results: AppliedResult[]
  rollbackAvailable: boolean
}

/** 快照列表项摘要 */
export interface SnapshotSummary {
  id: string
  createdAt: string
  description: string
  reason: string
  recordCount: number
}

/** rollback_to 的返回 */
export interface RollbackOutcome {
  /** 恢复了多少项 */
  restored: number
  /** 跳过的项及理由 */
  skipped: string[]
  /** 回滚动作产生的新快照 id（可回滚的回滚） */
  newSnapshotId: string
}

/** export_snapshot 的返回 */
export interface ExportBundle {
  ps1: string
  reg: string
}