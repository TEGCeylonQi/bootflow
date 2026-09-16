import { useMemo, useState } from 'react'
import ReactECharts from 'echarts-for-react'
import { ChevronRight, Loader2, RefreshCw, ShieldAlert } from 'lucide-react'
import { useAppStore } from '@/store/useAppStore'
import { PHASE_COLOR, PHASE_LABEL } from '@/constants'
import { readableOn } from '@/lib/color'
import {
  isTauri,
  openBootLog,
  requestElevation,
  probeBootRecord,
  enableBootRecord,
} from '@/api/commands'
import type { BootTimeline, PhaseSpan, StartupItem } from '@/types/model'
import { BootDiagInline } from '@/components/Diagnose/BootDiag'
import { BootSelfRecords } from './BootSelfRecords'
import { ItemCostTimeline } from './ItemCostTimeline'
import { ImpactPanel } from './ImpactPanel'
import { BootRecordingToggle } from './BootRecordingToggle'
import { ChartCard, LegendDot } from './ChartCard'

const AXIS_TEXT = '#8b949e'
const SPLIT_LINE = '#21262d'
/** 相位之间的空隙（系统没记录到的那段）。用最低调的颜色，不参与视觉叙事。 */
const GAP_COLOR = '#21262d'

/**
 * 慢启动记录按**来源**着色。
 *
 * 系统把三条不同的记录塞进同一个概念里：服务（103）、应用（101）、驱动（102）。
 * 对用户来说这三者的应对方式完全不同——服务可以去服务管理器里查，驱动则基本
 * 不该动。用同一种黄色画出来会让人以为"都是同一种东西"。
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

/**
 * 耗时分析。四个尺度自上而下：**各阶段 → 每项在第几秒出现 → 每项占多少资源
 * → 最近几次开机的总时长**。
 *
 * 之所以要四个而不是一个，是因为它们来自**四条互不相干的通路**，
 * 各有各的失败方式，谁都不能替谁兜底：
 *
 * | 通路 | 要权限？ | 要开关？ | 拿不到时 |
 * |---|---|---|---|
 * | 开机相位 / 慢项耗时 | 要管理员 | 否 | 只能给「读不到」+ 提权入口 |
 * | 每项的出现时刻 | **不要** | 否 | 只可能是读不到本次开机起点 |
 * | 每项的资源占用 | 要管理员 | 否 | 单独说明原因，不拖累其余三块 |
 * | 自记账总时长 | 不要 | **要用户开** | 没开就一条都没有 |
 *
 * 把四者揉成一个数字或一张图，就必然要在某处编一点东西出来。
 *
 * ── 排版契约（这一页被抱怨过"冗余杂乱"，改的时候请守住）──
 * 1. 一级界面 = 四张图 + 每图**一行**口径；三句话以上的解释进 `Methodology`。
 * 2. 三种"拿不到数据"的分支共用同一套骨架，只在顶部那张状态卡里换措辞——
 *    上一版是三套几乎一样的布局各写一遍，改一处要改三处。
 */
export function GanttView({ items }: { items: StartupItem[] }) {
  const bootTimeline = useAppStore((s) => s.bootTimeline)
  const bootRecords = useAppStore((s) => s.bootRecords)
  const observation = useAppStore((s) => s.observation)
  const impact = useAppStore((s) => s.impact)
  const refreshTimeline = useAppStore((s) => s.refreshTimeline)
  const refreshBootRecords = useAppStore((s) => s.refreshBootRecords)

  const phases = bootTimeline?.phases ?? []
  const hasPhases = phases.length > 0

  /** 两条通路一起重读：系统分段（要提权）与自记账（不用提权）。 */
  const refresh = () => {
    void refreshTimeline()
    void refreshBootRecords()
  }

  return (
    <div className="space-y-2.5 p-3">
      <StatusCard timeline={bootTimeline} onRefresh={refresh} />

      {hasPhases && <PhaseChart timeline={bootTimeline as BootTimeline} />}

      {/* 这两块与上面那张图**无关**地各自成立：进程采样不要权限，
          启动影响要权限但和事件日志是另一份文件。谁都可能单独缺席。 */}
      <ItemCostTimeline items={items} observation={observation} />
      <ImpactPanel items={items} overview={impact} />

      <BootSelfRecords records={bootRecords} />

      <BootRecordingToggle />

      <Methodology />
    </div>
  )
}

