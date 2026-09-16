import { useMemo } from 'react'
import ReactECharts from 'echarts-for-react'
import type { BootRecord } from '@/types/model'
import { ChartCard } from './ChartCard'

/**
 * 「最近几次开机用时」—— BootFlow 自己记的总时长。
 *
 * ## 它回答什么、不回答什么
 *
 * 回答：「我最近几次开机各用了多久」。不回答：「每一段各花了多久 / 哪个启动项
 * 拖慢的」——那是系统事件日志（Event 100/103）才有的数据，自记账拿不到，
 * 所以这里**只有总时长，没有分段**。把总时长画成一根柱是如实的；
 * 编几段"大概是这样"堆上去就是把估算说成实测。
 *
 * ## 为什么这条通路重要
 *
 * 系统那条通路在开了快速启动的机器上可以几个月一条记录都没有（实测某台
 * 403 天零记录）。自记账每次登录静默跑一次，普通权限、不依赖任何系统策略，
 * 所以这里**总能画出点东西**——前提是用户装好之后至少开机过一次。
 */

const AXIS_TEXT = '#8b949e'
const SPLIT_LINE = '#21262d'
/** 实测口径的柱色。与画布 accent 一致，避免多引一套色板。 */
const BAR_COLOR = '#58a6ff'
/**
 * 推算口径的柱色（灰）。
 *
 * 「实测 / 估算要用视觉区分」是本项目的硬规则——同一种蓝会让人以为每根柱子
 * 都是同样可信的读数，而灰色的那几根在快速启动的机器上可能是跨了好几次
 * 开关机的累计运行时长。宁可难看，不可误导。
 */
const BAR_ESTIMATED = '#6e7681'

/** 图上最多画几根柱。再多 x 轴标签就挤成一团，趋势也看不出来了。 */
const MAX_CHART_BARS = 12

const fmt = (ms: number) => `${(ms / 1000).toFixed(1)}s`

/** 「3 小时前」这类相对时间。 */
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

/** x 轴刻度的短标签：`8/28 09:12`。不写年份——历史最多 50 条，年份没有信息量。 */
function axisLabelOf(iso: string): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return '—'
  const hh = String(d.getHours()).padStart(2, '0')
  const mm = String(d.getMinutes()).padStart(2, '0')
  return `${d.getMonth() + 1}/${d.getDate()} ${hh}:${mm}`
}

function tooltipLabelOf(iso: string): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return '时间未知'
  return d.toLocaleString('zh-CN', {
    month: 'numeric',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}

export function BootSelfRecords({ records }: { records: BootRecord[] }) {
  const view = useMemo(() => {
    if (records.length === 0) return null
    // records 是旧 → 新。取最近 N 条，并保持时间顺序。
    const recent = records.slice(-MAX_CHART_BARS)
    const values = recent.map((r) => Math.round(r.totalMs / 100) / 10) // 秒，保留 1 位
    const labels = recent.map((r) => axisLabelOf(r.bootStartedAt))
    const slowest = recent.reduce((a, b) => (b.totalMs > a.totalMs ? b : a))
    const latest = recent[recent.length - 1]
    const avg = recent.reduce((s, r) => s + r.totalMs, 0) / recent.length
    // 「依据」不是 log 的即视为推算口径（含旧版本没有该字段的记录）
    const estimatedCount = recent.filter((r) => r.basis !== 'log').length
    return { recent, values, labels, slowest, latest, avg, estimatedCount }
  }, [records])

  if (!view) return null

  const { recent, values, labels, slowest, latest, avg, estimatedCount } = view

  const option = {
    backgroundColor: 'transparent',
    animationDuration: 500,
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'shadow' },
      backgroundColor: '#161b22',
      borderColor: '#30363d',
      textStyle: { color: '#e6edf3', fontSize: 12 },
      formatter: (params: unknown) => {
        const list = (Array.isArray(params) ? params : [params]) as Array<{ dataIndex?: number }>
        const r = recent[list[0]?.dataIndex ?? 0]
        if (!r) return ''
        return (
          `开机于 ${tooltipLabelOf(r.bootStartedAt)}` +
          `<br/>用时 <b>${fmt(r.totalMs)}</b>` +
          `<br/><span style="color:${AXIS_TEXT}">自记账 · ${
            r.basis !== 'log' ? '按运行时长推算（估算）' : '系统日志确认（实测）'
          }</span>`
        )
      },
    },
    grid: { left: 0, right: 12, top: 22, bottom: 0, containLabel: true },
    xAxis: {
      type: 'category',
      data: labels,
      axisLine: { lineStyle: { color: SPLIT_LINE } },
      axisTick: { show: false },
      axisLabel: { color: AXIS_TEXT, fontSize: 10, interval: 0, hideOverlap: true },
    },
    yAxis: {
      type: 'value',
      axisLine: { show: false },
      axisTick: { show: false },
      splitLine: { lineStyle: { color: SPLIT_LINE } },
      axisLabel: { color: AXIS_TEXT, fontSize: 10, formatter: (v: number) => `${v}s` },
    },
    series: [
      {
        name: '开机用时',
        type: 'bar' as const,
        data: recent.map((r, i) => ({
          value: values[i],
          itemStyle: {
            color: r.basis === 'log' ? BAR_COLOR : BAR_ESTIMATED,
            borderRadius: [3, 3, 0, 0],
          },
        })),
        barMaxWidth: 26,
        label: {
          show: true,
          position: 'top' as const,
          formatter: (p: { value?: number }) => `${p.value ?? 0}s`,
          color: AXIS_TEXT,
          fontSize: 10,
        },
      },
    ],
  }

  return (
    <ChartCard title="最近几次开机用时" sub="BootFlow 自记账 · 每次开机都记">
      <p className="text-2xs leading-5 text-ink-muted">
        最近一次 <span className="text-ink-dim">{relTime(latest.bootStartedAt)}</span>，用时{' '}
        <span className="tnum font-semibold text-ink">{fmt(latest.totalMs)}</span>
        {latest.basis !== 'log' && <span className="text-ink-dim">（推算）</span>}
        <span className="text-ink-dim">
          {' '}
          · 平均 {fmt(avg)} · 最慢 {fmt(slowest.totalMs)}
        </span>
      </p>

      {/* 只有一条时没有趋势可言，画一根柱只是重复上面那句话 */}
      {recent.length >= 2 && (
        <div data-chart="boot-history">
          <ReactECharts
            option={option}
            style={{ height: 132, width: '100%', marginTop: 4 }}
            opts={{ renderer: 'svg' }}
            notMerge
          />
        </div>
      )}

      <p className="text-2xs leading-5 text-ink-dim">
        这是系统启动 → 登录完成的总时长。它只有总时长，拿不到「每一段各花了多久」
        ——那需要系统事件日志，而系统只在开机偏慢时写。
        {estimatedCount > 0 && (
          <>
            {' '}
            <span style={{ color: '#d29922' }}>灰色那 {estimatedCount} 条是按运行时长推算的，不是实测</span>
            （当时读不到系统日志，那种情况下算出来的数字可能把好几次开关机算成一次）。
          </>
        )}
      </p>
    </ChartCard>
  )
}
