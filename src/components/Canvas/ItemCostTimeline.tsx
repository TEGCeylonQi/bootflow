import { useMemo } from 'react'
import { useAppStore } from '@/store/useAppStore'
import { displayNameOf } from '@/lib/item'
import type { ItemObservation, StartupItem } from '@/types/model'

/**
 * 「每一项在开机后第几秒出现」——把开机开销拆到单项。
 *
 * ## 这里为什么能拆、拆出来的是什么
 *
 * Windows **没有单项墙钟耗时计时器**：拉自启项的 Explorer、拉服务的 SCM、
 * 拉任务的计划任务程序，谁都不为自己的"孩子"打点。Event 100 给的是
 * 12 个**相位**耗时（`BootExplorerInitTime` 把所有自启项合成一个数字）；
 * 只有**异常到被系统判定为慢**的项才会单独留下耗时记录（Event 101/102/103）。
 *
 * ⚠️ 但"单项数据不存在"这个结论**是错的**——Windows 确实为每一项留了数据，
 * 只是不在事件日志里：逐项的 CPU 时间与磁盘 IO 在 WDI 的 `StartupInfo` 里，
 * 由 `ImpactPanel` 那块展示（任务管理器「启动影响」列读的就是它）。
 * 准确的切分是三份不同的量：
 *
 * | 量 | 覆盖 | 来源 | 界面上长什么样 |
 * |---|---|---|---|
 * | 出现时刻 | 每一项 | 内核（`GetProcessTimes`） | 本图：圆点 →「开机后 12.4 秒出现」 |
 * | 资源占用 | 每一项（需提权） | WDI `StartupInfo` | `ImpactPanel`：条 + 高/中/低 |
 * | 墙钟耗时 | **只有**被判慢的项 | Event 101/102/103 | 蓝色实心条 →「花了 3.2 秒」 |
 *
 * 于是这张图把两件事分开画：**青色圆点说"什么时候出现"**，
 * **蓝色实条说"花了多久"**。两者都有的项画成一条真实的横条。
 * 至于"它有多重"，那是另一个问题，交给下面那块资源占用图——
 * 合成一个数字就两头都说不清。
 *
 * ## 绝不做的两件事
 *
 * 1. **不给没有耗时记录的项补一个耗时。** 把相位区间长度摊给区间里的每一项，
 *    得到的数字看着精确、实际是编的，而且同相位 N 项拿到同一个值，排序都排不出来。
 * 2. **不把累计读盘当"开机阶段消耗"。** 采样拍在开机后 42 秒，那么
 *    `read_bytes` 就是"这 42 秒里累计读了多少"，不是"开机时读了多少"。
 *    所以它只在详情里出现，且必须带上采样时刻。
 */

/** 时间轴最多画几行。再多就该靠筛选去看，而不是把图拉成一堵墙。 */
const MAX_ROWS = 24

const fmt = (ms: number) => `${(ms / 1000).toFixed(1)}s`
const fmtBytes = (b: number) => (b >= 1_048_576 ? `${(b / 1_048_576).toFixed(1)} MB` : `${Math.round(b / 1024)} KB`)

const AXIS_TEXT = '#8b949e'
const GRID = '#21262d'
/** 实测耗时（事件日志）。与画布 accent 一致。 */
const C_MEASURED = '#58a6ff'
/** 实测出现时刻（内核记录的进程创建时刻）。 */
const C_OBSERVED = '#39c5bb'

/** 时间轴上限：向上取整到一个好读的刻度，并留一点右边距。 */
function axisMaxOf(values: number[]): number {
  const raw = Math.max(...values, 1)
  const step = raw > 120_000 ? 30_000 : raw > 60_000 ? 10_000 : raw > 20_000 ? 5_000 : 2_000
  return Math.ceil((raw * 1.08) / step) * step
}

interface Row {
  item: StartupItem
  start: number
  /** 有实测耗时才有长度 */
  duration?: number
}

