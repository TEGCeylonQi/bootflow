import { Lock, RotateCcw, SlidersHorizontal } from 'lucide-react'
import { usePlanStore } from '@/store/usePlanStore'
import type { StartupItem } from '@/types/model'
import {
  DELAY_PRESETS,
  PRIORITY_OPTIONS,
  type ChangeKind,
  type PlanPatch,
  conflictReason,
  describeChange,
  diffRowsOf,
  isNoopPatch,
  isOrchestrable,
  orchestrableReason,
  snapshotOf,
} from '@/types/plan'
import { ControlRow, Toggle } from '@/components/common/Toggle'
import { displayNameOf } from '@/lib/item'

const PLAN_COLOR = '#a371f7'

/**
 * 「编排设置」区块。
 *
 * 【为什么把只读版换成可编辑版】
 * 上一版这里是一排灰掉的占位字段，本意是"预告产品走向"。但灰字段传达的信息
 * 是"这里有东西但你用不了"，而不是"你将来能做什么"——用户看完只会觉得功能没做完。
 * 现在改成真的能拨动，但**改动只进草稿、不动系统**，把边界写在明处而不是灰在暗处。
 *
 * 【体检模式仍然保持只读】
 * 体检的语义是"看看现在什么样"。在这个模式下控件可编辑，会让用户分不清
 * 屏幕上的值到底是机器的现状，还是自己刚才拨的。所以模式切换本身
 * 就是一道语义闸门：切过去之前，界面只描述事实。
 */
