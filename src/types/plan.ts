/**
 * 编排层的数据契约。
 *
 * 【为什么单独建一层，而不是直接改 StartupItem.desired】
 *
 * 体检与编排是两种性质完全不同的操作：
 *   体检 → 读，描述**现状**
 *   编排 → 写，描述**意图**
 *
 * 如果把"我打算把它停用"直接写进 `StartupItem.desired`，界面就再也分不清
 * 屏幕上显示的是"这台电脑现在的样子"还是"我想要的样子"。用户会看着一张
 * 混了两者的清单，无法判断哪个改动已经生效、哪个还只是想法。
 *
 * 所以意图必须放在**独立的草稿层**：`StartupItem` 永远只描述现状，
 * 草稿层只描述"与现状的差异"，两者在界面上分开呈现。这也是撤销能够
 * 简单可靠的原因——撤销就是丢掉草稿，不需要回滚任何真实状态。
 */
import type { DesiredState, StartupItem } from './model'

/**
 * 变更类型。
 *
 * 刻意枚举而非自由字符串：每一种都对应**一个用户能自己复述的动作**。
 * 加新类型时先问一句"用户会怎么跟朋友描述这个操作"，答不上来就说明
 * 这个变更不该出现在一级界面。
 */
export type ChangeKind =
  /** 让它开机时不再自动运行（可逆：记录保留，随时开回来） */
  | 'disable'
  /** 把它重新打开 */
  | 'enable'
  /** 推迟一段时间再启动，给更重要的东西让路 */
  | 'delay'
  /** 调整 CPU 优先级，降低它对开机过程的影响 */
  | 'priority'
  /** 清理：这条启动记录已无作用，移除它 */
  | 'remove'
  /** 调整在启动序列中的先后位置 */
  | 'order'

/**
 * 变更后要写入的目标态。与 `DesiredState` 同构，但字段全部可选——只写被改动的那几项。
 *
 * `removed` 是唯一一个不属于 `DesiredState` 的字段：它表达的是"这条启动记录
 * 本身不该再存在"，而不是"它的某个属性该改成什么"。语义上无法用属性表达，
 * 所以单独列出来。只有 `remove` 类变更会用到它。
 */
export type PlanPatch = Partial<DesiredState> & {
  /** 目标态是"这条起始记录不该存在"（清理卸载残留） */
  removed?: boolean
}

/**
 * 草稿里的一条变更。
 *
 * `before` 是**操作那一刻的现状快照**，不是"当前值"。原因：
 * 扫描可能被重新触发，用户也可能在草稿未清空时切去别的项；
 * 如果 diff 每次都去 items 里现查，重扫之后"变更前/变更后"就会出现
 * 对不上的情况（比如现状已经变成停用了，草稿还写着"启用 → 停用"）。
 * 快照让每条变更自证，与外部状态解耦。
 */
export interface PlanEntry {
  itemId: string
  kind: ChangeKind
  /** 变更后的值 */
  next: PlanPatch
  /** 变更前的快照（从 StartupItem 抽取的相关字段） */
  before: PlanSnapshot
  /** 记录时间戳（毫秒），用于变更篮排序 */
  at: number
}

/** 一条变更所涉及的、可能被改动的现状字段 */
export interface PlanSnapshot {
  enabled: boolean
  delaySec?: number
  priorityClass?: number
  /** 在启动序列中的位次（同一启动位置内从 0 开始） */
  order?: number
}

/** 可编排字段的取值域——控件与校验共用同一份定义，避免两边写出不一致的范围 */
export const DELAY_PRESETS = [
  { sec: 0, label: '不延迟' },
  { sec: 10, label: '10 秒' },
  { sec: 30, label: '30 秒' },
  { sec: 60, label: '1 分钟' },
  { sec: 120, label: '2 分钟' },
] as const

export const PRIORITY_OPTIONS = [
  { value: 1, label: '最低', hint: '让出 CPU，适合后台同步、更新检查这类不着急的东西' },
  { value: 2, label: '普通', hint: '系统默认，和大多数程序一样' },
  { value: 3, label: '较高', hint: '优先进场，适合开机后马上就要用的东西' },
] as const

/** 界面模式。体检=只读看现状；编排=可以表达意图 */
export type AppMode = 'inspect' | 'orchestrate'

/**
 * 这一项能不能编排。
 *
 * 两道闸：
 * 1. `Locked` 风险等级（系统关键组件）——本工具任何版本都不提供修改入口
 * 2. 类型为 system（Windows 自带）——同上
 *
 * 判定放在这里而不是各控件里，是为了让"能不能改"有唯一答案：
 * 界面上灰掉的控件和拦住写入的逻辑用的是同一个函数，
 * 不会出现"控件能点但写完被拒"这种自相矛盾的体验。
 */
export function isOrchestrable(item: Pick<StartupItem, 'risk' | 'kind'>): boolean {
  if (item.risk === 'Locked') return false
  return item.kind !== 'system'
}

/** 不可编排的原因——必须能回答用户"为什么这里点不了" */
export function orchestrableReason(item: Pick<StartupItem, 'risk' | 'kind'>): string | null {
  if (item.risk === 'Locked') return '系统关键组件，本工具不提供修改入口'
  if (item.kind === 'system') return 'Windows 自带组件，改动它们通常会弄坏系统'
  return null
}