/* ───────────────────────── 顶部状态卡 ───────────────────────── */

/**
 * 本次开机能拿到什么、拿不到什么——**一张卡，一段人话，一组出口**。
 *
 * 三种情形（有分段 / 没读成 / 系统没写记录）措辞完全不同，因为处置完全不同：
 * 前者的下一步是"提权"，后者是"别等了，那台机器就是没记"。
 * 唯一相同的是：**绝不用空图或 0 秒掩盖"没读成"**。
 */
function StatusCard({
  timeline,
  onRefresh,
}: {
  timeline: BootTimeline | null
  onRefresh: () => void
}) {
  const phases = timeline?.phases ?? []
  const hasPhases = phases.length > 0

  if (hasPhases) {
    const total = timeline?.totalBootMs ?? phases[phases.length - 1].endMs
    const real = withGaps(phases).filter((p) => p.name !== 'unknown')
    const slowest = real.reduce(
      (a, b) => (b.endMs - b.startMs > a.endMs - a.startMs ? b : a),
      real[0],
    )
    const slowN = timeline?.slowServices.length ?? 0
    const slowestMs = slowest.endMs - slowest.startMs

    return (
      <section className="rounded-card border border-line bg-base px-3 py-2">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <span className="text-xs text-ink">
            本次开机用时 <span className="tnum font-semibold">{fmt(total)}</span>
          </span>
          <span className="text-2xs text-ink-dim">
            最慢的一段：{PHASE_LABEL[slowest.name]} <span className="tnum">{fmt(slowestMs)}</span>
            <span className="text-ink-faint">（占 {Math.round((slowestMs / total) * 100)}%）</span>
          </span>
          {slowN > 0 && (
            <span className="text-2xs text-ink-dim">
              系统标记拖慢项 <span className="tnum">{slowN}</span> 个
            </span>
          )}
          <div className="flex-1" />
          <RefreshButton onClick={onRefresh} />
        </div>
        {timeline?.bootStartedAt && (
          <p className="mt-0.5 text-2xs text-ink-faint">
            读数来自 {relTime(timeline.bootStartedAt)} 的那次开机 · 各阶段为实测值
          </p>
        )}
      </section>
    )
  }

  const denied = !!timeline?.unavailableReason

  return (
    <section
      className="rounded-card border px-3 py-2"
      style={
        denied
          ? { borderColor: '#d299224d', background: '#d299220f' }
          : { borderColor: '#30363d' }
      }
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        {denied && <ShieldAlert size={14} style={{ color: '#d29922' }} />}
        <span className="text-xs font-medium text-ink">
          {denied ? '这次没能读到开机耗时' : '这次没有系统的「分段耗时」'}
        </span>
        <div className="flex-1" />
        <RefreshButton onClick={onRefresh} />
      </div>

      <p className="mt-1 text-2xs leading-5 text-ink-muted">
        {denied ? (
          <>
            {timeline?.unavailableReason}
            <span className="text-ink-dim">
              {' '}
              这不影响上面列的启动项——它们是从注册表、启动文件夹、计划任务和服务里直接读的，
              不需要这项权限。唯一缺的是<span className="text-ink-muted">「每一段各花了多久」</span>这一层数据。
            </span>
          </>
        ) : (
          <>
            Windows 只在开机偏慢、且走完整引导时才写这份记录。「关机再开」（快速启动）不算完整引导，
            所以可能几个月都不写一条——<span className="text-ink">但没有分段记录，不等于没有耗时。</span>
          </>
        )}
      </p>

      <div className="mt-2 flex flex-wrap items-center gap-2">
        {denied && (
          <button
            type="button"
            onClick={() => void openBootLog()}
            className="rounded-md border px-2.5 py-1 text-xs transition-colors hover:bg-hover"
            style={{ borderColor: '#d2992266', color: '#d29922' }}
          >
            打开系统诊断日志查看
          </button>
        )}

        {denied && timeline?.needsElevation && isTauri() && (
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

        {/* 想看分段耗时，只有系统那条路可走——所以这条指引只在这两种情形下出现 */}
        <BootRecordGuide />
      </div>
    </section>
  )
}

function RefreshButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center gap-1.5 rounded-md border border-line px-2 py-1 text-2xs text-ink-muted transition-colors hover:bg-hover hover:text-ink"
      title="重新读一次开机性能日志与自记账记录"
    >
      <RefreshCw size={12} />
      重新读取
    </button>
  )
}

