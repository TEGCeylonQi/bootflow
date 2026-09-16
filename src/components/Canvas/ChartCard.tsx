import type { ReactNode } from 'react'

/**
 * 耗时分析里每张图的统一外壳：**标题 + 图 + 一行口径说明**。
 *
 * 之所以把它抽出来：这一页有四张互不相干的图，如果每张各自排版，
 * 很快就会变成"四块纸片堆在一起"——标题字号不一、说明有长有短、
 * 间距各不相同。这正是它上一版被抱怨"冗余杂乱"的原因之一。
 *
 * 排版契约（改这一页时请守住）：
 * 1. 一级界面只放**一行**口径说明，讲清这张图画的是哪个量、不是什么。
 * 2. 三句话以上的解释一律进 `Methodology`（折叠区），不要堆在图上。
 */
export function ChartCard({
  title,
  sub,
  legend,
  children,
}: {
  title: string
  /** 标题右侧的灰色限定语（口径、时刻、来源） */
  sub?: string
  /** 右上角的图例（可省略） */
  legend?: ReactNode
  children: ReactNode
}) {
  return (
    <div className="rounded-card border border-line bg-base px-3 py-2.5">
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
        <span className="text-xs font-medium text-ink">{title}</span>
        {sub && <span className="text-2xs text-ink-dim">{sub}</span>}
        <div className="flex-1" />
        {legend}
      </div>
      <div className="mt-1.5">{children}</div>
    </div>
  )
}

export function LegendDot({ color, text }: { color: string; text: string }) {
  return (
    <span className="flex items-center gap-1 text-2xs text-ink-dim">
      <span className="h-2 w-2 rounded-[2px]" style={{ background: color }} />
      {text}
    </span>
  )
}
