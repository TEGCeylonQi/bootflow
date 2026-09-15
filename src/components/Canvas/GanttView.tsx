import { useMemo, useState } from 'react'
import ReactECharts from 'echarts-for-react'
import { Loader2, RefreshCw, ShieldAlert } from 'lucide-react'
import { useAppStore } from '@/store/useAppStore'
import { PHASE_COLOR, PHASE_LABEL } from '@/constants'
import {
  isTauri,
  openBootLog,
  requestElevation,
  probeBootRecord,
  enableBootRecord,
} from '@/api/commands'
import type { PhaseSpan, StartupItem } from '@/types/model'
import { BootDiagInline } from '@/components/Diagnose/BootDiag'
import { BootSelfRecords } from './BootSelfRecords'
import { ItemCostTimeline } from './ItemCostTimeline'
import { ImpactPanel } from './ImpactPanel'
import { BootRecordingToggle } from './BootRecordingToggle'

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
 * 「系统记录开关」的检查卡片。
 *
 * 注意这**不是**「让每次都记录」的开关——那个需求由自记账（BootSelfRecords）
 * 满足了，且默认开启。这张卡回答的是另一件事：**为什么拿不到「分段」耗时**，
 * 以及系统那一侧的开关到底有没有被策略关掉。
 *
 * 诚实原则声明：**开启 ≠ 每次开机一定记录**。Windows 默认允许记录，
 * 但 Event 100 往往只在开机偏慢时写入。一键开启只能保证"系统允许记录"
 * （如果策略显式禁用了的话），以及提醒用户用**重启**而不是「关机再开」——
 * 快速启动（Fast Startup）会跳过完整引导，不产生记录。
 */
function BootRecordGuide() {
  const [state, setState] = useState<'idle' | 'checking' | 'done' | 'error'>('idle')
  const [msg, setMsg] = useState('')

  const open = async () => {
    setState('checking')
    setMsg('')
    try {
      const status = await probeBootRecord()
      if (status.state === 'allowed') {
        // 系统已允许记录：解释为什么还是没数据，并指引用重启触发一次
        setMsg(
          '这个开关其实一直开着。Windows 平时只在开机偏慢时记录一次，「关机再开」（快速启动）也不算完整引导。' +
            '想要在下次开机时看到耗时数据，请用「完整重启」而不是「关机再开机」。',
        )
        setState('done')
        return
      }
      if (status.state === 'disabled') {
        // 系统被策略禁了：试着帮他打开
        try {
          await enableBootRecord()
          setMsg(
            '已帮你恢复允许记录（系统策略会在 15 分钟后自动还原）。' +
              '下次「完整重启」时就会写一次开机性能记录。注意：关机再开属于快速启动，不会触发记录。',
          )
          setState('done')
        } catch (e) {
          setMsg(e instanceof Error ? e.message : String(e))
          setState('error')
        }
        return
      }
      // unreadable：无权限等原因
      setMsg(status.message || '无法确认当前是否允许记录，请以管理员身份重试。')
      setState('error')
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e))
      setState('error')
    }
  }

  const isBusy = state === 'checking'

  return (
    <div
      className="rounded-card border border-line bg-base px-3 py-2.5"
      style={{ marginTop: 8 }}
    >
      <div className="flex items-center gap-2">
        <span className="text-2xs text-ink-dim">想看「每一段各花多久」？那要靠系统记录</span>
        <button
          type="button"
          onClick={open}
          disabled={isBusy}
          className="flex items-center gap-1.5 rounded-md border border-line px-2 py-1 text-2xs text-ink transition-colors hover:text-accent disabled:opacity-50"
        >
          {isBusy ? (
            <>
              <Loader2 size={12} className="animate-spin" />
              检查中…
            </>
          ) : (
            '检查系统记录开关'
          )}
        </button>
      </div>
      {msg && <p className="mt-2 text-2xs leading-5 text-ink-muted">{msg}</p>}
    </div>
  )
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

/**
 * 耗时分析。三个尺度自上而下：**各段耗时 → 每项在第几秒出现 → 最近几次开机的总时长**。
 *
 * 之所以要三个而不是一个，是因为它们来自**三条互不相干的通路**，
 * 各有各的失败方式，谁都不能替谁兜底：
 *
 * | 通路 | 要权限？ | 要开关？ | 拿不到时 |
 * |---|---|---|---|
 * | 开机相位 / 慢项耗时 | 要管理员 | 否 | 只能给「读不到」+ 提权入口 |
 * | 每项的出现时刻 | **不要** | 否 | 只可能是读不到本次开机起点 |
 * | 自记账总时长 | 不要 | **要用户开** | 没开就一条都没有 |
 *
 * 把三者揉成一个数字或一张图，就必然要在某处编一点东西出来。
 */
