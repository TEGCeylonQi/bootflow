import { useMemo } from 'react'
import ReactECharts from 'echarts-for-react'
import { ShieldAlert } from 'lucide-react'
import { useAppStore } from '@/store/useAppStore'
import { PHASE_COLOR, PHASE_LABEL } from '@/constants'
import { isTauri, requestElevation } from '@/api/commands'
import type { PhaseSpan } from '@/types/model'

const AXIS_TEXT = '#8b949e'
const SPLIT_LINE = '#21262d'
/** 相位之间的空隙（系统没记录到的那段）。用最低调的颜色，不参与视觉叙事。 */
const GAP_COLOR = '#21262d'

/**
 * 慢启动记录按**来源**着色。
 *
 * 系统把三条不同的记录塞进同一个概念里：服务（103）、应用（101）、驱动（102）。
 * 对用户来说这三者的应对方式完全不同——服务可以去服务管理器里查，
 * 驱动则基本不该动。用同一种黄色画出来会让人以为"都是同一种东西"。
 */
const SLOW_COLOR: Record<number, string> = {
  103: '#d29922',
  101: '#db6d28',
  102: '#8b949e',
}
const SLOW_SOURCE: Record<number, string> = { 103: '服务', 101: '应用', 102: '驱动' }

interface SlowMeta {
  title: string
  source: string
  total: number
  degradation: number
}

const fmt = (ms: number) => `${(ms / 1000).toFixed(1)}s`

/** "3 小时前"这类相对时间。只用于新鲜度提示，粗粒度就够。 */
function relTime(iso: string): string {
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return ''
  const mins = Math.max(0, Math.round((Date.now() - t) / 60000))
  if (mins < 2) return '刚刚'
  if (mins < 60) return `${mins} 分钟前`
  const hours = Math.round(mins / 60)
  if (hours < 36) return `${hours} 小时前`
  return `${Math.round(hours / 24)} 天前`
}

/**
 * 把相位序列补成一条**连续**的时间带。
 *
 * 为什么要补：相位来自事件日志的绝对锚点（`BootPNPInitStartTimeMS` 等），
 * 某些阶段在部分机型上根本不写记录，于是 `phases` 之间会留缝。如果直接按
 * "每段长度 = 该段耗时"堆叠，后面的相位会被整体前移——图上看不出问题，
 * 但它已经和 x 轴的秒数对不上了，用户在图上读到的时刻是错的。
 *
 * 补出来的那段标成「未记录」，颜色压到最低。**图可以不完整，但不能不准。**
 */
function withGaps(phases: PhaseSpan[]): PhaseSpan[] {
  const out: PhaseSpan[] = []
  let cursor = 0
  for (const p of [...phases].sort((a, b) => a.startMs - b.startMs)) {
    if (p.startMs > cursor) out.push({ name: 'unknown', startMs: cursor, endMs: p.startMs })
    out.push(p)
    cursor = Math.max(cursor, p.endMs)
  }
  return out
}

