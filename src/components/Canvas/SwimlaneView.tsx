import { useMemo, useState } from 'react'
import { useAppStore } from '@/store/useAppStore'
import { usePlanStore } from '@/store/usePlanStore'
import { KIND_META, PHASE_LABEL, PHASE_ORDER, RISK_META, SOURCE_PLAIN } from '@/constants'
import type { BootPhase, PhaseSpan, StartupItem } from '@/types/model'
import { isOrchestrable } from '@/types/plan'
import { AppIcon } from '@/components/common/Icon'
import { Badge } from '@/components/common/Badge'
import { displayNameOf, resolveKind, shortPublisher } from '@/lib/item'

const fmtMs = (ms: number) => `${(ms / 1000).toFixed(1)}s`
const PLAN_COLOR = '#a371f7'

/**
 * 这一项在时间轴上的位置。实测的观测时刻优先，退到相位起点。
 *
 * 返回 `1e9` 表示"没有位置"——排在最后，而不是排在最前。
 * 用 `??` 而不是 `||`：`0` 是一个合法时刻（开机后 0 秒），不能被当成缺失。
 */
const startOf = (it: StartupItem): number =>
  it.timing.observedStartMs ?? it.timing.startEstimateMs ?? 1e9

/**
 * 卡片上那个耗时角标的三种形态。
 *
 * 三档必须**看起来就不一样**，否则用户会把估算当实测：
 *
 * | 档 | 边框 | 文案 | 说的是什么 |
 * |---|---|---|---|
 * | 实测耗时 | 实线 · 蓝 | `3.2s` | 系统事件日志记了这一项花了 3.2 秒 |
 * | 实测出现时刻 | 实线 · 青 | `@12.4s` | 内核记的：它在开机后 12.4 秒出现 |
 * | 估算 | 虚线 · 灰 | `~19.4s` | 它所属相位的起点，**不是**它自己的时刻 |
 *
 * 第二档最容易说错：`@12.4s` 是"什么时候出现"，不是"花了多久"。
 * 所以它的 tooltip 必须把这句话写出来——只靠颜色区分是不够的。
 */
function timingBadge(item: StartupItem): { text: string; color: string; title: string; solid: boolean } | null {
  const { timing } = item
  if (timing.confidence === 'measured' && timing.durationMs !== undefined) {
    return {
      text: fmtMs(timing.durationMs),
      color: '#58a6ff',
      solid: true,
      title: `系统记录了它上一次开机启动花了 ${fmtMs(timing.durationMs)}（事件 ${timing.sourceEventId ?? '—'}）。这是实测值。`,
    }
  }
  if (timing.confidence === 'observed' && timing.observedStartMs !== undefined) {
    return {
      text: `@${fmtMs(timing.observedStartMs)}`,
      color: '#39c5bb',
      solid: true,
      title:
        `内核记录了它的进程创建时刻：开机后 ${fmtMs(timing.observedStartMs)} 出现。` +
        '这是「出现时刻」，不是它花了多久——Windows 不为没有异常表现的启动项记录耗时。',
    }
  }
  if (timing.confidence === 'estimated' && timing.startEstimateMs !== undefined) {
    return {
      text: `~${fmtMs(timing.startEstimateMs)}`,
      color: '#6e7681',
      solid: false,
      title:
        `按开机相位推算：它属于「${PHASE_LABEL[item.bootPhase] ?? '未知阶段'}」这一段，` +
        '而这一段从开机后这个时刻开始。同一相位里的所有项都是这个数字，所以它不是这一项自己的启动时刻。',
    }
  }
  return null
}

/**
 * 泳道视图：泳道 = 开机阶段，卡片 = 启动项。
 *
 * 卡片边框区分数据可信度：实测（measured）用实线，估算（estimated）用虚线，
 * 这是「诚实原则」在界面上的体现，避免用户把估算值当成实测值。
 *
 * ─────────────────────────────────────────────────────────────
 * 【拖拽编排：为什么只在同一启动方式内部生效】
 *
 * 用户的第一直觉是"把不急的东西拖到后面去"。这个直觉在**同一启动方式**内
 * 完全成立：注册表 Run 键里的条目确实有先后顺序，计划任务的触发器时刻也是
 * 一个明确的排队依据。执行层照着写就行。
 *
 * 但跨越启动方式就不成立了——把一条「计划任务」拖到某条「注册表启动项」前面，
 * 在系统里根本没有一个地方能表达这个先后关系：它们由两套互不相干的机制拉起，
 * 谁先谁后取决于系统当时的调度，不是我们能设定的。
 *
 * 所以跨来源拖拽会被**明确拒绝并说明原因**，而不是默默接受、生成一份
 * 执行不了的方案。让用户以为自己安排好了，比直接告诉他"这条路不通"要糟糕得多。
 * ─────────────────────────────────────────────────────────────
 */
