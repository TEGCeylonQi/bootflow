import { useMemo } from 'react'
import ReactECharts from 'echarts-for-react'
import { useAppStore } from '@/store/useAppStore'
import { displayNameOf } from '@/lib/item'
import { PHASE_COLOR, PHASE_LABEL } from '@/constants'
import { readableOn } from '@/lib/color'
import type { BootPhase, ItemObservation, PhaseSpan, StartupItem } from '@/types/model'
import { ChartCard, LegendDot } from './ChartCard'

/**
 * 「每项在开机后第几秒出现」—— 把开机时间轴拆到单项。
 *
 * ## 这里为什么能拆、拆出来的是什么
 *
 * Windows **没有单项墙钟耗时计时器**：拉自启项的 Explorer、拉服务的 SCM、
 * 拉任务的计划任务程序，谁都不为自己的"孩子"打点。Event 100 给的是 12 个
 * **相位**耗时（`BootExplorerInitTime` 把所有自启项合成一个数字）；
 * 只有**异常到被系统判定为慢**的项才会单独留下耗时记录（Event 101/102/103）。
 *
 * ⚠️ 但"单项数据不存在"这个结论**是错的**——逐项的 CPU 时间与磁盘 IO
 * 在 WDI 的 `StartupInfo` 里，由 `ImpactPanel` 那块展示。准确的切分是三个量：
 *
 * | 量 | 覆盖 | 来源 | 本图里长什么样 |
 * |---|---|---|---|
 * | 出现时刻 | 每一项 | 内核（`GetProcessTimes`） | 青色圆点 |
 * | 资源占用 | 每一项（需提权） | WDI `StartupInfo` | 不在本图，见下一块 |
 * | 墙钟耗时 | **只有**被判慢的项 | Event 101/102/103 | 蓝色实条，长度即耗时 |
 *
 * ## 为什么画成图而不是列表
 *
 * 这条轴与上面的「开机各阶段」**共用同一套秒数**，背景还铺了相位色带——
 * 于是"它在开机后 12.4 秒出现"可以直接读成"它是在登录那一阵被拉起来的"，
 * 不用在两个列表之间来回对数字。
 *
 * ## 绝不做的两件事
 *
 * 1. **不给没有耗时记录的项补一个耗时。** 把相位区间长度摊给区间里的每一项，
 *    得到的数字看着精确、实际是编的，而且同相位 N 项拿到同一个值，排序都排不出来。
 * 2. **不把累计读盘当"开机阶段消耗"。** 采样拍在开机后 42 秒，那么 `read_bytes`
 *    就是"这 42 秒里累计读了多少"，不是"开机时读了多少"，所以它只出现在详情里。
 */

/** 时间轴最多画几行。再多就该靠左侧筛选去看，而不是把图拉成一堵墙。 */
const MAX_ROWS = 10
/** 每行高度（像素）。行距固定，行数变化时只改容器高度。 */
const ROW_H = 22
/** 顶部那条相位标尺的高度。它和下面的行共用同一条秒数轴，所以位置可以直接对齐。 */
const RULER_H = 16
/** 名称列宽度 */
const NAME_W = 104

const AXIS_TEXT = '#8b949e'
const SPLIT_LINE = '#21262d'
/** 实测耗时（事件日志）。与画布 accent 一致。 */
const C_MEASURED = '#58a6ff'
/** 实测出现时刻（内核记录的进程创建时刻）。 */
const C_OBSERVED = '#39c5bb'

const fmt = (ms: number) => `${(ms / 1000).toFixed(1)}s`

/** 时间轴上限：向上取整到一个好读的刻度，并留一点右边距。 */
function axisMaxOf(values: number[]): number {
  const raw = Math.max(...values, 1)
  const step = raw > 120_000 ? 30_000 : raw > 60_000 ? 10_000 : raw > 20_000 ? 5_000 : 2_000
  return Math.ceil((raw * 1.08) / step) * step
}

/** 时间轴上的一行：一个启动项的出现时刻，以及它可能有的实测耗时。 */
interface Row {
  id: string
  name: string
  /** 实测出现时刻（毫秒，相对本次开机起点） */
  start: number
  /** 有实测耗时才有长度 */
  duration?: number
  phase: BootPhase
}