export function GanttView(_props: { items: unknown[] }) {
  const bootTimeline = useAppStore((s) => s.bootTimeline)

  const option = useMemo(() => {
    if (!bootTimeline || bootTimeline.phases.length === 0) return null

    const { phases, slowServices } = bootTimeline
    const segments = withGaps(phases)
    const total = bootTimeline.totalBootMs ?? segments[segments.length - 1].endMs
    const hasSlow = slowServices.length > 0

    // 相位堆叠段：长度 = 该相位耗时，补上缝隙后堆叠总长即为整条开机时间轴
    const phaseSeries = segments.map((p) => {
      const ms = p.endMs - p.startMs
      const gap = p.name === 'unknown'
      return {
        name: gap ? '未记录' : PHASE_LABEL[p.name],
        type: 'bar' as const,
        stack: 'boot',
        xAxisIndex: 0,
        yAxisIndex: 0,
        barWidth: 34,
        data: [ms],
        itemStyle: { color: gap ? GAP_COLOR : PHASE_COLOR[p.name] },
        label: {
          show: ms > total * 0.06,
          position: 'inside' as const,
          formatter: gap ? '?' : PHASE_LABEL[p.name],
          color: gap ? AXIS_TEXT : '#0d1117',
          fontSize: 10,
          fontWeight: 'bold' as const,
        },
      }
    })

    /** 慢启动的主指标取"多花的"那部分；系统没给时退回总耗时 */
    const degOf = (s: { durationMs: number; degradationMs: number }) =>
      s.degradationMs > 0 ? s.degradationMs : s.durationMs
    const slowMax = Math.max(1000, ...slowServices.map(degOf)) * 1.4

    const grids = hasSlow
      ? [
          { left: 108, right: 32, top: 24, height: 46 },
          { left: 108, right: 32, top: 132, bottom: 28 },
        ]
      : [{ left: 108, right: 32, top: 24, height: 46 }]

    return {
      backgroundColor: 'transparent',
      animationDuration: 600,
      tooltip: {
        trigger: 'axis',
        axisPointer: { type: 'shadow' },
        backgroundColor: '#161b22',
        borderColor: '#30363d',
        textStyle: { color: '#e6edf3', fontSize: 12 },
        formatter: (params: unknown) => {
          const list = (Array.isArray(params) ? params : [params]) as Array<{
            marker?: string
            seriesName?: string
            value?: number
            data?: { meta?: SlowMeta }
          }>
          return list
            .map((p) => {
              const meta = p.data?.meta
              if (meta) {
                return (
                  `${p.marker ?? ''}${meta.title} <span style="color:${AXIS_TEXT}">（${meta.source}）</span>` +
                  `<br/>&nbsp;&nbsp;总计 ${fmt(meta.total)}` +
                  (meta.degradation > 0 ? ` · 其中多花 ${fmt(meta.degradation)}` : '')
                )
              }
              return `${p.marker ?? ''}${p.seriesName ?? ''} ${fmt(Number(p.value ?? 0))}`
            })
            .join('<br/>')
        },
      },
      grid: grids,
      xAxis: [
        {
          type: 'value',
          gridIndex: 0,
          max: total,
          axisLine: { lineStyle: { color: SPLIT_LINE } },
          axisLabel: {
            color: AXIS_TEXT,
            fontSize: 10,
            formatter: (v: number) => `${(v / 1000).toFixed(0)}s`,
          },
          splitLine: { show: false },
        },
        ...(hasSlow
          ? [
              {
                type: 'value' as const,
                gridIndex: 1,
                max: Math.round(slowMax),
                axisLine: { lineStyle: { color: SPLIT_LINE } },
                axisLabel: {
                  color: AXIS_TEXT,
                  fontSize: 10,
                  formatter: (v: number) => `${(v / 1000).toFixed(0)}s`,
                },
                splitLine: { show: false },
              },
            ]
          : []),
      ],
      yAxis: [
        {
          type: 'category',
          gridIndex: 0,
          data: ['开机全过程'],
          axisLine: { show: false },
          axisTick: { show: false },
          axisLabel: { color: AXIS_TEXT, fontSize: 11 },
        },
        ...(hasSlow
          ? [
              {
                type: 'category' as const,
                gridIndex: 1,
                // 显示名优先，但系统大多数时候只给了服务名——那就如实显示服务名，
                // 不编一个看起来更好看但查不到的称呼
                data: slowServices.map((s) => s.friendlyName ?? s.name),
                axisLine: { show: false },
                axisTick: { show: false },
                axisLabel: {
                  color: AXIS_TEXT,
                  fontSize: 11,
                  formatter: (v: string) => (v.length > 14 ? `${v.slice(0, 13)}…` : v),
                },
              },
            ]
          : []),
      ],
      series: [
        ...phaseSeries,
        ...(hasSlow
          ? [
              {
                name: '慢启动',
                type: 'bar' as const,
                xAxisIndex: 1,
                yAxisIndex: 1,
                barWidth: 16,
                data: slowServices.map((s) => {
                  const deg = degOf(s)
                  const color = SLOW_COLOR[s.eventId] ?? '#d29922'
                  return {
                    value: deg,
                    itemStyle: { color, borderRadius: [0, 3, 3, 0] },
                    label: {
                      show: true,
                      position: 'right' as const,
                      formatter: fmt(deg),
                      color,
                      fontSize: 10,
                    },
                    meta: {
                      title: s.friendlyName ?? s.name,
                      source: `${SLOW_SOURCE[s.eventId] ?? '未知来源'} · ${s.name}`,
                      total: s.durationMs,
                      degradation: s.degradationMs,
                    } satisfies SlowMeta,
                  }
                }),
              },
            ]
          : []),
      ],
    }
  }, [bootTimeline])

  /* ── 分支一：读不到。说清楚是"没读成"，不是"没有" ── */

  if (bootTimeline && bootTimeline.unavailableReason && !option) {
    return (
      <div className="p-4">
        <div
          className="rounded-card border p-3"
          style={{ borderColor: '#d299224d', background: '#d299220f' }}
        >
          <div className="flex items-center gap-2">
            <ShieldAlert size={14} style={{ color: '#d29922' }} />
            <span className="text-xs font-medium text-ink">这次没能读到开机耗时</span>
          </div>
          <p className="mt-1.5 text-xs leading-5 text-ink-muted">{bootTimeline.unavailableReason}</p>

          <p className="mt-2 text-2xs leading-5 text-ink-dim">
            这不影响上面列的启动项——它们是从注册表、启动文件夹、计划任务和服务里直接读的，
            不需要这项权限。
            <span className="text-ink-muted">唯一缺的是「每一段各花了多久」这一层数据。</span>
          </p>

          {bootTimeline.needsElevation &&
            (isTauri() ? (
              <button
                type="button"
                onClick={() => void requestElevation()}
                className="mt-2.5 rounded-md border px-2.5 py-1 text-xs transition-colors hover:bg-hover"
                style={{ borderColor: '#d2992266', color: '#d29922' }}
              >
                以管理员身份重新打开，补齐耗时数据
              </button>
            ) : (
              <p className="mt-2 text-2xs text-ink-dim">
                打包成桌面应用后，这里会出现「以管理员身份重新打开」的入口。
              </p>
            ))}
        </div>
      </div>
    )
  }

  /* ── 分支二：权限没问题，但系统确实还没写过性能记录 ── */

  if (!option) {
    return (
      <div className="flex h-full items-center justify-center p-4">
        <p className="max-w-sm text-center text-xs leading-5 text-ink-dim">
          Windows 还没有为这台电脑记录开机性能数据。
          系统通常在开机较慢时才会写这份记录，正常速度的开机可能一直是空的——
          <span className="text-ink-muted">没有记录不代表没有耗时，只是系统觉得不值得记。</span>
        </p>
      </div>
    )
  }

  /* ── 分支三：有数据 ── */

  const startedAt = bootTimeline?.bootStartedAt
  const phases = bootTimeline?.phases ?? []
  const total = bootTimeline?.totalBootMs ?? phases[phases.length - 1]?.endMs ?? 0
  const hasSlow = (bootTimeline?.slowServices.length ?? 0) > 0

  return (
    <div className="p-3">
      <div className="mb-1 flex flex-wrap items-baseline gap-x-2 px-1">
        <span className="text-xs text-ink">
          本次开机用时 <span className="tnum font-semibold">{fmt(total)}</span>
        </span>
        {startedAt && (
          <span className="text-2xs text-ink-dim">
            · 读数来自 {relTime(startedAt)} 的那次开机
          </span>
        )}
      </div>

      <ReactECharts
        option={option}
        style={{ height: hasSlow ? 300 : 120, width: '100%' }}
        opts={{ renderer: 'svg' }}
        notMerge
      />

      <p className="mt-1 px-1 text-2xs leading-5 text-ink-dim">
        上排是开机各阶段的实际耗时（系统事件日志的实测值）。
        {hasSlow ? (
          <>
            {' '}
            下排是系统自己标记为「启动偏慢」的项，按
            <span className="text-ink-muted">多花的时间</span>
            排序——总耗时里有一部分是它正常启动本来就要用的，多花的那部分才是拖慢。
          </>
        ) : (
          <> 这次系统没有标记出启动偏慢的项。</>
        )}
        <span className="text-ink-muted"> 本工具不推算系统没记的数据，没记录就是没记录。</span>
      </p>
    </div>
  )
}
