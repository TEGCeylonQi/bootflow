import { useMemo } from 'react'
import ReactECharts from 'echarts-for-react'
import { useAppStore } from '@/store/useAppStore'
import { displayNameOf } from '@/lib/item'
import { IMPACT_COLOR, IMPACT_LABEL } from '@/constants'
import type { ImpactOverview, ItemImpact, StartupItem } from '@/types/model'
import { ChartCard, LegendDot } from './ChartCard'

/**
 * 「每一项占了多少资源」—— Windows **自己**为每一项量出来的资源消耗。
 *
 * ## 这一栏为什么可信度最高
 *
 * 它来自 WDI 每次登录后落的
 * `%WINDIR%\System32\WDI\LogFiles\StartupInfo\<SID>_StartupInfo<N>.xml`，
 * 一个 `<Process>` 节点就是一个进程，带 `CpuUsage`（微秒）与 `DiskUsage`（字节）
 * —— **任务管理器「启动应用」页的「启动影响」列读的就是它**。
 *
 * 也就是说：用户能打开任务管理器，逐条对照我们给出的档位。这是整个产品里
 * 唯一一份**能被用户当场复核**的数据，而复核结果一致，正是这个软件值得信的地方。
 *
 * ## 条长画的是什么（上一版画错了）
 *
 * 上一版把 `CPU + 磁盘折算` 揉成一个无量纲分数当条长——两个不同单位的量相加，
 * 用户没法从图上读出任何一个真实数字。现在**条长就是 CPU 时间**（毫秒，
 * 有真实刻度），档位色仍然是 Windows 的判定，磁盘读写量在悬浮里给。
 *
 * ## 它不是什么（界面必须说清，否则一定被误读）
 *
 * | 常见误读 | 实情 |
 * |---|---|
 * | "CPU 1.4 秒 = 它让开机慢了 1.4 秒" | CPU 时间**跨核累加**，多线程程序可以超过窗口本身长度 |
 * | "这就是整个开机期间的消耗" | 只覆盖**登录后那段窗口**，开机早期的服务/驱动不在里面 |
 * | "这项没数据 = 它不占资源" | 也可能是它没在窗口里跑，或我们没有权限读这份文件 |
 */

const fmtMs = (ms: number) => (ms >= 1000 ? `${(ms / 1000).toFixed(2)}s` : `${ms}ms`)
/** 刻度上的数字要短：2.00s 那种两位小数会把轴挤满噪音。 */
const fmtAxis = (v: number) => (v >= 1000 ? `${(v / 1000).toFixed(1)}s` : `${v}ms`)
const fmtBytes = (b: number) =>
  b >= 1_048_576
    ? `${(b / 1_048_576).toFixed(1)} MB`
    : b >= 1024
      ? `${Math.round(b / 1024)} KB`
      : `${b} B`

/** 排序用的分数：CPU 按毫秒算，磁盘按 KB 折算，两者相加。只用于**排序**，不上图。 */
const scoreOf = (im: ItemImpact) => im.cpuMs + im.diskBytes / 1024

/** 最多画几行。再多就该靠左侧筛选去看，而不是把卡片撑成一面墙。 */
const MAX_ROWS = 10
const ROW_H = 22
const NAME_W = 104

const AXIS_TEXT = '#8b949e'
const SPLIT_LINE = '#21262d'

/** 轴上限：取整到一个好读的刻度，留出右侧档位标签的位置。 */
function axisMaxOf(values: number[]): number {
  const raw = Math.max(...values, 1)
  const step = raw > 4000 ? 1000 : raw > 1000 ? 500 : raw > 300 ? 100 : raw > 60 ? 50 : 20
  return Math.ceil((raw * 1.12) / step) * step
}

