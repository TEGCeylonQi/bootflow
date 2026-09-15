import { useMemo } from 'react'
import { useAppStore } from '@/store/useAppStore'
import { displayNameOf } from '@/lib/item'
import { IMPACT_COLOR, IMPACT_LABEL } from '@/constants'
import type { ImpactOverview, ItemImpact, StartupItem } from '@/types/model'

/**
 * 「启动影响」—— Windows **自己**为每一项量出来的资源消耗。
 *
 * ## 这一栏为什么可信度最高
 *
 * 它来自 WDI（Windows 诊断基础架构）每次登录后落的
 * `%WINDIR%\System32\WDI\LogFiles\StartupInfo\<SID>_StartupInfo<N>.xml`，
 * 里面一个 `<Process>` 节点就是一个进程，带 `CpuUsage`（微秒）与
 * `DiskUsage`（字节）—— **任务管理器「启动应用」页的「启动影响」列读的就是它**。
 *
 * 也就是说：用户能打开任务管理器，逐条对照我们给出的档位。
 * 这是整个产品里唯一一份**能被用户当场复核**的数据，
 * 而复核结果一致，正是这个软件值得信的地方。
 *
 * ## 它不是什么（界面必须说清，否则一定被误读）
 *
 * | 常见误读 | 实情 |
 * |---|---|
 * | "CPU 1.4 秒 = 它让开机慢了 1.4 秒" | CPU 时间**跨核累加**，多线程程序可以超过窗口本身长度 |
 * | "这就是整个开机期间的消耗" | 只覆盖**登录后那段窗口**，开机早期的服务/驱动不在里面 |
 * | "这项没数据 = 它不占资源" | 也可能是它没在窗口里跑，或我们没有权限读这份文件 |
 *
 * 所以下面每一处都在带口径：档位旁边标窗口，数字旁边标单位，
 * "读不到"时**单独说明原因**而不是把整块藏起来。
 */

const fmtMs = (ms: number) => (ms >= 1000 ? `${(ms / 1000).toFixed(2)}s` : `${ms}ms`)
const fmtBytes = (b: number) =>
  b >= 1_048_576
    ? `${(b / 1_048_576).toFixed(1)} MB`
    : b >= 1024
      ? `${Math.round(b / 1024)} KB`
      : `${b} B`

/** 影响排序用的分数：CPU 按毫秒算，磁盘按 KB 折算，两者相加。 */
const scoreOf = (im: ItemImpact) => im.cpuMs + im.diskBytes / 1024

/** 最多画几行。再多就该靠左侧筛选去看，而不是把卡片撑成一面墙。 */
const MAX_ROWS = 12