/**
 * 「系统记录开关」的检查入口。
 *
 * 注意这**不是**「让每次都记录」的开关——那个需求由自记账满足了。这一条回答的是
 * 另一件事：**为什么拿不到「分段」耗时**，以及系统那一侧的开关到底有没有被策略关掉。
 *
 * 诚实原则声明：**开启 ≠ 每次开机一定记录**。Windows 默认允许记录，但 Event 100
 * 往往只在开机偏慢时写入。一键开启只能保证"系统允许记录"，以及提醒用户用
 * **重启**而不是「关机再开」——快速启动会跳过完整引导，不产生记录。
 */
function BootRecordGuide() {
  const [state, setState] = useState<'idle' | 'checking' | 'done' | 'error'>('idle')
  const [msg, setMsg] = useState('')
  const [open, setOpen] = useState(false)

  const check = async () => {
    setOpen(true)
    setState('checking')
    setMsg('')
    try {
      const status = await probeBootRecord()
      if (status.state === 'allowed') {
        setMsg(
          '这个开关其实一直开着。Windows 平时只在开机偏慢时记录一次，「关机再开」（快速启动）也不算完整引导。' +
            '想要在下次开机时看到分段耗时，请用「完整重启」而不是「关机再开机」。',
        )
        setState('done')
        return
      }
      if (status.state === 'disabled') {
        try {
          await enableBootRecord()
          setMsg(
            '已帮你恢复允许记录（系统策略会在 15 分钟后自动还原）。下次「完整重启」时就会写一次开机性能记录。' +
              '注意：关机再开属于快速启动，不会触发记录。',
          )
          setState('done')
        } catch (e) {
          setMsg(e instanceof Error ? e.message : String(e))
          setState('error')
        }
        return
      }
      setMsg(status.message || '无法确认当前是否允许记录，请以管理员身份重试。')
      setState('error')
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e))
      setState('error')
    }
  }

  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-2xs text-ink-dim">想看「每一段各花多久」？那要靠系统记录</span>
        <button
          type="button"
          onClick={() => void check()}
          disabled={state === 'checking'}
          className="flex items-center gap-1 rounded-md border border-line px-2 py-1 text-2xs text-ink-muted transition-colors hover:bg-hover hover:text-ink disabled:opacity-50"
          title="查清系统那一侧到底允不允许记录"
        >
          {state === 'checking' && <Loader2 size={12} className="animate-spin" />}
          检查系统记录开关
        </button>
      </div>
      {open && msg && (
        <p
          className="mt-1.5 text-2xs leading-5 text-ink-muted"
          style={state === 'error' ? { color: '#f85149' } : undefined}
        >
          {msg}
        </p>
      )}
    </>
  )
}

/* ───────────────────────── 图一：各阶段 ───────────────────────── */

/**
 * 各阶段实测耗时（相位堆叠条）+ 系统标记的「启动偏慢」项。
 *
 * 两个 grid 共用"秒"这个单位，但**刻度不同**——上排是开机时间轴，
 * 下排是单项耗时排行。它们回答的是两个问题，所以分开画而不是拼成一张。
 */