export function ImpactPanel({
  items,
  overview,
}: {
  items: StartupItem[]
  overview: ImpactOverview | null
}) {
  const select = useAppStore((s) => s.select)

  const rows = useMemo(
    () =>
      items
        .filter((it) => it.timing.impact)
        .sort((a, b) => scoreOf(b.timing.impact!) - scoreOf(a.timing.impact!)),
    [items],
  )

  /* ── 读不到：**必须**说清是"没读成"而不是"没有" ── */

  if (!overview || overview.unavailableReason) {
    return (
      <ChartCard title="每一项占了多少资源" sub="实测 · Windows 自记（与任务管理器同源）">
        <p className="text-2xs leading-5 text-ink-muted">
          {overview?.unavailableReason ?? '这次没能读到 Windows 自己记录的启动影响数据。'}
        </p>
        <p className="mt-1 text-2xs leading-5 text-ink-dim">
          读取这份数据需要管理员权限（它的目录对普通用户是拒读的）。以管理员身份重开本程序后，
          这里会列出每一项的 CPU 时间与磁盘读写量——和任务管理器「启动影响」列是同一份数据。
        </p>
      </ChartCard>
    )
  }

  if (rows.length === 0) {
    return (
      <ChartCard
        title="每一项占了多少资源"
        sub={`实测 · Windows 自记（与任务管理器同源）${
          overview.windowMs !== undefined ? ` · 覆盖登录后 ${fmtMs(overview.windowMs)}` : ''
        }`}
      >
        <p className="text-2xs leading-5 text-ink-muted">
          读到 <span className="tnum text-ink">{overview.recordCount}</span> 条进程记录，但一项启动项都没对上
          ——这些记录多半是登录期被拉起的普通程序（开机早期启动的服务与驱动不在这份数据里）。
        </p>
      </ChartCard>
    )
  }

  const shown = rows.slice(0, MAX_ROWS).reverse()
  const hidden = rows.length - MAX_ROWS
  const axisMax = axisMaxOf(shown.map((r) => r.timing.impact!.cpuMs))

  const option = {
    backgroundColor: 'transparent',
    animationDuration: 500,
    grid: { left: 0, right: 26, top: 4, bottom: 0, containLabel: true },
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'shadow' },
      backgroundColor: '#161b22',
      borderColor: '#30363d',
      textStyle: { color: '#e6edf3', fontSize: 12 },
      formatter: (params: unknown) => {
        const list = (Array.isArray(params) ? params : [params]) as Array<{ dataIndex?: number }>
        const it = shown[list[0]?.dataIndex ?? 0]
        const im = it?.timing.impact
        if (!it || !im) return ''
        return [
          displayNameOf(it),
          `<span style="color:${IMPACT_COLOR[im.level]}">启动影响：${IMPACT_LABEL[im.level]}</span>` +
            `<span style="color:${AXIS_TEXT}">（与任务管理器同一阈值）</span>`,
          `CPU 时间 <b>${fmtMs(im.cpuMs)}</b>` + `<span style="color:${AXIS_TEXT}"> · 跨核累加，不是墙钟耗时</span>`,
          `磁盘读写 <b>${fmtBytes(im.diskBytes)}</b>`,
          im.processCount > 1 ? `<span style="color:${AXIS_TEXT}">由 ${im.processCount} 个进程合计</span>` : '',
          im.startedInTraceMs !== undefined
            ? `<span style="color:${AXIS_TEXT}">登录窗口内第 ${(im.startedInTraceMs / 1000).toFixed(1)} 秒出现</span>`
            : '',
        ]
          .filter(Boolean)
          .join('<br/>')
      },
    },
    xAxis: {
      type: 'value',
      min: 0,
      max: axisMax,
      axisLine: { lineStyle: { color: SPLIT_LINE } },
      axisTick: { show: false },
      axisLabel: { color: AXIS_TEXT, fontSize: 10, formatter: (v: number) => fmtAxis(v) },
      splitLine: { lineStyle: { color: SPLIT_LINE } },
    },
    yAxis: {
      type: 'category',
      data: shown.map((it) => displayNameOf(it)),
      axisLine: { show: false },
      axisTick: { show: false },
      axisLabel: { color: AXIS_TEXT, fontSize: 10, width: NAME_W, overflow: 'truncate' },
    },
    series: [
      {
        name: '启动影响',
        type: 'bar' as const,
        barWidth: 10,
        itemStyle: { borderRadius: [0, 2, 2, 0] },
        label: {
          show: true,
          position: 'right' as const,
          formatter: (p: { dataIndex?: number }) =>
            IMPACT_LABEL[shown[p.dataIndex ?? 0]?.timing.impact?.level ?? 'low'],
          fontSize: 10,
          color: AXIS_TEXT,
        },
        data: shown.map((it) => ({
          value: it.timing.impact!.cpuMs,
          itemStyle: { color: IMPACT_COLOR[it.timing.impact!.level] },
        })),
      },
    ],
  }

  return (
    <ChartCard
      title="每一项占了多少资源"
      sub={`实测 · Windows 自记（与任务管理器同源）${
        overview.windowMs !== undefined ? ` · 覆盖登录后 ${fmtMs(overview.windowMs)}` : ''
      }`}
      legend={
        <>
          {(['high', 'medium', 'low'] as const).map((lv) => (
            <LegendDot key={lv} color={IMPACT_COLOR[lv]} text={IMPACT_LABEL[lv]} />
          ))}
        </>
      }
    >
      <div data-chart="item-impact">
        <ReactECharts
          option={option}
          style={{ height: shown.length * ROW_H + 30, width: '100%', marginTop: 2 }}
          opts={{ renderer: 'svg' }}
          notMerge
          onEvents={{
            click: (params: { dataIndex?: number }) => {
              const it = shown[params?.dataIndex ?? -1]
              if (it) select(it.id)
            },
          }}
        />
      </div>

      <p className="text-2xs leading-5 text-ink-muted">
        条长 = <span className="text-ink">CPU 时间</span>（磁盘读写量在悬浮里），颜色是 Windows 按
        <span className="text-ink">微软公开阈值</span>给的档位——同一把尺子，可以打开任务管理器逐条对照。
        它量的是<span className="text-ink">资源占用</span>，不是「让开机慢了几秒」：CPU 时间跨核累加，
        一项的时间可以超过窗口长度。
      </p>

      {overview.isCurrentUser === false && (
        <p className="mt-1 text-2xs leading-5" style={{ color: '#d29922' }}>
          ⚠ 这份记录来自<span className="text-ink">另一个账户</span>的登录会话
          （{overview.sourceSid ?? '未知'}），不代表当前用户的开机情况。
        </p>
      )}

      {hidden > 0 && (
        <p className="mt-1 text-2xs leading-5 text-ink-dim">
          另有 <span className="tnum text-ink-muted">{hidden}</span> 项也有数据，这里只画最重的 {MAX_ROWS} 项
          ——左侧清单切到「<span className="text-ink-muted">按资源占用</span>」可以看到全部。
        </p>
      )}
    </ChartCard>
  )
}