export function ImpactPanel({
  items,
  overview,
}: {
  items: StartupItem[]
  overview: ImpactOverview | null
}) {
  const select = useAppStore((s) => s.select)
  const selectedId = useAppStore((s) => s.selectedId)

  const rows = useMemo(
    () =>
      items
        .filter((it) => it.timing.impact)
        .sort((a, b) => scoreOf(b.timing.impact!) - scoreOf(a.timing.impact!)),
    [items],
  )

  /** 读不到时的说明。**必须分开**三种情况，它们的处置完全不同。 */
  if (!overview || overview.unavailableReason) {
    return (
      <div className="rounded-card border border-line bg-base px-3 py-2.5">
        <Header window={undefined} />
        <p className="mt-1.5 text-2xs leading-5 text-ink-muted">
          {overview?.unavailableReason ??
            '这次没能读到 Windows 自己记录的启动影响数据。'}
        </p>
        <p className="mt-1 text-2xs leading-5 text-ink-dim">
          读这份数据需要管理员权限（它的目录对普通用户是拒读的）。以管理员身份重开本程序后，
          这里会列出每一项的 CPU 时间与磁盘读写量——和任务管理器「启动影响」列是同一份数据。
        </p>
      </div>
    )
  }

  if (rows.length === 0) {
    return (
      <div className="rounded-card border border-line bg-base px-3 py-2.5">
        <Header window={overview.windowMs} />
        <p className="mt-1.5 text-2xs leading-5 text-ink-muted">
          读到了 <span className="tnum text-ink">{overview.recordCount}</span> 条进程记录，
          但一项启动项都没对上。这通常说明：这些记录对应的是登录期被拉起的普通程序，
          而不是我们列表里的启动项（开机早期启动的服务与驱动不在这份数据里）。
        </p>
      </div>
    )
  }

  const hidden = rows.length - MAX_ROWS
  const shown = rows.slice(0, MAX_ROWS)
  const maxScore = Math.max(...shown.map((r) => scoreOf(r.timing.impact!)), 1)

  return (
    <div className="rounded-card border border-line bg-base px-3 py-2.5">
      <Header window={overview.windowMs} />

      <p className="mt-1 text-2xs leading-5 text-ink-muted">
        Windows 在每次登录后量一遍每一项占了多少 CPU 与磁盘，按
        <span className="text-ink">微软公开的阈值</span>分档——和任务管理器「启动影响」列同一把尺子，
        可以逐条对照。它量的是<span className="text-ink">资源占用</span>，
        不是"让开机慢了几秒"：CPU 时间跨核累加，一项的时间可以超过窗口长度。
      </p>

      {overview.isCurrentUser === false && (
        <p className="mt-1.5 text-2xs leading-5" style={{ color: '#d29922' }}>
          ⚠ 这份记录来自<span className="text-ink">另一个账户</span>的登录会话
          （{overview.sourceSid ?? '未知'}），不代表当前用户的开机情况。
        </p>
      )}

      <div className="mt-2 space-y-[3px]">
        {shown.map((it) => {
          const im = it.timing.impact!
          const active = selectedId === it.id
          return (
            <button
              key={it.id}
              type="button"
              onClick={() => select(it.id)}
              title={
                `${displayNameOf(it)}\n` +
                `启动影响：${IMPACT_LABEL[im.level]}（与任务管理器同一阈值）\n` +
                `CPU 时间 ${fmtMs(im.cpuMs)} · 磁盘读写 ${fmtBytes(im.diskBytes)}\n` +
                (im.processCount > 1 ? `由 ${im.processCount} 个进程合计\n` : '') +
                (im.startedInTraceMs !== undefined
                  ? `登录窗口内的第 ${(im.startedInTraceMs / 1000).toFixed(1)} 秒出现\n`
                  : '') +
                '\n注意：这是资源占用，不是耗时。'
              }
              className={[
                'flex w-full items-center gap-2 rounded px-1 py-[1px] text-left transition-colors',
                active ? 'bg-hover' : 'hover:bg-hover/60',
              ].join(' ')}
            >
              <span className="w-[132px] shrink-0 truncate text-2xs text-ink-muted">
                {displayNameOf(it)}
              </span>

              <span className="relative h-3 flex-1" style={{ background: '#21262d', borderRadius: 2 }}>
                <span
                  className="absolute top-1/2 h-[7px] -translate-y-1/2 rounded-[2px]"
                  style={{
                    left: 0,
                    width: `max(3px, ${(scoreOf(im) / maxScore) * 100}%)`,
                    background: IMPACT_COLOR[im.level],
                  }}
                />
              </span>

              <span
                className="w-[34px] shrink-0 text-right text-2xs font-medium"
                style={{ color: IMPACT_COLOR[im.level] }}
              >
                {IMPACT_LABEL[im.level]}
              </span>
              <span className="w-[72px] shrink-0 text-right text-2xs tnum text-ink-dim">
                {fmtMs(im.cpuMs)}
              </span>
            </button>
          )
        })}
      </div>

      <div className="mt-1 flex items-center gap-2 text-[9px] text-ink-dim">
        <span className="w-[132px] shrink-0" />
        <span className="flex-1">条长 = CPU 时间 + 磁盘读写量（折算）</span>
        <span className="w-[34px] shrink-0 text-right">档位</span>
        <span className="w-[72px] shrink-0 text-right">CPU</span>
      </div>

      {hidden > 0 && (
        <p className="mt-1.5 text-2xs leading-5 text-ink-dim">
          另有 <span className="tnum text-ink-muted">{hidden}</span> 项也有数据，这里只画了最重的 {MAX_ROWS} 项。
          左侧清单切到「<span className="text-ink-muted">按资源占用</span>」可以看到全部。
        </p>
      )}

      {overview.sourceFile && (
        <p className="mt-1.5 text-2xs leading-5 text-ink-dim">
          数据来源：<span className="text-ink-muted">{overview.sourceFile}</span>
          —— 这个文件可以自己打开核对（在 Windows 目录下，需要管理员权限）。
        </p>
      )}
    </div>
  )
}

function Header({ window: win }: { window?: number }) {
  return (
    <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
      <span className="text-xs font-medium text-ink">每一项占了多少资源</span>
      <span className="text-2xs text-ink-dim">
        实测 · Windows 自记（与任务管理器同源）
        {win !== undefined && ` · 覆盖登录后 ${fmtMs(win)}`}
      </span>
    </div>
  )
}
