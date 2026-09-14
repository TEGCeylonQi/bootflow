import { useAppStore } from '@/store/useAppStore'
import { useFilteredItems } from '@/hooks/useFilteredItems'
import type { ViewMode } from '@/store/useAppStore'
import { SwimlaneView } from './SwimlaneView'
import { GanttView } from './GanttView'
import { PhaseLegend } from './PhaseLegend'

const TABS: { key: ViewMode; label: string; hint: string }[] = [
  { key: 'swimlane', label: '启动时序', hint: '按开机阶段分组，看谁在什么时间点启动' },
  { key: 'gantt', label: '耗时分析', hint: '各阶段耗时占比，以及拖慢开机的具体服务' },
]

export function WorkCanvas() {
  const view = useAppStore((s) => s.view)
  const setView = useAppStore((s) => s.setView)
  const bootTimeline = useAppStore((s) => s.bootTimeline)
  const status = useAppStore((s) => s.status)
  const filtered = useFilteredItems()

  return (
    <div className="flex h-full min-h-0 flex-col bg-base">
      <div className="flex shrink-0 items-center gap-1 border-b border-line-subtle px-3 py-1.5">
        {TABS.map((t) => (
          <button
            key={t.key}
            type="button"
            title={t.hint}
            onClick={() => setView(t.key)}
            className={[
              'rounded px-2 py-1 text-xs transition-colors',
              view === t.key ? 'bg-hover text-ink' : 'text-ink-muted hover:text-ink',
            ].join(' ')}
          >
            {t.label}
          </button>
        ))}

        <div className="flex-1" />

        <span className="text-2xs text-ink-dim">
          {status === 'scanning' ? '扫描中…' : `当前显示 ${filtered.length} 项`}
        </span>
      </div>

      <div className="min-h-0 flex-1 overflow-auto scroll-thin">
        {view === 'swimlane' ? <SwimlaneView items={filtered} /> : <GanttView items={filtered} />}
      </div>

      <div className="shrink-0 border-t border-line-subtle px-3 py-2">
        <PhaseLegend timeline={bootTimeline} />
      </div>
    </div>
  )
}