export function SwimlaneView({ items }: { items: StartupItem[] }) {
  const bootTimeline = useAppStore((s) => s.bootTimeline)
  const planEntries = usePlanStore((s) => s.entries)
  const stageMany = usePlanStore((s) => s.stageMany)
  const setMode = usePlanStore((s) => s.setMode)

  const [dragId, setDragId] = useState<string | null>(null)
  const [dropAt, setDropAt] = useState<{ id: string; side: 'before' | 'after' } | null>(null)
  const [hint, setHint] = useState<string | null>(null)

  const lanes = useMemo(() => {
    const map = new Map<BootPhase, StartupItem[]>()
    for (const p of [...PHASE_ORDER, 'unknown' as BootPhase]) map.set(p, [])
    for (const it of items) map.get(it.bootPhase)?.push(it)

    for (const [, list] of map) {
      list.sort((a, b) => {
        // 已被编排过顺序的项优先按编排后的位次排——
        // 用户刚拖完就该立刻看到它落在新位置上，否则这次拖拽像没生效
        const oa = planEntries[a.id]?.next.order
        const ob = planEntries[b.id]?.next.order
        if (oa !== undefined || ob !== undefined) {
          if (oa === undefined) return 1
          if (ob === undefined) return -1
          if (oa !== ob) return oa - ob
        }
        // 无编排顺序时按**启动时刻**升序，两者都没有的排最后。
        // 优先用实测的观测时刻：它是这一项自己的时间位置（内核记录）；
        // 相位估算只是"它属于哪一段"，同相位的项会拿到同一个数字。
        return startOf(a) - startOf(b)
      })
    }

    const spanOf = (p: BootPhase): PhaseSpan | undefined =>
      bootTimeline?.phases.find((s) => s.name === p)

    return [...map.entries()]
      .filter(([, list]) => list.length > 0)
      .map(([phase, list]) => ({ phase, list, span: spanOf(phase) }))
  }, [items, bootTimeline, planEntries])

  const say = (msg: string) => {
    setHint(msg)
    window.setTimeout(() => setHint((cur) => (cur === msg ? null : cur)), 3600)
  }

  /** 拖拽落定：重排同来源一组，作为一次动作写入草稿 */
  const commitDrop = (phase: BootPhase, targetId: string, side: 'before' | 'after') => {
    const lane = lanes.find((l) => l.phase === phase)?.list ?? []
    const dragged = lane.find((i) => i.id === dragId)
    const target = lane.find((i) => i.id === targetId)
    if (!dragged || !target || dragged.id === target.id) return

    if (dragged.source !== target.source) {
      say(
        `「${SOURCE_PLAIN[dragged.source]}」和「${SOURCE_PLAIN[target.source]}」由两套不同的机制启动，` +
          `它们之间没有统一的先后顺序可以设定。请在同一类启动方式内部调整。`,
      )
      return
    }

    const group = lane.filter((i) => i.source === dragged.source)
    const rest = group.filter((i) => i.id !== dragged.id)
    const anchor = rest.findIndex((i) => i.id === target.id)
    if (anchor < 0) return
    const insertAt = side === 'before' ? anchor : anchor + 1
    rest.splice(insertAt, 0, dragged)

    // 整组一起写：只写被拖动的那一项，其余项的位置就没变，
    // 执行时无法还原出用户想要的那份完整顺序
    setMode('orchestrate')
    stageMany(rest.map((it, idx) => ({ item: it, kind: 'order' as const, next: { order: idx } })))
    say(`已把「${displayNameOf(dragged)}」排到第 ${insertAt + 1} 位（同一类启动方式内）`)
  }

  if (items.length === 0) {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-xs text-ink-dim">没有可展示的启动项</p>
      </div>
    )
  }

  return (
    <div className="space-y-2 p-3">
      {hint && (
        <div
          className="animate-slide-up rounded border px-2.5 py-1.5 text-mini leading-4"
          style={{ borderColor: `${PLAN_COLOR}4d`, background: `${PLAN_COLOR}0f`, color: PLAN_COLOR }}
        >
          {hint}
        </div>
      )}

      {lanes.map(({ phase, list, span }) => (
        <section key={phase} className="rounded-card border border-line-subtle bg-panel/50">
          <header className="flex items-center gap-2 border-b border-line-subtle px-3 py-1.5">
            <span className="text-2xs font-semibold uppercase tracking-wide text-ink-muted">
              {PHASE_LABEL[phase]}
            </span>
            {span && (
              <span className="tnum text-2xs text-ink-dim">
                {fmtMs(span.startMs)} – {fmtMs(span.endMs)}
              </span>
            )}
            <div className="flex-1" />
            <span className="tnum text-2xs text-ink-dim">{list.length} 项</span>
          </header>

          <div className="flex flex-wrap gap-1.5 p-2">
            {list.map((it) => (
              <LaneCard
                key={it.id}
                item={it}
                staged={!!planEntries[it.id]}
                dragging={dragId === it.id}
                dropSide={dropAt?.id === it.id ? dropAt.side : null}
                onDragStart={() => setDragId(it.id)}
                onDragEnd={() => {
                  setDragId(null)
                  setDropAt(null)
                }}
                onHover={(side) => setDropAt({ id: it.id, side })}
                onLeave={() => setDropAt((cur) => (cur?.id === it.id ? null : cur))}
                onDrop={(side) => {
                  commitDrop(phase, it.id, side)
                  setDragId(null)
                  setDropAt(null)
                }}
              />
            ))}
          </div>
        </section>
      ))}
    </div>
  )
}