export function ItemCostTimeline({
  items,
  observation,
}: {
  items: StartupItem[]
  observation: ItemObservation | null
}) {
  const select = useAppStore((s) => s.select)
  const selectedId = useAppStore((s) => s.selectedId)

  const view = useMemo(() => {
    const rows: Row[] = items
      .filter((it) => it.timing.observedStartMs !== undefined)
      .map((it) => ({
        item: it,
        start: it.timing.observedStartMs as number,
        duration: it.timing.durationMs,
      }))
      .sort((a, b) => a.start - b.start)

    // 估算档的项：没有出现时刻，只能按相位给一个起点。
    // 它们**不上时间轴**——同相位的项会落在同一个位置，画出来是假的整齐。
    const estimated = items.filter((it) => it.timing.confidence === 'estimated').length
    const measured = items.filter((it) => it.timing.confidence === 'measured').length

    const ends = rows.map((r) => r.start + (r.duration ?? 0))
    const captured = observation?.capturedAtOffsetMs
    const axisMax = axisMaxOf([...ends, captured ?? 0])

    return { rows: rows.slice(0, MAX_ROWS), total: rows.length, estimated, measured, axisMax }
  }, [items, observation])

  // 没采到数据时不是"没有这一项"，而是"这次没读到"。两种情况的处置完全不同，
  // 所以这里必须分别说清楚，不能都给一句"暂无数据"。
  if (!observation || observation.capturedAtOffsetMs === undefined) {
    return (
      <div className="rounded-card border border-line bg-base px-3 py-2.5">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <span className="text-xs font-medium text-ink">每项在开机后第几秒出现</span>
          <span className="text-2xs text-ink-dim">实测 · 内核记录的进程创建时刻</span>
        </div>
        <p className="mt-1.5 text-2xs leading-5 text-ink-muted">
          这次没能读到本次开机的起点时刻，所以没法把进程创建时刻换算成「开机后第几秒」。
          <span className="text-ink-dim">
            {' '}
            {observation?.unavailableReason ?? '系统日志里没有本机上次开机的记录。'}
          </span>
        </p>
        <p className="mt-1 text-2xs leading-5 text-ink-dim">
          重开一次电脑（开始菜单 → 重启，不要「关机再开」）后重新扫描，通常就有了。
        </p>
      </div>
    )
  }

  if (view.rows.length === 0) {
    return (
      <div className="rounded-card border border-line bg-base px-3 py-2.5">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <span className="text-xs font-medium text-ink">每项在开机后第几秒出现</span>
          <span className="text-2xs text-ink-dim">实测 · 内核记录的进程创建时刻</span>
        </div>
        <p className="mt-1.5 text-2xs leading-5 text-ink-muted">
          采样到了 <span className="tnum text-ink">{observation.processCount}</span> 个进程，
          但一项启动项都没对上。常见原因有两个：
        </p>
        <ul className="mt-1 space-y-0.5 pl-4 text-2xs leading-5 text-ink-dim">
          <li>
            · 多个启动项指向<span className="text-ink-muted">同一个</span>可执行文件（例如几十个服务共用{' '}
            <code className="text-ink-muted">svchost.exe</code>），那时这个进程的时刻无法代表其中任何一项，
            所以放弃归因。
          </li>
          <li>· 这些项在本次开机时没有被拉起（已停用、任务触发器没到、或目标已失效）。</li>
        </ul>
      </div>
    )
  }

  const tickCount = 4

  return (
    <div className="rounded-card border border-line bg-base px-3 py-2.5">
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
        <span className="text-xs font-medium text-ink">每项在开机后第几秒出现</span>
        <span className="text-2xs text-ink-dim">
          实测 · 内核记录的进程创建时刻 · 采样拍于开机后 {fmt(observation.capturedAtOffsetMs)}
        </span>
      </div>

      <p className="mt-1 text-2xs leading-5 text-ink-muted">
        Windows 不为单个启动项记录<span className="text-ink">「花了多久」</span>，
        所以这里画的是它<span className="text-ink">什么时候出现</span>——
        它有多重（占了多少 CPU 与磁盘）在下面那块里。
        {view.measured > 0 && (
          <>
            {' '}
            其中 <span className="tnum text-ink">{view.measured}</span> 项被系统判定过「启动偏慢」，
            才有真实的耗时——它们同时画成一条
            <span style={{ color: C_MEASURED }}>蓝色的条</span>，长度就是耗时。
          </>
        )}
      </p>

      {/* 时间刻度 */}
      <div className="mt-2 flex items-center gap-2">
        <div className="w-[132px] shrink-0" />
        <div className="relative h-4 flex-1">
          {Array.from({ length: tickCount + 1 }, (_, i) => {
            const v = (view.axisMax / tickCount) * i
            return (
              <span
                key={i}
                className="absolute text-[9px] tnum"
                style={{
                  left: `${(i / tickCount) * 100}%`,
                  color: AXIS_TEXT,
                  transform: i === tickCount ? 'translateX(-100%)' : i === 0 ? 'none' : 'translateX(-50%)',
                }}
              >
                {fmt(v)}
              </span>
            )
          })}
        </div>
        <div className="w-[68px] shrink-0" />
      </div>

      {/* 逐行 */}
      <div className="mt-0.5 space-y-[3px]">
        {view.rows.map(({ item, start, duration }) => {
          const active = selectedId === item.id
          return (
            <button
              key={item.id}
              type="button"
              onClick={() => select(item.id)}
              title={`${displayNameOf(item)}\n开机后 ${fmt(start)} 出现（内核记录的进程创建时刻，实测）${
                duration !== undefined ? `\n系统记录了它的启动耗时：${fmt(duration)}` : ''
              }\n\n注意：出现时刻不是耗时。`}
              className={[
                'flex w-full items-center gap-2 rounded px-1 py-[1px] text-left transition-colors',
                active ? 'bg-hover' : 'hover:bg-hover/60',
              ].join(' ')}
            >
              <span className="w-[132px] shrink-0 truncate text-2xs text-ink-muted">
                {displayNameOf(item)}
              </span>

              <span className="relative h-3 flex-1" style={{ background: GRID, borderRadius: 2 }}>
                {/* 只有"什么时候出现"：一个点 */}
                {duration === undefined ? (
                  <span
                    className="absolute top-1/2 h-2 w-2 -translate-y-1/2 rounded-full"
                    style={{ left: `calc(${(start / view.axisMax) * 100}% - 4px)`, background: C_OBSERVED }}
                  />
                ) : (
                  /* 出现时刻 + 实测耗时：一条真正的条 */
                  <span
                    className="absolute top-1/2 h-[7px] -translate-y-1/2 rounded-[2px]"
                    style={{
                      left: `${(start / view.axisMax) * 100}%`,
                      width: `max(3px, ${(duration / view.axisMax) * 100}%)`,
                      background: C_MEASURED,
                    }}
                  />
                )}
              </span>

              <span className="w-[68px] shrink-0 text-right text-2xs tnum text-ink-dim">
                {duration !== undefined ? `+${fmt(duration)}` : fmt(start)}
              </span>
            </button>
          )
        })}
      </div>

      {(view.total > MAX_ROWS || view.estimated > 0) && (
        <p className="mt-2 text-2xs leading-5 text-ink-dim">
          {view.total > MAX_ROWS && (
            <>
              另有 {view.total - MAX_ROWS} 项也测到了出现时刻，这里只画了最早的 {MAX_ROWS} 项。
              {view.estimated > 0 ? ' ' : ''}
            </>
          )}
          {view.estimated > 0 && (
            <>
              还有 <span className="tnum text-ink-muted">{view.estimated}</span> 项
              <span className="text-ink-muted">没有</span>出现时刻——它们要么本次没被拉起，
              要么和别的项共用一个可执行文件（比如几十个服务共用{' '}
              <code className="text-ink-muted">svchost.exe</code>，那时这个进程的时刻代表不了其中任何一项）。
              这些项只能按开机相位估算，所以没有画进来：同一个相位里的项会落在同一个位置，画出来是假的整齐。
              在「启动时序」里能按 <span className="text-ink-muted">~</span> 前缀看到它们。
            </>
          )}
        </p>
      )}

      <p className="mt-1.5 text-2xs leading-5" style={{ color: '#d29922' }}>
        ⚠ 「出现时刻」不是「花了多久」。一个程序在开机后 12 秒被拉起、然后自己慢慢初始化了
        40 秒，在 Windows 眼里都只记成「12 秒时出现」——除非它慢到被系统判定为异常，
        那才会有上面那种蓝色的耗时条。
      </p>
    </div>
  )
}

/** 供详情面板复用的读盘量格式化。 */
export { fmtBytes }
