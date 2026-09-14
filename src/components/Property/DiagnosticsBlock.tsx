import { ShieldCheck } from 'lucide-react'
import { RISK_META } from '@/constants'
import type { DiagnosticInfo } from '@/types/model'

/**
 * 诊断结论区。
 *
 * 顺序刻意颠倒过：**人话结论在前，技术诊断码在后**。
 * 之前的版本把 `MANUAL_BUT_SHOULD_AUTO` 这种码放在最醒目位置，
 * 那是写给开发者看的，不是写给用户看的。
 */
export function DiagnosticsBlock({ items }: { items: DiagnosticInfo[] }) {
  if (items.length === 0) {
    return (
      <div className="flex items-start gap-1.5 rounded border border-ok/30 bg-ok/5 px-2 py-1.5">
        <ShieldCheck size={12} className="mt-[2px] shrink-0 text-ok" />
        <p className="text-mini leading-4 text-ink-muted">未发现异常，这一项运行正常。</p>
      </div>
    )
  }

  return (
    <div className="space-y-2">
      {items.map((d, i) => {
        const color = RISK_META[d.severity].color
        return (
          <div
            key={`${d.code}-${i}`}
            className="rounded-r border-l-2 py-1.5 pl-2 pr-1.5"
            style={{ borderColor: color, background: `${color}0f` }}
          >
            <p className="selectable text-mini leading-4 text-ink">{d.message}</p>

            {(d.code || d.evidence) && (
              <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-2xs text-ink-dim">
                <span className="font-mono">{d.code}</span>
                {d.evidence && <span className="selectable font-mono">{d.evidence}</span>}
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}