function PhaseChart({ timeline }: { timeline: BootTimeline }) {
  const select = useAppStore((s) => s.select)

  /** 慢启动的主指标取"多花的"那部分；系统没给时退回总耗时 */
  const slowRows = useMemo(() => {
    const degOf = (s: { durationMs: number; degradationMs: number }) =>
      s.degradationMs > 0 ? s.degradationMs : s.durationMs
    return [...timeline.slowServices].sort((a, b) => degOf(b) - degOf(a))
  }, [timeline])

  const chart = useMemo(() => {
    const { phases } = timeline
    const segments = withGaps(phases)
    const total = timeline.totalBootMs ?? segments[segments.length - 1].endMs
    const hasSlow = slowRows.length > 0

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
        barWidth: 30,
        data: [ms],
        itemStyle: { color: gap ? GAP_COLOR : PHASE_COLOR[p.name] },
        label: {
          show: ms > total * 0.06,
          position: 'inside' as const,
          formatter: gap ? '?' : PHASE_LABEL[p.name],
          // 相位配色横跨深蓝到橙，单一的深色文字在深蓝那几段上根本读不出来
          color: gap ? AXIS_TEXT : readableOn(PHASE_COLOR[p.name]),
          fontSize: 10,
          fontWeight: 'bold' as const,
        },
      }
    })

    const degOf = (s: { durationMs: number; degradationMs: number }) =>
      s.degradationMs > 0 ? s.degradationMs : s.durationMs
    const slowMax = Math.max(1000, ...slowRows.map(degOf)) * 1.4
    // 类目轴索引 0 在最下方：反转后自上而下即为"最拖慢的在上"
    const display = [...slowRows].reverse()

    const grids = hasSlow
      ? [
          { left: 96, right: 28, top: 4, height: 30 },
          { left: 96, right: 28, top: 74, height: slowRows.length * 20 + 8 },
        ]
      : [{ left: 96, right: 28, top: 4, height: 30 }]

    const axisFmt = (v: number) => `${(v / 1000).toFixed(0)}s`
    const axisCommon = {
      type: 'value' as const,
      axisLine: { lineStyle: { color: SPLIT_LINE } },
      axisLabel: { color: AXIS_TEXT, fontSize: 10, formatter: axisFmt },
      splitLine: { show: false },
    }

    return {
      height: hasSlow ? 74 + slowRows.length * 20 + 8 + 28 : 74,
      display,
      option: {
        backgroundColor: 'transparent',
        animationDuration: 500,
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
          { ...axisCommon, gridIndex: 0, max: total },
          ...(hasSlow ? [{ ...axisCommon, gridIndex: 1, max: Math.round(slowMax) }] : []),
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
                  data: display.map((s) => s.friendlyName ?? s.name),
                  axisLine: { show: false },
                  axisTick: { show: false },
                  axisLabel: {
                    color: AXIS_TEXT,
                    fontSize: 10,
                    width: 84,
                    overflow: 'truncate' as const,
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
                  barWidth: 12,
                  data: display.map((s) => {
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
      },
    }
  }, [timeline, slowRows])

  const slow = timeline.slowServices

  /** 点慢项 → 选中清单里对应的那一项（对不上就不动，不猜） */
  const pickSlow = (dataIndex: number) => {
    const s = chart.display[dataIndex]
    if (!s) return
    const lower = s.name.toLowerCase()
    const hit = useAppStore
      .getState()
      .items.find(
        (it) =>
          it.name.toLowerCase() === lower ||
          it.resolvedPath.toLowerCase().endsWith(`\\${lower}`) ||
          it.displayName?.toLowerCase() === lower,
      )
    if (hit) select(hit.id)
  }

  return (
    <ChartCard
      title="开机各阶段"
      sub="实测 · 系统事件日志"
      legend={
        <>
          {slow.some((s) => s.eventId === 103) && <LegendDot color={SLOW_COLOR[103]} text="拖慢的服务" />}
          {slow.some((s) => s.eventId === 101) && <LegendDot color={SLOW_COLOR[101]} text="拖慢的应用" />}
          {slow.some((s) => s.eventId === 102) && <LegendDot color={SLOW_COLOR[102]} text="拖慢的驱动" />}
        </>
      }
    >
      <div data-chart="boot-phases">
        <ReactECharts
          option={chart.option}
          style={{ height: chart.height, width: '100%' }}
          opts={{ renderer: 'svg' }}
          notMerge
          onEvents={{
            click: (params: { seriesName?: string; dataIndex?: number }) => {
              if (params?.seriesName === '慢启动') pickSlow(params.dataIndex ?? -1)
            },
          }}
        />
      </div>

      <p className="text-2xs leading-5 text-ink-muted">
        上排是各阶段的实际耗时。
        {slow.length > 0 ? (
          <>
            {' '}
            下排是系统判定「启动偏慢」的项，按<span className="text-ink">多花的时间</span>排序
            ——总耗时里有一部分是它正常启动本来就要用的，多花的那部分才是拖慢。
          </>
        ) : (
          <> 这次系统没有标记出启动偏慢的项。</>
        )}
        <span className="text-ink-dim"> 系统没记的数据本工具不推算，没记录就是没记录。</span>
      </p>
    </ChartCard>
  )
}