export function ItemCostTimeline({
  items,
  observation,
}: {
  items: StartupItem[]
  observation: ItemObservation | null
}) {
  const bootTimeline = useAppStore((s) => s.bootTimeline)
  const select = useAppStore((s) => s.select)

  /** 出现时刻已知的项，按时先后排序。相位估算的项**不进这条轴**（见文件头第 2 条）。 */
  const rows = useMemo<Row[]>(
    () =>
      items
        .filter((it) => it.timing.observedStartMs !== undefined)
        .map((it) => ({
          id: it.id,
          name: displayNameOf(it),
          start: it.timing.observedStartMs as number,
          duration: it.timing.durationMs,
          phase: it.bootPhase,
        }))
        .sort((a, b) => a.start - b.start),
    [items],
  )

  const phases = bootTimeline?.phases ?? []

  const view = useMemo(() => {
    // ECharts 类目轴把索引 0 画在最下方；反转后自上而下即为时间先后，
    // 与"先出现的在上"这个阅读习惯一致。
    const shown = rows.slice(0, MAX_ROWS).reverse()
    // 轴上限只由**项自己的位置**决定。采样时刻（"拍于开机后 42.6 秒"）不参与，
    // 否则右边会白白留出一大段空档，把几十个点挤在左边三分之一里。
    const ends = rows.map((r) => r.start + (r.duration ?? 0))
    const axisMax = axisMaxOf(ends)
    const measured = rows.filter((r) => r.duration !== undefined).length
    const estimated = items.filter((it) => it.timing.confidence === 'estimated').length
    return { shown, axisMax, measured, estimated, total: rows.length }
  }, [rows, items])

  /* ── 没读到采样：不是"没有这一项"，而是"这次没读到" ── */

  if (!observation || observation.capturedAtOffsetMs === undefined) {
    return (
      <ChartCard title="每项在开机后第几秒出现" sub="实测 · 内核记录的进程创建时刻">
        <p className="text-2xs leading-5 text-ink-muted">
          没能确定本次开机的起点时刻，所以没法把进程创建时刻换算成「开机后第几秒」。
          <span className="text-ink-dim">
            {' '}
            {observation?.unavailableReason ?? '系统日志里没有本机上次开机的记录。'}
          </span>
        </p>
        <p className="mt-1 text-2xs leading-5 text-ink-dim">
          开始菜单 → 重启（不要「关机再开」）后再扫一次，通常就有了。
        </p>
      </ChartCard>
    )
  }

  /* ── 采到了，但一项都没对上 ── */

  if (view.shown.length === 0) {
    return (
      <ChartCard title="每项在开机后第几秒出现" sub="实测 · 内核记录的进程创建时刻">
        <p className="text-2xs leading-5 text-ink-muted">
          采样到 <span className="tnum text-ink">{observation.processCount}</span> 个进程，但这些项一个都没对上。
          常见原因：多个启动项共用一个可执行文件（几十个服务共用 <code className="text-ink-muted">svchost.exe</code>{' '}
          时，这个进程的时刻代表不了其中任何一项），或它们本次根本没被拉起。
        </p>
      </ChartCard>
    )
  }

  const { shown, axisMax, measured, estimated, total } = view

  const grid1Top = RULER_H + 18
  const option = {
    backgroundColor: 'transparent',
    animationDuration: 500,
    grid: [
      /* 上：相位标尺。它和下面的行共用同一条秒数轴，所以"第几秒"可以直接对下来。 */
      { left: 0, right: 14, top: 0, height: RULER_H, containLabel: true },
      { left: 0, right: 14, top: grid1Top, height: shown.length * ROW_H, containLabel: true },
    ],
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'shadow' },
      backgroundColor: '#161b22',
      borderColor: '#30363d',
      textStyle: { color: '#e6edf3', fontSize: 12 },
      formatter: (params: unknown) => {
        const list = (Array.isArray(params) ? params : [params]) as Array<{ dataIndex?: number }>
        const r = shown[list[0]?.dataIndex ?? 0]
        if (!r) return ''
        return [
          r.name,
          `<span style="color:${C_OBSERVED}">开机后 ${fmt(r.start)} 出现</span>` +
            `<span style="color:${AXIS_TEXT}">（内核记录，实测）</span>`,
          r.duration !== undefined
            ? `<span style="color:${C_MEASURED}">系统记录了它的耗时：${fmt(r.duration)}</span>`
            : `<span style="color:${AXIS_TEXT}">系统没有记录它的耗时</span>`,
          `<span style="color:${AXIS_TEXT}">所处阶段：${PHASE_LABEL[r.phase] ?? '未归类'}</span>`,
          `<span style="color:${AXIS_TEXT}">注意：出现时刻不是耗时。</span>`,
        ].join('<br/>')
      },
    },
    xAxis: [
      {
        type: 'value',
        gridIndex: 0,
        min: 0,
        max: axisMax,
        axisLine: { show: false },
        axisTick: { show: false },
        axisLabel: { show: false },
        splitLine: { show: false },
      },
      {
        type: 'value',
        gridIndex: 1,
        min: 0,
        max: axisMax,
        axisLine: { lineStyle: { color: SPLIT_LINE } },
        axisTick: { show: false },
        axisLabel: {
          color: AXIS_TEXT,
          fontSize: 10,
          formatter: (v: number) => `${(v / 1000).toFixed(0)}s`,
        },
        splitLine: { lineStyle: { color: SPLIT_LINE } },
      },
    ],
    yAxis: [
      {
        type: 'category',
        gridIndex: 0,
        data: ['开机阶段'],
        axisLine: { show: false },
        axisTick: { show: false },
        axisLabel: { color: AXIS_TEXT, fontSize: 10, width: NAME_W, overflow: 'truncate' },
      },
      {
        type: 'category',
        gridIndex: 1,
        data: shown.map((r) => r.name),
        axisLine: { show: false },
        axisTick: { show: false },
        axisLabel: { color: AXIS_TEXT, fontSize: 10, width: NAME_W, overflow: 'truncate' },
      },
    ],
    series: [
      /* 相位标尺：整条开机时间被切成若干段，色块的宽度就是那一段的长度 */
      ...phases.map((p: PhaseSpan, i: number) => {
        const ms = p.endMs - p.startMs
        return {
          name: `phase-${i}`,
          type: 'bar' as const,
          xAxisIndex: 0,
          yAxisIndex: 0,
          stack: 'ruler',
          barWidth: RULER_H - 2,
          silent: true,
          data: [ms],
          itemStyle: { color: PHASE_COLOR[p.name] },
          label: {
            show: ms > axisMax * 0.07,
            position: 'inside' as const,
            formatter: PHASE_LABEL[p.name],
            color: readableOn(PHASE_COLOR[p.name]),
            fontSize: 9,
          },
        }
      }),
      {
        /* 透明占位条：把"出现时刻"垫成条的起点，让有耗时的项从它出现的那一刻开始画 */
        name: '位置',
        type: 'bar' as const,
        xAxisIndex: 1,
        yAxisIndex: 1,
        stack: 'pos',
        barWidth: 4,
        silent: true,
        itemStyle: { color: 'transparent' },
        data: shown.map((r) => r.start),
      },
      {
        name: '实测耗时',
        type: 'bar' as const,
        xAxisIndex: 1,
        yAxisIndex: 1,
        stack: 'pos',
        barWidth: 9,
        itemStyle: { color: C_MEASURED, borderRadius: 2 },
        data: shown.map((r) => r.duration ?? 0),
      },
      {
        name: '出现时刻',
        type: 'scatter' as const,
        xAxisIndex: 1,
        yAxisIndex: 1,
        symbolSize: 8,
        itemStyle: { color: C_OBSERVED },
        z: 3,
        data: shown.map((r) => [r.start, r.name]),
      },
    ],
  }

  return (
    <ChartCard
      title="每项在开机后第几秒出现"
      sub={`实测 · 内核记录的进程创建时刻 · 采样拍于开机后 ${fmt(observation.capturedAtOffsetMs)}`}
      legend={
        <>
          <LegendDot color={C_OBSERVED} text="出现时刻" />
          {measured > 0 && <LegendDot color={C_MEASURED} text="实测耗时" />}
        </>
      }
    >
      <div data-chart="item-timeline">
        <ReactECharts
          option={option}
          style={{ height: grid1Top + shown.length * ROW_H + 30, width: '100%', marginTop: 2 }}
          opts={{ renderer: 'svg' }}
          notMerge
          onEvents={{
            click: (params: { dataIndex?: number }) => {
              const r = shown[params?.dataIndex ?? -1]
              if (r) select(r.id)
            },
          }}
        />
      </div>

      <p className="text-2xs leading-5 text-ink-muted">
        Windows 不为单个启动项记录<span className="text-ink">「花了多久」</span>，这条轴上画的是它
        <span className="text-ink">什么时候出现</span>——<span className="text-ink">「出现时刻」不是「花了多久」</span>；
        它有多重（CPU 与磁盘）见下面
        <span className="text-ink">「每一项占了多少资源」</span>。
        {measured > 0 && <> 其中 {measured} 项被系统判定过「启动偏慢」，才有真实的耗时——画成蓝色条，长度即耗时。</>}
      </p>

      {(total > MAX_ROWS || estimated > 0) && (
        <p className="mt-1 text-2xs leading-5 text-ink-dim">
          {total > MAX_ROWS && <>另有 {total - MAX_ROWS} 项也测到了出现时刻，这里只画最早的 {MAX_ROWS} 项。 </>}
          {estimated > 0 && (
            <>
              还有 <span className="tnum text-ink-muted">{estimated}</span> 项
              <span className="text-ink-muted">没有</span>出现时刻（本次没被拉起，或与别的项共用一个可执行文件），
              按相位估算的项会落在同一个位置，画进来是假的整齐——它们只在「启动时序」里以 ~ 前缀出现。
            </>
          )}
        </p>
      )}
    </ChartCard>
  )
}