export function GanttView({ items }: { items: StartupItem[] }) {
  const bootTimeline = useAppStore((s) => s.bootTimeline)
  const bootRecords = useAppStore((s) => s.bootRecords)
  const observation = useAppStore((s) => s.observation)
  const impact = useAppStore((s) => s.impact)
  const refreshTimeline = useAppStore((s) => s.refreshTimeline)
  const refreshBootRecords = useAppStore((s) => s.refreshBootRecords)

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

          <div className="mt-2.5 flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={() => void openBootLog()}
              className="rounded-md border px-2.5 py-1 text-xs transition-colors hover:bg-hover"
              style={{ borderColor: '#d2992266', color: '#d29922' }}
            >
              打开系统诊断日志查看
            </button>

            {bootTimeline.needsElevation && isTauri() && (
              <button
                type="button"
                onClick={() => void requestElevation()}
                className="rounded-md border border-line px-2.5 py-1 text-xs text-ink-muted transition-colors hover:text-ink"
                title="以管理员身份重新打开，程序启动时会自动读取耗时数据"
              >
                以管理员权限重开
              </button>
            )}

            <BootDiagInline />

            <button
              type="button"
              onClick={() => {
                // 两条通路一起重读：系统分段（要提权）与自记账（不用提权）。
                // 分开点两次在用户看来就是"点了没反应"，所以这里一并刷新。
                void refreshTimeline()
                void refreshBootRecords()
              }}
              className="flex items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-xs text-ink-muted transition-colors hover:bg-hover hover:text-ink"
              title="重新读一次开机性能日志"
            >
              <RefreshCw size={12} />
              重新读取
            </button>
          </div>
        </div>

        {/* 三条通路互相独立：系统分段（要提权）、自记账（要开关）、
            进程采样（都不要）。所以下面两块即便在这个"读不到"分支里也照常有数据。 */}
        <div style={{ marginTop: 8 }}>
          <ItemCostTimeline items={items} observation={observation} />
          <ImpactPanel items={items} overview={impact} />
        </div>

        <div style={{ marginTop: 8 }}>
          <BootSelfRecords records={bootRecords} />
        </div>

        <div style={{ marginTop: 8 }}>
          <BootRecordingToggle />
        </div>

        {/* 引导用户开启「每次开机都记录」 */}
        <BootRecordGuide />
      </div>
    )
  }

  /* ── 分支二：权限没问题，但系统确实还没写过性能记录 ── */

  if (!option) {
    const hasSelf = bootRecords.length > 0

    return (
      <div className="p-4">
        <div className="rounded-card border border-line bg-base px-3 py-2.5">
          <div className="flex items-center gap-2">
            <span className="text-xs font-medium text-ink">这次没有系统的「分段耗时」</span>
          </div>
          <p className="mt-1 text-xs leading-5 text-ink-muted">
            Windows 只在开机偏慢、且走完整引导时才写这份记录。「关机再开」（快速启动）
            不算完整引导，所以可能几个月都不写一条——
            <span className="text-ink">但没有分段记录，不等于没有耗时。</span>
          </p>

          <div className="mt-2.5 flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={() => {
                void refreshTimeline()
                void refreshBootRecords()
              }}
              className="flex items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-xs text-ink-muted transition-colors hover:bg-hover hover:text-ink"
              title="重新读一次开机性能日志"
            >
              <RefreshCw size={12} />
              重新读取
            </button>
            <BootDiagInline />
          </div>
        </div>

        {/* 进程采样这条通路：不需提权，所以这里**一定有**单项的出现时刻。
            系统没写分段记录，不代表我们说不出"谁在第几秒被拉起"。 */}
        <div style={{ marginTop: 8 }}>
          <ItemCostTimeline items={items} observation={observation} />
          <ImpactPanel items={items} overview={impact} />
        </div>

        {/* 自记账这条通路：不需提权、快速启动下也照样记，所以这里通常有数据 */}
        <div style={{ marginTop: 8 }}>
          <BootSelfRecords records={bootRecords} />
        </div>

        <div style={{ marginTop: 8 }}>
          <BootRecordingToggle />
        </div>

        {/* 一条记录都还没有。这里刻意**不**断言"已经登记为自启了"——
            自记账是可选功能、默认关闭，所以"没有记录"完全可能只是因为还没打开它。 */}
        {!hasSelf && (
          <p className="mt-2 px-1 text-2xs leading-5 text-ink-dim">
            还没有自记账记录。打开下面的
            <span className="text-ink-muted">「每次开机记一条用时」</span>
            （可选功能，默认关闭），它会在
            <span className="text-ink">下次开机</span>时写下第一条。
            <span className="text-ink-muted">
              {' '}
              若已经打开、开机几次后仍然没有，那说明登记没成功——把这条告诉我们即可。
            </span>
          </p>
        )}

        <BootRecordGuide />
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

      {/*
        下方是历史趋势：上面的图讲"这一次的各段耗时"，这里讲"最近几次的总时长"。
        只有一条时不画——一次数据没有趋势可言，只是重复上面已经说过的话。
      */}
      {/* 从上往下是三个尺度：各段耗时（这里）→ 每一项在第几秒出现（下面）
          → 最近几次开机的总时长（最下面）。尺度由大到小，读起来是一路下钻。 */}
      <div style={{ marginTop: 10 }}>
        <ItemCostTimeline items={items} observation={observation} />
        <ImpactPanel items={items} overview={impact} />
      </div>

      {bootRecords.length >= 2 && (
        <div style={{ marginTop: 10 }}>
          <BootSelfRecords records={bootRecords} />
        </div>
      )}

      <div style={{ marginTop: 10 }}>
        <BootRecordingToggle />
      </div>
    </div>
  )
}
