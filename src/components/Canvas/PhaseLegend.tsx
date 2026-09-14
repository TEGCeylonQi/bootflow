import { PHASE_COLOR, PHASE_LABEL } from '@/constants'
import type { BootTimeline } from '@/types/model'

/**
 * 底栏图例：开机各阶段配色与耗时，合计值即本次开机总耗时。
 *
 * 三种"没有数据"的情形必须分开说，不能都显示成一句「暂无数据」：
 *   1. 没读过（timeline 为 null）—— 应用刚启动，扫描还没回来
 *   2. 没读成（unavailableReason）—— 权限不够，这是**最容易被误解成"开机不花时间"**的一种
 *   3. 读成了但空（系统没写记录）—— 正常速度的开机，系统常常什么都不记
 */
export function PhaseLegend({ timeline }: { timeline: BootTimeline | null }) {
  if (!timeline) {
    return <p className="text-2xs text-ink-dim">开机耗时数据读取中…</p>
  }

  if (timeline.phases.length === 0) {
    if (timeline.unavailableReason) {
      return (
        <p className="truncate text-2xs" style={{ color: '#d29922' }} title={timeline.unavailableReason}>
          开机耗时未读取 —— {timeline.unavailableReason}
        </p>
      )
    }
    return <p className="text-2xs text-ink-dim">系统还没有记录开机性能数据</p>
  }

  const total = timeline.totalBootMs ?? timeline.phases[timeline.phases.length - 1].endMs

  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
      {timeline.phases.map((p) => (
        <span key={p.name} className="flex items-center gap-1 text-2xs">
          <span className="h-2 w-2 rounded-[2px]" style={{ background: PHASE_COLOR[p.name] }} />
          <span className="text-ink-muted">{PHASE_LABEL[p.name]}</span>
          <span className="tnum text-ink-dim">{((p.endMs - p.startMs) / 1000).toFixed(1)}s</span>
        </span>
      ))}

      <span className="ml-auto tnum text-2xs text-ink-muted">
        合计 {(total / 1000).toFixed(1)}s
      </span>
    </div>
  )
}