/* ───────────────────────── 口径说明（折叠） ───────────────────────── */

/**
 * 二级折叠区：所有"三句话以上"的解释都放这里。
 *
 * 一级界面（四张图）只讲结论；这里讲**为什么只能这样给数据**。
 * 放进来的判据：一条解释如果拿掉之后用户仍能做出判断，它就该在这里，
 * 而不是挤在图上把图表淹掉。
 */
function Methodology() {
  const [open, setOpen] = useState(false)

  return (
    <section className="rounded-card border border-line bg-base">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-1.5 px-3 py-2 text-left transition-colors hover:bg-hover/40"
      >
        <ChevronRight
          size={12}
          className={['shrink-0 text-ink-dim transition-transform', open ? 'rotate-90' : ''].join(' ')}
        />
        <span className="text-mini font-medium tracking-wide text-ink-muted">数据口径与来源</span>
        <span className="flex-1" />
        {!open && <span className="text-2xs text-ink-dim">四条通路 · 各自可能读不到什么</span>}
      </button>

      {open && (
        <div className="space-y-2 px-3 pb-3 text-2xs leading-5 text-ink-muted">
          <p>
            本页四个量来自<span className="text-ink">四条互不相干的通路</span>
            ，各自可能读不到，谁都不能替谁兜底：
          </p>
          <ul className="space-y-0.5 pl-3 text-ink-dim">
            <li>
              · <span className="text-ink-muted">各阶段耗时</span> —— 系统事件日志（Event 100）。要管理员权限，
              且系统只在开机偏慢、走完整引导时才写。
            </li>
            <li>
              · <span className="text-ink-muted">每项的出现时刻</span> —— 内核记录的进程创建时刻。不要权限，
              但要能确定本次开机的起点。
            </li>
            <li>
              · <span className="text-ink-muted">每项的资源占用</span> —— Windows 诊断基础架构留下的
              <code className="text-ink-muted">System32\WDI\LogFiles\StartupInfo\*.xml</code>
              。要管理员权限，只覆盖登录后约 90 秒。
            </li>
            <li>
              · <span className="text-ink-muted">开机总时长</span> —— BootFlow 自记账（可选开关）。只有总长，
              没有分段。
            </li>
          </ul>
          <p>
            <span className="text-ink-muted">为什么给不出「每一项花了多少秒」</span>：Windows 不为自启动项
            记录墙钟耗时——拉自启项的 Explorer、拉服务的 SCM 都不为自己的"孩子"打点。只有异常到被系统
            判定为慢的项，才会留下一条单独的耗时记录（Event 101/102/103）。
          </p>
          <p>
            <span className="text-ink-muted">CPU 时间与磁盘量是资源占用，不是耗时</span>：CPU 时间跨核累加，
            多线程程序的时间可以超过窗口本身长度，所以它从不被当成"让开机慢了几秒"来用。
          </p>
          <p>
            <span className="text-ink-muted">「未读取」不等于 0</span>：读不到时本页不画空图、不显示 0 秒，
            也从不把估算标成实测。橙色文字一律表示"没读成"，而不是"没有"。
          </p>
          <p>
            <span className="text-ink-muted">哪些项不进时间轴</span>：只有相位估算的项（同一相位里的项会拿到
            同一个时刻，画出来是假的整齐，只在「启动时序」里以 ~ 前缀出现）；以及与别的项共用一个可执行
            文件的项（几十个服务共用 <code className="text-ink-muted">svchost.exe</code>{' '}
            时，那个进程的时刻代表不了其中任何一项）。
          </p>
        </div>
      )}
    </section>
  )
}