interface CardProps {
  item: StartupItem
  staged: boolean
  dragging: boolean
  dropSide: 'before' | 'after' | null
  onDragStart: () => void
  onDragEnd: () => void
  onHover: (side: 'before' | 'after') => void
  onLeave: () => void
  onDrop: (side: 'before' | 'after') => void
}

function LaneCard({
  item,
  staged,
  dragging,
  dropSide,
  onDragStart,
  onDragEnd,
  onHover,
  onLeave,
  onDrop,
}: CardProps) {
  const select = useAppStore((s) => s.select)
  const selectedId = useAppStore((s) => s.selectedId)

  const meta = RISK_META[item.risk]
  const kind = KIND_META[resolveKind(item)]
  const title = displayNameOf(item)
  const active = selectedId === item.id
  const badge = timingBadge(item)
  const draggable = isOrchestrable(item)

  /** 落点提示：卡片是横排的，用左右半区决定插到前面还是后面 */
  const sideOf = (e: React.DragEvent<HTMLElement>): 'before' | 'after' => {
    const r = e.currentTarget.getBoundingClientRect()
    return e.clientX < r.left + r.width / 2 ? 'before' : 'after'
  }

  return (
    <button
      type="button"
      draggable={draggable}
      onClick={() => select(item.id)}
      title={
        draggable
          ? `${title}\n${item.command}\n\n拖动可以调整它在同类启动方式中的先后顺序`
          : `${title}\n${item.command}\n\n系统组件，不参与编排`
      }
      onDragStart={(e) => {
        e.dataTransfer.effectAllowed = 'move'
        e.dataTransfer.setData('text/plain', item.id)
        onDragStart()
      }}
      onDragEnd={onDragEnd}
      onDragOver={(e) => {
        if (!draggable) return
        e.preventDefault()
        e.dataTransfer.dropEffect = 'move'
        onHover(sideOf(e))
      }}
      onDragLeave={onLeave}
      onDrop={(e) => {
        e.preventDefault()
        onDrop(sideOf(e))
      }}
      className={[
        'w-[172px] rounded border px-2 py-1.5 text-left transition-colors',
        dragging ? 'drag-source' : active ? 'bg-hover' : 'bg-elevated/60 hover:bg-hover',
        draggable ? 'cursor-grab active:cursor-grabbing' : 'cursor-pointer',
        dropSide === 'before' ? 'drag-over-before' : dropSide === 'after' ? 'drag-over-after' : '',
      ].join(' ')}
      style={{
        borderColor: staged
          ? PLAN_COLOR
          : active
            ? '#58a6ff'
            : meta.color === '#3fb950'
              ? '#30363d'
              : meta.color,
        // 实线 = 数据来自内核/事件日志（实测）；虚线 = 我们推的
        borderStyle: badge?.solid === false ? 'dashed' : 'solid',
        borderLeftWidth: 3,
      }}
    >
      <div className="flex items-center gap-1.5">
        <AppIcon name={title} iconData={item.iconData} size={14} />
        <span className="truncate text-2xs leading-4 text-ink">{title}</span>
        {staged && (
          <span className="shrink-0 text-[9px] leading-none" style={{ color: PLAN_COLOR }} title="已编排">
            ●
          </span>
        )}
      </div>

      <div className="mt-1 flex items-center gap-1">
        <span
          className="shrink-0 rounded px-1 text-2xs"
          style={{ color: kind.color, background: `${kind.color}1f` }}
        >
          {kind.short}
        </span>
        <span className="truncate text-2xs text-ink-dim">
          {shortPublisher(item.signer.publisher) ?? '未签名'}
        </span>
        <div className="flex-1" />
        {badge && (
          <Badge color={badge.color} title={badge.title}>
            {badge.text}
          </Badge>
        )}
      </div>
    </button>
  )
}