/** 从启动项抽取现状快照 */
export function snapshotOf(item: StartupItem): PlanSnapshot {
  return {
    enabled: item.enabled,
    delaySec: item.desired?.delaySec,
    priorityClass: item.desired?.priorityClass,
  }
}

/** 变更类型 → 用户看得懂的动作名 */
export const CHANGE_LABEL: Record<ChangeKind, string> = {
  disable: '停用',
  enable: '启用',
  delay: '设置延迟',
  priority: '调整优先级',
  remove: '清理记录',
  order: '调整顺序',
}

/**
 * 人话描述这条变更会做什么，用于变更篮列表。
 * 必须自包含——用户在一屏十条变更里，不能靠记忆去猜"停用"停用的是谁。
 */
export function describeChange(entry: PlanEntry, name: string): string {
  const { kind, next } = entry
  switch (kind) {
    case 'disable':
      return `不再自动启动「${name}」`
    case 'enable':
      return `恢复「${name}」开机自启`
    case 'delay': {
      const sec = next.delaySec ?? 0
      return sec === 0 ? `取消「${name}」的启动延迟` : `让「${name}」推迟 ${sec} 秒启动`
    }
    case 'priority': {
      const opt = PRIORITY_OPTIONS.find((o) => o.value === next.priorityClass)
      return `把「${name}」的 CPU 优先级设为${opt?.label ?? next.priorityClass}`
    }
    case 'remove':
      return `移除「${name}」这条已失效的启动记录`
    case 'order': {
      const o = next.order ?? 0
      return `把「${name}」挪到启动序列第 ${o + 1} 位`
    }
  }
}

/* ─────────────────────────────────────────────────────────────
   变更之间的相互约束
   ───────────────────────────────────────────────────────────── */

/**
 * 这个补丁是否等于现状（也就是"改了等于没改"）。
 *
 * 用户很容易把开关拨过去又拨回来。如果每次都往变更篮里塞一条，
 * 篮子会积累一堆"启用 → 启用"这种什么也不做的条目，
 * 真正要紧的几条就被淹了。所以写入前先判断，空变更直接从草稿里撤掉。
 */
export function isNoopPatch(patch: PlanPatch, snapshot: PlanSnapshot): boolean {
  if (patch.removed) return false
  if (patch.enabled !== undefined && patch.enabled !== snapshot.enabled) return false
  if (patch.delaySec !== undefined && patch.delaySec !== (snapshot.delaySec ?? 0)) return false
  if (patch.priorityClass !== undefined && patch.priorityClass !== snapshot.priorityClass) return false
  if (patch.order !== undefined && patch.order !== snapshot.order) return false
  return true
}

/**
 * 变更是否彼此冲突。
 *
 * 已经停用的东西再设延迟是没有意义的——系统根本不会启动它，延迟自然也不会发生。
 * 这类"看起来设置了但其实不起作用"的状态最坑人：用户以为安排好了，
 * 实际上什么都没发生，而且界面上还显示得好好的。
 *
 * 所以在写入前就挡住，并给出原因，而不是让用户设置完再困惑。
 */
export function conflictReason(
  kind: ChangeKind,
  next: PlanPatch,
  snapshot: PlanSnapshot,
): string | null {
  const willBeDisabled = next.enabled === false || (next.enabled === undefined && !snapshot.enabled)

  if (willBeDisabled && kind === 'delay') {
    return '它已经不再自动启动了，延迟设置不会起作用。需要延迟的话，先把它启用。'
  }
  if (willBeDisabled && kind === 'priority') {
    return '它已经不再自动启动了，优先级设置不会起作用。'
  }
  if (willBeDisabled && kind === 'order') {
    return '它已经不再自动启动了，调整顺序不会起作用。'
  }
  return null
}

/**
 * 计算「现在 → 之后」的差异行，供变更篮与详情面板展示。
 * 只返回真正发生变化的字段，不列出没动的项——否则每条变更都挂 4 行，没人看得完。
 */
export interface DiffRow {
  label: string
  from: string
  to: string
}

const PRIORITY_TEXT: Record<number, string> = { 1: '最低', 2: '普通', 3: '较高' }

export function diffRowsOf(entry: PlanEntry): DiffRow[] {
  const rows: DiffRow[] = []
  const { before, next } = entry

  if (next.enabled !== undefined && next.enabled !== before.enabled) {
    rows.push({
      label: '自动启动',
      from: before.enabled ? '开启' : '关闭',
      to: next.enabled ? '开启' : '关闭',
    })
  }

  if (next.delaySec !== undefined && next.delaySec !== (before.delaySec ?? 0)) {
    const fmt = (s?: number) => (!s ? '不延迟' : `${s} 秒`)
    rows.push({ label: '启动延迟', from: fmt(before.delaySec), to: fmt(next.delaySec) })
  }

  if (next.priorityClass !== undefined && next.priorityClass !== before.priorityClass) {
    const fmt = (p?: number) =>
      p === undefined ? '系统默认' : (PRIORITY_TEXT[p] ?? String(p))
    rows.push({ label: 'CPU 优先级', from: fmt(before.priorityClass), to: fmt(next.priorityClass) })
  }

  if (next.order !== undefined && next.order !== before.order) {
    const fmt = (o?: number) => (o === undefined ? '当前位置' : `第 ${o + 1} 位`)
    rows.push({ label: '启动顺序', from: fmt(before.order), to: fmt(next.order) })
  }

  if (next.removed) {
    rows.push({ label: '启动记录', from: '保留', to: '移除' })
  }

  return rows
}
