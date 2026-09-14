import { useMemo } from 'react'
import { ShieldCheck, TriangleAlert } from 'lucide-react'
import { useAppStore } from '@/store/useAppStore'
import { RISK_META } from '@/constants'
import { tallyAttention } from '@/lib/item'

/**
 * 「需要关注」提示条。
 *
 * 首版的核心价值是「告诉用户哪里有问题」，所以这句结论必须在第一屏就看得见，
 * 而不是让用户在 40 项清单里自己找。点击可在「只看问题项」和「看全部」之间切换。
 *
 * 统计口径已扩展为三类：风险项、卸载残留、有处置建议的项。
 * 三者性质不同，所以副信息分别列出，而不是笼统加总——
 * "有 12 项需要关注"和"4 项是卸载残留、3 项建议处理"给用户的感受完全不同。
 */
export function AttentionBanner() {
  const items = useAppStore((s) => s.items)
  const onlyProblems = useAppStore((s) => s.onlyProblems)
  const setOnlyProblems = useAppStore((s) => s.setOnlyProblems)

  const tally = useMemo(() => tallyAttention(items), [items])

  if (items.length === 0) return null

  if (tally.total === 0) {
    return (
      <div className="flex shrink-0 items-center gap-1.5 border-b border-line-subtle bg-ok/5 px-3 py-2">
        <ShieldCheck size={13} className="shrink-0 text-ok" />
        <span className="text-mini leading-4 text-ink-muted">
          未发现需要处理的问题，这份清单是干净的
        </span>
      </div>
    )
  }

  // 只有残留和建议、没有实际风险时，降级成中性灰，避免过度报警
  const color =
    tally.severe > 0
      ? RISK_META.High.color
      : tally.minor > 0
        ? RISK_META.Medium.color
        : '#8b949e'

  const parts: string[] = []
  if (tally.dead > 0) parts.push(`${tally.dead} 项是卸载残留`)
  if (tally.advice > 0) parts.push(`${tally.advice} 项建议处理`)
  if (tally.severe > 0) parts.push(`${tally.severe} 项风险较高`)

  return (
    <button
      type="button"
      onClick={() => setOnlyProblems(!onlyProblems)}
      title={onlyProblems ? '点击显示全部启动项' : '点击只看这些有问题的项'}
      className="flex w-full shrink-0 items-center gap-1.5 border-b border-line-subtle px-3 py-2 text-left transition-colors hover:bg-hover/60"
      style={{ background: onlyProblems ? `${color}14` : `${color}0a` }}
    >
      <TriangleAlert size={13} className="shrink-0" style={{ color }} />
      <span className="shrink-0 text-mini leading-4 text-ink">
        有 <span className="tnum font-medium">{tally.total}</span> 项需要关注
      </span>
      {parts.length > 0 && (
        <span className="truncate text-mini leading-4 text-ink-dim">
          · {parts.join(' · ')}
        </span>
      )}
      <span className="flex-1" />
      <span className="shrink-0 text-mini" style={{ color: onlyProblems ? color : undefined }}>
        {onlyProblems ? '显示全部' : '只看这些'}
      </span>
    </button>
  )
}