export function OrchestrationBlock({ item }: { item: StartupItem }) {
  const mode = usePlanStore((s) => s.mode)
  const entry = usePlanStore((s) => s.entries[item.id])
  const stage = usePlanStore((s) => s.stage)
  const unstage = usePlanStore((s) => s.unstage)
  const setMode = usePlanStore((s) => s.setMode)

  const locked = !isOrchestrable(item)
  const lockReason = orchestrableReason(item)

  const snapshot = snapshotOf(item)
  /** 当前生效值 = 草稿里的目标值（若有）盖在现状之上 */
  const enabled = entry?.next.enabled ?? item.enabled
  const delaySec = entry?.next.delaySec ?? item.desired?.delaySec ?? 0
  const priority = entry?.next.priorityClass ?? item.desired?.priorityClass ?? 2

  /**
   * 写入草稿。
   *
   * 三件事在这里一次做完，避免散落到每个控件里各写一遍：
   * 1. 新值盖在已有改动之上（先设延迟、再改优先级，不能把延迟冲掉）
   * 2. 结果与现状相同 → 撤掉这条草稿（拨回去就等于没动）
   * 3. 否则正常记录
   */
  const apply = (kind: ChangeKind, patch: PlanPatch) => {
    const merged: PlanPatch = { ...(entry?.next ?? {}), ...patch }
    if (isNoopPatch(merged, snapshot)) {
      unstage(item.id)
      return
    }
    stage(item, kind, merged)
  }

  // ——— 不可编排：说清为什么，而不是给一个点不动的灰控件 ———
  if (locked) {
    return (
      <div className="space-y-1.5">
        <div className="flex items-start gap-1.5 rounded border border-line-subtle bg-panel-soft px-2 py-1.5">
          <Lock size={11} className="mt-[3px] shrink-0 text-ink-dim" />
          <div className="min-w-0">
            <p className="text-mini leading-4 text-ink-muted">不提供修改入口</p>
            <p className="mt-0.5 text-2xs leading-4 text-ink-dim">{lockReason}</p>
          </div>
        </div>
      </div>
    )
  }

  // ——— 体检模式：只描述现状 ———
  if (mode === 'inspect') {
    return (
      <div className="space-y-2">
        <div className="rounded border border-line-subtle bg-panel-soft px-2 py-2">
          <ControlRow label="自动启动">
            <span className="text-mini" style={{ color: item.enabled ? '#3fb950' : '#6e7681' }}>
              {item.enabled ? '开机时自动运行' : '已停用'}
            </span>
          </ControlRow>
          <ControlRow label="启动延迟">
            <span className="text-mini text-ink-muted">
              {delaySec > 0 ? `${delaySec} 秒` : '不延迟'}
            </span>
          </ControlRow>
          <ControlRow label="CPU 优先级">
            <span className="text-mini text-ink-muted">
              {PRIORITY_OPTIONS.find((p) => p.value === priority)?.label ?? '普通'}
            </span>
          </ControlRow>
        </div>

        <button
          type="button"
          onClick={() => setMode('orchestrate')}
          className="flex w-full items-center justify-center gap-1.5 rounded border px-2 py-1.5 text-mini transition-colors"
          style={{ borderColor: `${PLAN_COLOR}66`, color: PLAN_COLOR }}
        >
          <SlidersHorizontal size={12} />
          进入编排模式，调整这一项
        </button>
      </div>
    )
  }

  // ——— 编排模式：可编辑 ———
  // 目标是"停用"时，延迟与优先级都不会发生——此时禁用控件并说明原因，
  // 而不是让用户设置完一个永远不生效的值
  const delayConflict = conflictReason('delay', { enabled }, snapshot)
  const priorityConflict = conflictReason('priority', { enabled }, snapshot)

  const diff = entry ? diffRowsOf(entry) : []

  return (
    <div className="space-y-2">
      <div className="rounded border border-line-subtle bg-panel-soft px-2 py-2">
        <ControlRow label="自动启动" hint="关掉后它不会随开机运行，记录会保留，随时可以开回来">
          <Toggle
            label={`${displayNameOf(item)} 开机自动启动`}
            checked={enabled}
            onChange={(v) => apply(v ? 'enable' : 'disable', { enabled: v })}
          />
          <span className="text-mini" style={{ color: enabled ? '#3fb950' : '#6e7681' }}>
            {enabled ? '开机时自动运行' : '不随开机运行'}
          </span>
        </ControlRow>

        <ControlRow label="启动延迟" hint="推迟一段时间再启动，把开机的头几秒让给更要紧的东西">
          {delayConflict ? (
            <span className="text-2xs leading-4 text-ink-faint">{delayConflict}</span>
          ) : (
            <div className="flex flex-wrap gap-1">
              {DELAY_PRESETS.map((d) => (
                <Chip
                  key={d.sec}
                  active={delaySec === d.sec}
                  onClick={() => apply('delay', { delaySec: d.sec === 0 ? undefined : d.sec })}
                >
                  {d.label}
                </Chip>
              ))}
            </div>
          )}
        </ControlRow>

        <ControlRow label="CPU 优先级" hint="优先级低的东西会让出处理器时间，减少对开机的争抢">
          {priorityConflict ? (
            <span className="text-2xs leading-4 text-ink-faint">{priorityConflict}</span>
          ) : (
            <div className="flex flex-wrap gap-1">
              {PRIORITY_OPTIONS.map((p) => (
                <Chip
                  key={p.value}
                  active={priority === p.value}
                  title={p.hint}
                  onClick={() => apply('priority', { priorityClass: p.value })}
                >
                  {p.label}
                </Chip>
              ))}
            </div>
          )}
        </ControlRow>
      </div>

      {entry ? (
        <div
          className="animate-slide-up rounded border px-2 py-1.5"
          style={{ borderColor: `${PLAN_COLOR}4d`, background: `${PLAN_COLOR}0f` }}
        >
          <div className="flex items-center gap-1.5">
            <span className="text-mini leading-4" style={{ color: PLAN_COLOR }}>
              已放入变更篮
            </span>
            <div className="flex-1" />
            <button
              type="button"
              onClick={() => unstage(item.id)}
              className="flex items-center gap-1 text-2xs text-ink-dim transition-colors hover:text-ink"
            >
              <RotateCcw size={11} />
              撤回
            </button>
          </div>

          <p className="mt-0.5 text-mini leading-4 text-ink-muted">
            {describeChange(entry, displayNameOf(item))}
          </p>

          {diff.length > 0 && (
            <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 border-t border-line-subtle pt-1">
              {diff.map((d) => (
                <span key={d.label} className="text-2xs text-ink-dim">
                  {d.label}
                  <span className="mx-1 text-ink-faint line-through">{d.from}</span>
                  <span className="text-ink-faint">→ </span>
                  <span style={{ color: PLAN_COLOR }}>{d.to}</span>
                </span>
              ))}
            </div>
          )}
        </div>
      ) : (
        <p className="text-2xs leading-4 text-ink-faint">
          改动会先收进底部变更篮，确认后才产出方案——本版本不会直接改动系统。
        </p>
      )}
    </div>
  )
}

function Chip({
  active,
  title,
  onClick,
  children,
}: {
  active: boolean
  title?: string
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      title={title}
      onClick={onClick}
      className="rounded border px-1.5 py-[1px] text-2xs leading-4 transition-colors"
      style={{
        borderColor: active ? PLAN_COLOR : '#30363d',
        color: active ? PLAN_COLOR : '#8b949e',
        background: active ? `${PLAN_COLOR}1f` : 'transparent',
      }}
    >
      {children}
    </button>
  )
}
